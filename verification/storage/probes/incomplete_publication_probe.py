"""Measure private unpublished tails and exercise real cleanup syscall errors.

Use installed, hash-checked wheels. Baseline leftover partials are intentionally
reported after normal GC; the driver removes only its private root afterwards.
Sparse large offsets live in regression tests, not in this payload measurement.
"""

import argparse
import errno
import hashlib
import json
import os
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import IndeterminateCommit, StorageUnavailable

PACKED = 64 * 1024 * 1024


def reset():
    for key in ("TARGET", "KIND", "ARM", "MODE", "LOG"):
        os.environ.pop("STRAW_INCOMPLETE_" + key, None)


def measure(call):
    start = time.monotonic()
    result = call()
    return result, (time.monotonic() - start) * 1000


def data_files(root):
    return {
        str(p.relative_to(root)): p.stat().st_size
        for p in root.rglob("*")
        if p.suffix in (".pack", ".sealed", ".partial")
    }


def case(root, size, target, phase, expected):
    reset()
    root.mkdir(parents=True, exist_ok=False)
    payload = b"x" * size
    with SharedFilesystemStore(root, "incomplete", online_gc=True, segment_target_bytes=target) as store:
        good, prefix_ms = measure(lambda: store.write_records([Record("good", b"g" * 2048)], submission_id="good"))
        prefix = (root / good[0].segment.path).stat().st_size
        before = data_files(root)
        events = []

        def fail(step):
            if step == phase:
                events.append(data_files(root))
                raise OSError(errno.EIO, "private probe interrupted write")

        store.backend.fault = fail
        errors, failure_ms, after_failures = [], [], []
        for _ in range(2):
            start = time.monotonic()
            try:
                store.write_records([Record("retry", payload)], submission_id="retry")
            except StorageUnavailable as error:
                errors.append(type(error).__name__)
            else:
                raise AssertionError("Write unexpectedly succeeded")
            failure_ms.append((time.monotonic() - start) * 1000)
            after_failures.append(data_files(root))
            assert store.read_record(good[0]).payload == b"g" * 2048
        assert len(events) == 2
        if expected == "fixed":
            assert all(files == before for files in after_failures)
        else:
            assert sum(after_failures[-1].values()) > sum(before.values())
        store.backend.fault = lambda _: None
        retry, retry_ms = measure(lambda: store.write_records([Record("retry", payload)], submission_id="retry"))
        segment = retry[0].segment
        if target:
            assert segment.offset == (prefix if expected == "fixed" else sum(after_failures[-1].values()))
        else:
            assert segment.offset == 0
        assert store.read_record(good[0]).payload == b"g" * 2048
        assert store.read_record(retry[0]).payload == payload
        store.seal()
        store.release_publications([*good, *retry])
        gc, gc_ms = measure(store.collect_garbage)
        leftover = data_files(root)
        assert gc["owners"] == 0
        assert gc["reclaimed_files"] == (1 if target else 2)
        assert gc["open_packs"] == (2 if expected == "baseline" and not target else 0)
        assert len(leftover) == gc["open_packs"]
    return {
        "payload_bytes": size,
        "target": target,
        "phase": phase,
        "prefix_bytes": prefix,
        "prefix_ms": prefix_ms,
        "at_failure": events,
        "after_failure": after_failures,
        "errors": errors,
        "failure_ms": failure_ms,
        "retry_ms": retry_ms,
        "retry_offset": segment.offset,
        "unpublished_bytes_after_failures": sum(after_failures[-1].values()) - sum(before.values()),
        "leftover_after_gc": leftover,
        "leftover_bytes_after_gc": sum(leftover.values()),
        "gc": gc,
        "gc_ms": gc_ms,
    }


def injected(root, logs, kind, mode):
    reset()
    root.mkdir(parents=True, exist_ok=False)
    logs.mkdir(parents=True, exist_ok=False)
    target = PACKED if kind in ("truncate", "file_sync") else 0
    arm, log = logs / "arm", logs / "trace.tsv"
    with SharedFilesystemStore(root, "incomplete", online_gc=True, segment_target_bytes=target) as store:
        good = store.write_records([Record("good", b"g" * 2048)], submission_id="good")
        prefix = (root / good[0].segment.path).stat().st_size
        failed = False

        def fail(step):
            nonlocal failed
            if step == "after_payload_chunk" and not failed:
                failed = True
                path = root / good[0].segment.path if target else next(root.rglob("*.partial"))
                if kind == "parent_sync":
                    path = path.parent
                os.environ.update(
                    STRAW_INCOMPLETE_TARGET=str(path),
                    STRAW_INCOMPLETE_KIND=kind if kind in ("truncate", "unlink") else "fsync",
                    STRAW_INCOMPLETE_ARM=str(arm),
                    STRAW_INCOMPLETE_LOG=str(log),
                    STRAW_INCOMPLETE_MODE=mode,
                )
                arm.touch()
                raise OSError(errno.EIO, "private probe interrupted write")

        payload = b"x" * (8 * 1024 * 1024)
        store.backend.fault = fail
        start = time.monotonic()
        try:
            store.write_records([Record("retry", payload)], submission_id="retry")
        except IndeterminateCommit as error:
            assert "cleanup" in str(error)
            error_text = str(error)
        else:
            raise AssertionError("Cleanup error must retain its guard")
        failure_ms = (time.monotonic() - start) * 1000
        assert failed and not arm.exists()
        (entry,) = [line.split("\t", 3) for line in log.read_text().splitlines()]
        assert entry[1] == mode and int(entry[2]) == (0 if mode == "after" else -2)
        after_failure = data_files(root)
        assert store.read_record(good[0]).payload == b"g" * 2048
        assert store.collect_garbage()["reclaimed_files"] == 0
        store.backend.fault = lambda _: None
        retry, retry_ms = measure(lambda: store.write_records([Record("retry", payload)], submission_id="retry"))
        assert retry[0].segment.offset == (prefix if target else 0)
        assert store.read_record(good[0]).payload == b"g" * 2048
        assert store.read_record(retry[0]).payload == payload
        store.seal()
        store.release_publications([*good, *retry])
        gc = store.collect_garbage()
        assert gc["open_packs"] == gc["owners"] == 0 and not data_files(root)
        reset()
    return {
        "kind": kind,
        "mode": mode,
        "trace": entry,
        "error": error_text,
        "failure_ms": failure_ms,
        "after_failure": after_failure,
        "retry_ms": retry_ms,
        "gc": gc,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--traces", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--inject", action="store_true")
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=False)
    cases = []
    if args.inject:
        assert args.expected == "fixed", "Old writer has no cleanup syscalls to intercept"
        for kind in ("truncate", "unlink", "file_sync", "parent_sync"):
            for mode in ("before", "after"):
                name = kind + "-" + mode
                cases.append(injected(args.root / name, args.traces / name, kind, mode))
    else:
        for size in (2048, 8 * 1024 * 1024):
            for target in (0, PACKED):
                for phase in ("after_segment_header", "after_payload_chunk", "after_record", "before_footer"):
                    cases.append(case(args.root / f"{size}-{target}-{phase}", size, target, phase, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "root": str(args.root),
                "cases": cases,
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
