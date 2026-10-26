"""Measure staging-release response loss, peer GC and fresh-client retries.

Each case has a private pool and one independent reader owner. Faults are raised
after successful catalog sync, not syscall errors or physical crashes. Timings
compare a failing old retry with a successful new retry, not equivalent work.
"""

import argparse
import errno
import hashlib
import json
import shutil
import struct
import sys
import time
from dataclasses import asdict
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable
from straw.protocol import digest

PACKED = 64 * 1024 * 1024


def store(root, target=0):
    return SharedFilesystemStore(root, "consumption-probe", online_gc=True, segment_target_bytes=target)


def graph(path):
    raw = path.read_bytes()
    offset = 0
    result = {}
    while offset < len(raw):
        length = struct.unpack_from("<Q", raw, offset + 16)[0]
        event = json.loads(raw[offset + 56 : offset + 56 + length])
        result.update(event.get("staging_edges", {}))
        offset += 96 + length
    assert offset == len(raw)
    return result


def case(root, api, target, size, depth, expected):
    root.mkdir(parents=True, exist_ok=False)
    with store(root, target) as writer, store(root) as keeper:
        keep = keeper.publish([Record("keep", b"reader")], submission_id="keep")
        keeper.retain("reader", [keep])
        keeper.release_publications([keep])
        refs = []
        start = time.monotonic()
        for i in range(depth):
            records = [Record(str(i), b"x" * size)]
            if api == "write_records":
                refs.extend(writer.write_records(records, submission_id=str(i)))
            elif api == "publish":
                refs.append(writer.publish(records, submission_id=str(i), dependencies=refs[-1:]))
            else:
                raw = writer.write_records(records, submission_id="raw:" + str(i))
                refs.append(writer.record_set(raw, submission_id=str(i), dependencies=refs[-1:]))
        writer.seal()
        setup_ms = (time.monotonic() - start) * 1000
        before = (root / "storage.log").stat().st_size
        fired = False
        phases = []

        def fail(at):
            nonlocal fired
            phases.append(at)
            if at == "after_storage_catalog_sync" and not fired:
                fired = True
                raise OSError(errno.EIO, "consumption response lost")

        writer.backend.fault = fail
        start = time.monotonic()
        try:
            writer.release_publications([refs[-1]])
        except StorageUnavailable:
            first_failure_ms = (time.monotonic() - start) * 1000
        else:
            raise AssertionError("Expected lost release response")
        assert fired
        writer.backend.fault = lambda _: None
        added = (root / "storage.log").stat().st_size - before
        with store(root) as peer:
            before_retry_gc = peer.collect_garbage()
            assert before_retry_gc["reclaimed_files"] > 0
            assert before_retry_gc["owners"] == 1
        persisted = graph(root / "storage.log")
        reachable = set()
        if expected == "fixed" and api != "write_records":
            pending = ["staged:" + digest(asdict(refs[-1]))]
            while pending:
                key = pending.pop()
                if key not in reachable:
                    reachable.add(key)
                    pending.extend(persisted[key])
            assert len(reachable) == depth * 2
            assert sum(len(persisted[key]) for key in reachable) == 2 * depth - 1
            assert phases.count("before_staging_manifest_read") == depth
        elif expected == "fixed":
            assert "staged:" + digest(asdict(refs[-1])) not in persisted
            assert phases.count("before_staging_manifest_read") == 0
        retry_error = None
        with store(root) as recovered:
            replay_phases = []
            recovered.backend.fault = replay_phases.append
            start = time.monotonic()
            try:
                recovered.release_publications([refs[-1], refs[0], refs[-1]])
            except StorageUnavailable as error:
                assert expected == "baseline" and api != "write_records"
                assert "No such file" in str(error)
                retry_error = str(error)
            else:
                assert expected == "fixed" or api == "write_records"
            retry_ms = (time.monotonic() - start) * 1000
            if expected == "fixed":
                assert "before_staging_manifest_read" not in replay_phases
                assert "before_storage_catalog_write" not in replay_phases
            assert next(recovered.read(keep)).payload == b"reader"
            recovered.release("reader")
            final_gc = recovered.collect_garbage()
            assert final_gc["owners"] == final_gc["open_packs"] == 0
        result = {
            "api": api,
            "target": target,
            "size": size,
            "depth": depth,
            "input_payload_bytes": size * depth,
            "setup_ms": setup_ms,
            "first_failure_ms": first_failure_ms,
            "retry_ms": retry_ms,
            "retry_error": retry_error,
            "first_manifest_read_hooks": phases.count("before_staging_manifest_read"),
            "retry_manifest_read_hooks": replay_phases.count("before_staging_manifest_read"),
            "hook_scope": "Read hooks exist only in the fixed binary; baseline zero is not zero I/O.",
            "catalog_bytes_added": added,
            "reachable_graph_nodes": len(reachable),
            "reachable_graph_edges": sum(len(persisted[key]) for key in reachable),
            "total_cached_nodes": len(persisted),
            "before_retry_gc": before_retry_gc,
            "final_gc": final_gc,
        }
    assert not [p for p in root.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
    shutil.rmtree(root)
    result["private_pool_removed"] = not root.exists()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--depths", nargs="+", type=int, default=[1, 12])
    args = parser.parse_args()
    assert args.samples > 0 and all(depth > 0 for depth in args.depths)
    args.root.mkdir(parents=True, exist_ok=False)
    native = Path(_native.__file__).resolve()
    assert native.is_relative_to(Path(sys.prefix).resolve())
    cases = [
        case(args.root / f"{api}-{target}-{size}-{depth}-{i}", api, target, size, depth, args.expected)
        for api in ("write_records", "publish", "record_set")
        for target in (0, PACKED)
        for size in (2048, 8388608)
        for depth in ([1] if api == "write_records" else args.depths)
        for i in range(1 if api == "write_records" else args.samples)
    ]
    print(
        json.dumps({"passed": True, "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(), "cases": cases})
    )


if __name__ == "__main__":
    main()
