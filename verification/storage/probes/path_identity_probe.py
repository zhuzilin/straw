"""Measure alias ownership and catalog replay on private pools.

The sample workload uses 2 KiB/8 MiB native records. History fixtures append
offline completed-deletion events while no clients are active; they measure
replay work, not historical publication or deletion throughput. Fresh objects
do not imply cold filesystem caches. The supervisor owns final fixture cleanup.
"""

import argparse
import hashlib
import json
import os
import struct
import time
from dataclasses import asdict, replace
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable
from straw.protocol import RecordRef

RUN = "path-identity"
KINDS = ("canonical", "file", "directory", "writer-directory")
SIZES = (2048, 8 * 1024 * 1024)
TARGETS = (0, 1024 * 1024)


def store(root, target=1024 * 1024):
    return SharedFilesystemStore(root, RUN, online_gc=True, segment_target_bytes=target)


def timed(call):
    start = time.perf_counter()
    result = call()
    return result, (time.perf_counter() - start) * 1000


def save(path, value):
    with path.open("w") as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def prepare(root, kind, size, target):
    root.mkdir(parents=True, exist_ok=False)
    if kind == "writer-directory":
        (root / "physical").mkdir()
        (root / "raw").symlink_to("physical", target_is_directory=True)
    payload = b"x" * size
    with store(root, target) as writer:
        refs, elapsed = timed(lambda: writer.write_records([Record("r", payload)], submission_id="source"))
        original = refs[0]
        path = original.segment.path
        if kind == "writer-directory":
            path = str((root / path).resolve().relative_to(root.resolve()))
        elif kind == "file":
            path = "alias" + Path(path).suffix
            (root / path).symlink_to(original.segment.path)
        elif kind == "directory":
            (root / "alias").symlink_to(Path(path).parent, target_is_directory=True)
            path = "alias/" + Path(path).name
        alias = replace(original, segment=replace(original.segment, path=path))
        writer.seal()
    state = {"original": asdict(original), "alias": asdict(alias), "kind": kind, "size": size, "target": target}
    save(root / "fixture.json", state)
    return {"publication_ms": elapsed, "fixture": state}


def state(root):
    value = json.loads((root / "fixture.json").read_text())
    return value, RecordRef.from_dict(value["original"]), RecordRef.from_dict(value["alias"])


def read_checked(reader, ref, size):
    try:
        record = reader.read_record(ref)
    except StorageUnavailable as error:
        return {"readable": False, "error": str(error)}
    assert record.payload == b"x" * size
    return {"readable": True}


def step(root, action):
    value, original, alias = state(root)
    with store(root) as reader:
        if action == "retain":
            _, ms = timed(lambda: reader.retain("reader", [alias]))
            result = {"retained": True}
        elif action == "drop-staging":
            _, ms = timed(lambda: reader.release_publications([original]))
            result = {"released_staging": True}
        elif action == "release":
            _, ms = timed(lambda: reader.release("reader"))
            result = {"released_reader": True}
        elif action == "read":
            result, ms = timed(lambda: read_checked(reader, alias, value["size"]))
        else:
            result, ms = timed(reader.collect_garbage)
    return {"action": action, "ms": ms, "result": result}


def sample(root, kind, size, target, expected):
    result = prepare(root, kind, size, target)
    _, original, alias = state(root)
    broken = expected == "baseline" and kind != "canonical"
    with store(root) as reader:
        assert read_checked(reader, alias, size)["readable"]
        _, retain_ms = timed(lambda: reader.retain("reader", [alias]))
        reader.release_publications([original])
        before, gc_ms = timed(reader.collect_garbage)
        assert before["owners"] == 1 and before["reclaimed_files"] == int(broken), before
        observed, read_ms = timed(lambda: read_checked(reader, alias, size))
        assert observed["readable"] == (not broken), observed
    with store(root) as fresh:
        reopened, reopen_ms = timed(fresh.collect_garbage)
        assert reopened["reclaimed_files"] == 0
        if not broken:
            assert read_checked(fresh, alias, size)["readable"]
        fresh.release("reader")
        final, final_ms = timed(fresh.collect_garbage)
        assert final["reclaimed_files"] == int(not broken) and final["owners"] == 0
    with store(root) as fresh:
        assert fresh.collect_garbage()["reclaimed_files"] == 0
    assert not (root / original.segment.path).exists()
    return {
        "kind": kind,
        "payload_bytes": size,
        "target": target,
        "publication_ms": result["publication_ms"],
        "retain_ms": retain_ms,
        "protected_gc_ms": gc_ms,
        "protected_read_ms": read_ms,
        "reopen_gc_ms": reopen_ms,
        "final_gc_ms": final_ms,
        "protected_gc": before,
        "read_after_gc": observed,
        "final_gc": final,
        "gc_reclaimed_files": before["reclaimed_files"] + final["reclaimed_files"],
    }


