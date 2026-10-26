"""Check directory retry barriers and time private 2 KiB publications.

Requires directory_sync_fault.c under LD_PRELOAD. A failed raw-directory fsync
is injected once per retry case. Barrier traces demonstrate the missing/repaired
parent sync; no storage service crash or loss of real data is simulated. Healthy
cold/hot timings disable tracing and injection. Each pool is reclaimed by GC.
"""

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable


def reset():
    for key in ("FAIL", "ARM", "LOG", "ROOT"):
        os.environ.pop("STRAW_DIRECTORY_" + key, None)


def syncs(log):
    return [
        {"result": int(result), "injected": bool(int(injected)), "path": path}
        for result, injected, path in (line.split("\t", 2) for line in log.read_text().splitlines())
    ]


def prepare_fixture(path):
    reset()
    path.mkdir(parents=True)
    resolved = path.resolve()
    for directory in (resolved, *resolved.parents):
        fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def publish(store, submission):
    started = time.monotonic()
    ref = store.publish([Record("payload", b"x" * 2048)], submission_id=submission)
    return ref, time.monotonic() - started


def finish(store, refs):
    for ref in refs:
        assert next(store.read(ref)).payload == b"x" * 2048
    store.seal()
    store.release_publications(refs)
    result = store.collect_garbage()
    assert result["reclaimed_files"] == 1 and result["owners"] == 0
    assert not any(store.backend.root.rglob("*.pack"))
    return result


def retry_case(private, expected):
    prepare_fixture(private)
    root = private / "pool"
    log, arm = private / "syncs.log", private / "arm"
    arm.touch()
    os.environ.update(
        STRAW_DIRECTORY_FAIL=str(root / "raw"),
        STRAW_DIRECTORY_ARM=str(arm),
        STRAW_DIRECTORY_LOG=str(log),
        STRAW_DIRECTORY_ROOT="/",
    )
    with SharedFilesystemStore(root, "directory", online_gc=True) as store:
        try:
            publish(store, "retry")
        except StorageUnavailable:
            pass
        else:
            raise AssertionError("Injected directory error was not returned")
        assert not arm.exists() and (root / "raw").is_dir()
        first = syncs(log)
        assert sum(s["injected"] for s in first) == 1
        log.write_text("")
        ref, seconds = publish(store, "retry")
        retried = syncs(log)
        directory = (root / ref.manifest.segment.path).parent.resolve()
        required = [str(directory), *(str(p) for p in directory.parents)]
        observed = {s["path"] for s in retried if s["result"] == 0}
        missing = sorted(set(required) - observed)
        if expected == "legacy":
            assert str(root.resolve()) in missing
        else:
            assert missing == []
        reset()
        gc = finish(store, [ref])
    return {
        "first_attempt_syncs": first,
        "retry_syncs": retried,
        "required_directories": required,
        "missing_retry_barriers": missing,
        "retry_seconds_with_trace": seconds,
        "gc": gc,
    }


def healthy_case(root):
    reset()
    with SharedFilesystemStore(root, "directory", online_gc=True) as store:
        cold, cold_seconds = publish(store, "cold")
        hot, hot_seconds = publish(store, "hot")
        gc = finish(store, [cold, hot])
    return {"cold_seconds": cold_seconds, "hot_seconds": hot_seconds, "gc": gc}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=25)
    parser.add_argument("--expected", choices=("legacy", "fixed"), required=True)
    args = parser.parse_args()
    if not args.root.is_absolute() or args.root.exists() or not 1 <= args.samples <= 100:
        parser.error("Use a fresh absolute private root and 1..100 samples")
    if "LD_PRELOAD" not in os.environ:
        parser.error("Preload directory_sync_fault.c's compiled shim")
    native = Path(_native.__file__)
    assert native.is_relative_to(sys.prefix)
    prepare_fixture(args.root)
    cases = []
    for index in range(args.samples):
        cases.append(
            {
                "retry": retry_case(args.root / f"{index}-retry", args.expected),
                "healthy": healthy_case(args.root / f"{index}-healthy"),
            }
        )
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
                "expected": args.expected,
                "cases": cases,
                "packs_reclaimed": 2 * len(cases),
                "scope": "Exact private-directory fsync EIO injection. Traced retry checks the complete resolved "
                "ancestor chain. Healthy cold/hot timings disable injection and logging; shim remains loaded. "
                "No actual crash, cache eviction, power-loss proof, large-payload bandwidth or delay bound.",
            }
        )
    )


if __name__ == "__main__":
    main()
