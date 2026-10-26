"""Compare completed-file handoff retries on exact old/new installed wheels.

Failures are callbacks after a real catalog sync, simulating a lost response.
They are not failed syscalls or machine crashes. Old/new timings include different
work. Each case owns a fresh pool; baseline leftovers are counted before explicit
fixture removal, separately from successful GC.
"""

import argparse
import errno
import hashlib
import json
import shutil
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import IndeterminateCommit, StorageUnavailable

PACKED = 64 * 1024 * 1024


def make(root, target=0):
    return SharedFilesystemStore(root, "handoff", online_gc=True, segment_target_bytes=target)


def case(root, api, target, size, phase, expected):
    root.mkdir(parents=True, exist_ok=False)
    with make(root) as source, make(root, target) as writer:
        raw = source.write_records([Record("raw", b"x" * size)], submission_id="raw")
        dep = source.publish([Record("dep", b"dep")], submission_id="dep")

        def invoke(identity):
            if api == "record_set":
                return [writer.record_set(raw, submission_id=identity)]
            records = [Record(identity, b"x" * size)]
            if api == "publish":
                return [writer.publish(records, submission_id=identity, dependencies=[dep])]
            return writer.write_records(records, submission_id=identity)

        occurrence = 1
        if phase == "release":
            occurrence = (5 if target == 0 else 4) if api == "record_set" else (3 if target == 0 else 2)
        armed, fired, seen = False, False, 0

        def fail(at):
            nonlocal armed, fired, seen
            if at == "after_data_sync":
                armed = True
            if armed and at == "after_storage_catalog_sync":
                seen += 1
                if seen == occurrence and not fired:
                    fired = True
                    raise OSError(errno.EIO, "handoff response lost")

        writer.backend.fault = fail
        start = time.monotonic()
        try:
            invoke("first")
        except StorageUnavailable:
            first_failure_ms = (time.monotonic() - start) * 1000
        else:
            raise AssertionError("Expected handoff failure")
        assert fired
        writer.backend.fault = lambda _: None
        before = source.collect_garbage()
        assert before["reclaimed_files"] == 0
        start = time.monotonic()
        try:
            other = invoke("other")
        except IndeterminateCommit:
            assert expected == "fixed"
            other = None
        else:
            assert expected == "baseline"
        other_request_ms = (time.monotonic() - start) * 1000
        phases = []
        writer.backend.fault = phases.append
        start = time.monotonic()
        first = invoke("first")
        retry_ms = (time.monotonic() - start) * 1000
        writes = phases.count("after_payload_chunk")
        assert (writes == 0) == (expected == "fixed")
        record = writer.read_record(first[0]) if api == "write_records" else next(writer.read(first[0]))
        assert record.payload == b"x" * size
        writer.backend.fault = lambda _: None
        if other is None:
            other = invoke("other")
        writer.release_publications([*first, *other])
        source.release_publications([*raw, dep])
        writer.seal()
        after = source.collect_garbage()
        files = [p for p in root.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]
        if expected == "fixed":
            assert not files and after["owners"] == after["open_packs"] == 0
        else:
            assert files and after["owners"] > 0
        result = {
            "api": api,
            "target": target,
            "size": size,
            "phase": phase,
            "first_failure_ms": first_failure_ms,
            "other_request_ms": other_request_ms,
            "retry_ms": retry_ms,
            "retry_payload_write_callbacks": writes,
            "before_retry_gc": before,
            "after_release_gc": after,
            "manual_fixture_data_files": len(files),
            "manual_fixture_data_bytes": sum(p.stat().st_size for p in files),
            "manual_fixture_owners": after["owners"],
            "manual_fixture_open_reservations": after["open_packs"],
        }
    shutil.rmtree(root)
    result["private_pool_removed"] = not root.exists()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    assert args.samples > 0
    args.root.mkdir(parents=True, exist_ok=False)
    native = Path(_native.__file__).resolve()
    assert native.is_relative_to(Path(sys.prefix).resolve())
    cases = [
        case(args.root / f"{api}-{target}-{size}-{phase}-{i}", api, target, size, phase, args.expected)
        for api in ("write_records", "publish", "record_set")
        for target in (0, PACKED)
        for size in (2048, 8388608)
        for phase in (("stage",) if api == "write_records" else ("stage", "release"))
        for i in range(args.samples)
    ]
    print(
        json.dumps({"passed": True, "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(), "cases": cases})
    )


if __name__ == "__main__":
    main()
