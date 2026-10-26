"""Measure cold, warm and failed-empty-refresh retry over catalog history.

Preparation appends a legal alternating retain/release history offline, while no
clients are active, and syncs it once. One native 2 KiB publication remains owned.
This measures replay work, not per-event commit throughput or payload bandwidth.
"Cold" means a new catalog instance, not evicted filesystem/client caches.
Measure under catalog_fsync_fault.c's LD_PRELOAD shim; only the armed empty
refresh sync is failed. Timing excludes injection and setup; caches stay enabled.
"""

import argparse
import hashlib
import json
import os
import struct
import sys
import time
from dataclasses import asdict
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable
from straw.protocol import RecordSetRef

RUN = "catalog-history"


def checksum(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def frame(sequence, raw):
    prefix = b"STRGC001" + struct.pack("<QQ", sequence, len(raw))
    header = prefix + hashlib.sha256(prefix).digest()
    return header + raw + hashlib.sha256(header + raw).digest() + b"STRGEND1"


def store(root):
    return SharedFilesystemStore(root, RUN, online_gc=True)


def prepare(root, frames):
    if root.exists():
        raise ValueError("Preparation requires a fresh private root")
    with store(root) as writer:
        reference = writer.publish([Record("data", b"x" * 2048)], submission_id="source")
        writer.seal()
        writer.retain("source", [reference])
        writer.release_publications([reference])
    log = root / "storage.log"
    raw = log.read_bytes()
    offset = sequence = 0
    while offset < len(raw):
        seq, size = struct.unpack_from("<QQ", raw, offset + 8)
        assert seq == sequence
        offset += 96 + size
        sequence += 1
    assert offset == len(raw)
    bodies = [
        json.dumps(
            {"run_id": RUN, "owners": {"churn": [reference.manifest.segment.path] if retain else None}},
            separators=(",", ":"),
        ).encode()
        for retain in (True, False)
    ]
    # A protected, sealed pack is retained/released repeatedly. Even counts end
    # with no churn owner. This is valid history; preparation is not benchmarked.
    with log.open("ab") as stream:
        for index in range(frames):
            stream.write(frame(sequence + index, bodies[index % 2]))
        stream.flush()
        os.fsync(stream.fileno())
    state = {
        "reference": asdict(reference),
        "base_frames": sequence,
        "history_frames": frames,
        "catalog_bytes": log.stat().st_size,
        "catalog_sha256": checksum(log),
    }
    (root / "probe-state.json").write_text(json.dumps(state))
    return state


def healthy(result):
    assert result["owners"] == result["retained_packs"] == 1, result
    assert result["reclaimed_files"] == result["open_packs"] == result["tombstones"] == 0, result


def measure(root, samples):
    state = json.loads((root / "probe-state.json").read_text())
    log, arm = root / "storage.log", root / "arm"
    assert checksum(log) == state["catalog_sha256"]
    cold, warm, retry = [], [], []
    for _ in range(samples):
        with store(root) as reader:
            started = time.monotonic()
            result = reader.collect_garbage()
            cold.append(time.monotonic() - started)
            healthy(result)
    os.environ["STRAW_FAIL_SYNC_PATH"] = str(log)
    os.environ["STRAW_FAIL_SYNC_ARM"] = str(arm)
    with store(root) as reader:
        healthy(reader.collect_garbage())
        for _ in range(samples):
            started = time.monotonic()
            result = reader.collect_garbage()
            warm.append(time.monotonic() - started)
            healthy(result)
            arm.write_text("fail next catalog sync")
            try:
                reader.collect_garbage()
            except StorageUnavailable:
                pass
            else:
                raise AssertionError("Missing injected sync failure; check LD_PRELOAD")
            assert not arm.exists()
            started = time.monotonic()
            result = reader.collect_garbage()
            retry.append(time.monotonic() - started)
            healthy(result)
        reference = RecordSetRef.from_dict(state["reference"])
        assert next(reader.read(reference)).payload == b"x" * 2048
    assert checksum(log) == state["catalog_sha256"], "Read-only GC calls must not grow or alter the log"
    return {**state, "samples": samples, "cold_seconds": cold, "warm_seconds": warm, "retry_seconds": retry}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("prepare", "measure", "cleanup"))
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--frames", type=int, default=0)
    parser.add_argument("--samples", type=int, default=25)
    args = parser.parse_args()
    if not args.root.is_absolute() or not 0 <= args.frames <= 1_000_000 or args.frames % 2:
        parser.error("Use an absolute root and an even frame count in 0..1000000")
    if not 1 <= args.samples <= 200:
        parser.error("Require 1..200 samples")
    native = Path(_native.__file__)
    assert native.is_relative_to(sys.prefix), native
    if args.mode == "prepare":
        result = prepare(args.root, args.frames)
    elif args.mode == "measure":
        result = measure(args.root, args.samples)
    else:
        with store(args.root) as reader:
            reader.release("source")
            result = reader.collect_garbage()
            assert result["reclaimed_files"] == 1 and result["owners"] == 0, result
    print(json.dumps({"passed": True, "mode": args.mode, "native_sha256": checksum(native), "result": result}))


if __name__ == "__main__":
    main()