def prepare_migration(root):
    root.mkdir(parents=True, exist_ok=False)
    cases = []
    for kind in ("file", "writer-directory"):
        for size in SIZES:
            for target in TARGETS:
                child = root / f"{kind}-{size}-{target}"
                prepare(child, kind, size, target)
                step(child, "retain")
                # The old staging owner remains until the upgraded reader
                # replays the old alias spelling. Do not expose it to old GC.
                cases.append(str(child.relative_to(root)))
    save(root / "migration.json", cases)
    return {"prepared": len(cases)}


def finish_migration(root):
    cases = []
    for name in json.loads((root / "migration.json").read_text()):
        child = root / name
        dropped = step(child, "drop-staging")
        protected = step(child, "collect")
        assert protected["result"]["reclaimed_files"] == 0
        assert step(child, "read")["result"]["readable"]
        step(child, "release")
        final = step(child, "collect")
        assert final["result"]["reclaimed_files"] == 1
        assert step(child, "collect")["result"]["reclaimed_files"] == 0
        cases.append({"name": name, "drop_staging": dropped, "protected": protected, "final": final})
    return {"cases": cases, "gc_reclaimed_files": len(cases)}


def frame(sequence, value):
    raw = json.dumps(value, separators=(",", ":")).encode()
    prefix = b"STRGC001" + struct.pack("<QQ", sequence, len(raw))
    header = prefix + hashlib.sha256(prefix).digest()
    return header + raw + hashlib.sha256(header + raw).digest() + b"STRGEND1"


def prepare_history(root, count):
    prepare(root, "canonical", 2048, 1024 * 1024)
    log = root / "storage.log"
    data = log.read_bytes()
    offset = sequence = 0
    while offset < len(data):
        seq, size = struct.unpack_from("<QQ", data, offset + 8)
        assert seq == sequence
        offset += 96 + size
        sequence += 1
    assert offset == len(data)
    (root / "retired").mkdir()
    # An offline completed-history fixture, with one live real publication.
    # No historical payload bandwidth/deletion cost is inferred from setup.
    with log.open("ab") as stream:
        for i in range(count):
            path = f"retired/{i}.pack"
            stream.write(
                frame(sequence + i, {"run_id": RUN, "packs": {path: True}, "deleted": [path], "reclaimed": [path]})
            )
        stream.flush()
        os.fsync(stream.fileno())
    value = {
        "count": count,
        "catalog_bytes": log.stat().st_size,
        "catalog_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
    }
    save(root / "history.json", value)
    return value


def history(root, samples):
    fixture = json.loads((root / "history.json").read_text())
    cold, warm = [], []

    def healthy(result):
        assert result["reclaimed_files"] == result["open_packs"] == 0
        assert result["owners"] == result["retained_packs"] == 1
        assert result["tombstones"] == fixture["count"]

    for _ in range(samples):
        with store(root) as reader:
            result, elapsed = timed(reader.collect_garbage)
            healthy(result)
            cold.append(elapsed)
    with store(root) as reader:
        healthy(reader.collect_garbage())
        for _ in range(samples):
            result, elapsed = timed(reader.collect_garbage)
            healthy(result)
            warm.append(elapsed)
    assert hashlib.sha256((root / "storage.log").read_bytes()).hexdigest() == fixture["catalog_sha256"]
    return {**fixture, "cold_ms": cold, "warm_ms": warm}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument(
        "--action",
        choices=("samples", "prepare", "step", "prepare-migration", "finish-migration", "prepare-history", "history"),
        default="samples",
    )
    parser.add_argument("--expected", choices=("baseline", "fixed"), default="fixed")
    parser.add_argument("--kind", choices=KINDS, default="file")
    parser.add_argument("--size", type=int, choices=SIZES, default=2048)
    parser.add_argument("--target", type=int, choices=TARGETS, default=1024 * 1024)
    parser.add_argument("--step", choices=("retain", "drop-staging", "release", "read", "collect"), default="collect")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--count", type=int, choices=(0, 128, 4096), default=4096)
    args = parser.parse_args()
    if not 1 <= args.samples <= 100:
        parser.error("--samples must be 1..100")
    if args.action == "samples":
        args.root.mkdir(parents=True, exist_ok=False)
        cases = [
            sample(args.root / f"{i}-{kind}-{size}-{target}", kind, size, target, args.expected)
            for i in range(args.samples)
            for kind in KINDS
            for size in SIZES
            for target in TARGETS
        ]
        result = {"cases": cases, "gc_reclaimed_files": sum(c["gc_reclaimed_files"] for c in cases)}
    elif args.action == "prepare":
        result = prepare(args.root, args.kind, args.size, args.target)
    elif args.action == "step":
        result = step(args.root, args.step)
    elif args.action == "prepare-migration":
        result = prepare_migration(args.root)
    elif args.action == "finish-migration":
        result = finish_migration(args.root)
    elif args.action == "prepare-history":
        result = prepare_history(args.root, args.count)
    else:
        result = history(args.root, args.samples)
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                **result,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
