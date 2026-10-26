"""Compare complete-file reuse after fsync failures on isolated installed wheels.

publication_sync_fault.c injects one before/after-real-fsync EIO. A third case
raises from after_data_sync, when Straw has received success. Trace/arm paths
must be private local /tmp paths; data pools may be shared-filesystem paths.
"""

import argparse
import hashlib
import json
import os
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable


def reset():
    for key in ("ROOT", "MODE", "ARM", "LOG"):
        os.environ.pop("STRAW_PENDING_" + key, None)


def trace(path):
    return [
        {
            "result": int(result),
            "injected": bool(int(injected)),
            "performed": bool(int(performed)),
            "actual_result": int(actual),
            "path": name,
        }
        for result, injected, performed, actual, name in (
            line.split("\t", 4) for line in path.read_text().splitlines()
        )
    ]


def case(root, logs, api, size, mode, expected):
    reset()
    root.mkdir(parents=True, exist_ok=False)
    logs.mkdir(parents=True, exist_ok=False)
    payload = b"x" * size
    with SharedFilesystemStore(root, "pending", online_gc=True, segment_target_bytes=0) as store:
        raw = store.write_records([Record("raw", payload)], submission_id="raw") if api == "record_set" else None

        def publish():
            if api == "record_set":
                return [store.record_set(raw, submission_id="retry")]
            if api == "write_records":
                return store.write_records([Record("r", payload)], submission_id="retry")
            return [store.publish([Record("r", payload)], submission_id="retry")]

        arm, log = logs / "arm", logs / "fsync.log"
        arm.touch()
        os.environ.update(
            STRAW_PENDING_ROOT=str(root),
            STRAW_PENDING_MODE=mode,
            STRAW_PENDING_ARM=str(arm),
            STRAW_PENDING_LOG=str(log),
        )
        callback_failed = False

        def fail(phase):
            nonlocal callback_failed
            if mode == "callback" and phase == "after_data_sync":
                callback_failed = True
                raise OSError(5, "reported success then interrupted")

        store.backend.fault = fail
        try:
            publish()
        except StorageUnavailable:
            pass
        else:
            raise AssertionError("Expected uncertain sync")
        initial = trace(log)
        assert len(initial) == 1
        event = initial[0]
        if mode == "callback":
            assert callback_failed and event["result"] == 0 and not event["injected"]
        else:
            assert not callback_failed and not arm.exists() and event["injected"] and event["result"] == -1
            assert event["performed"] == (mode == "after")
            assert event["actual_result"] == (0 if mode == "after" else -2)
        (original,) = root.rglob("*.partial")
        original_bytes = original.stat().st_size
        assert store.collect_garbage()["reclaimed_files"] == 0
        phases = []
        store.backend.fault = phases.append
        start = time.monotonic()
        refs = publish()
        seconds = time.monotonic() - start
        all_events = trace(log)
        retried = all_events[len(initial) :]
        segment = refs[0].segment if api == "write_records" else refs[0].manifest.segment
        same = Path(segment.path).stem == original.stem
        assert same == (expected == "fixed")
        assert ("after_payload_chunk" in phases) == (expected == "baseline")
        assert len(retried) == (0 if expected == "fixed" and mode == "callback" else 1)
        assert all(e["result"] == 0 and e["performed"] and not e["injected"] for e in retried)
        record = store.read_record(refs[0]) if api == "write_records" else next(store.read(refs[0]))
        assert record.payload == payload
        store.release_publications(refs)
        collected = store.collect_garbage()
        assert collected["reclaimed_files"] == (2 if api == "record_set" else 1)
        assert collected["owners"] == 0
        remaining = [str(p.relative_to(root)) for p in root.rglob("*.partial")]
        assert len(remaining) == (1 if expected == "baseline" else 0)
        assert collected["open_packs"] == len(remaining)
        reset()
    return {
        "api": api,
        "payload_bytes": size,
        "failure_mode": mode,
        "same_file": same,
        "initial_fsync": initial,
        "retry_fsync": retried,
        "retry_ms": seconds * 1000,
        "retry_payload_chunks": phases.count("after_payload_chunk"),
        "completed_extent_bytes": original_bytes,
        "rewritten_extent_bytes": segment.size if not same else 0,
        "remaining_private_partials": remaining,
        "remaining_partial_bytes": sum((root / p).stat().st_size for p in remaining),
        "gc": collected,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--traces", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    args = parser.parse_args()
    if not 1 <= args.samples <= 1000:
        parser.error("--samples must be 1..1000")
    args.root.mkdir(parents=True, exist_ok=False)
    args.traces.mkdir(parents=True, exist_ok=False)
    cases = []
    for sample in range(args.samples):
        for api in ("write_records", "record_set", "publish"):
            for size in (2048, 8 * 1024 * 1024):
                for mode in ("before", "after", "callback"):
                    name = f"{api}-{size}-{mode}-{sample}"
                    cases.append(case(args.root / name, args.traces / name, api, size, mode, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "root": str(args.root),
                "traces": str(args.traces),
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                "cases": cases,
                "gc_reclaimed_files": sum(c["gc"]["reclaimed_files"] for c in cases),
                "rewritten_extent_bytes": sum(c["rewritten_extent_bytes"] for c in cases),
                "remaining_partial_files": sum(len(c["remaining_private_partials"]) for c in cases),
                "remaining_partial_bytes": sum(c["remaining_partial_bytes"] for c in cases),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
