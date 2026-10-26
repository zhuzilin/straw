"""Exercise uncertain-publication holds and conflicting retries through the API.

Use an isolated installed wheel. The callback reports EIO after data fsync;
there is no storage crash. Baseline mode expects actual dependency deletion.
Its unresolved output packs are left for the supervisor's private-fixture
cleanup, explicitly separate from successful GC reclamation.
"""

import argparse
import hashlib
import json
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import IdempotencyConflict, StorageUnavailable


def case(root, api, conflict, expected):
    root.mkdir(parents=True, exist_ok=False)
    events = []
    timings = {}
    reclaimed = 0
    payload = b"x" * 2048
    with (
        SharedFilesystemStore(root, "retry", online_gc=True) as source,
        SharedFilesystemStore(root, "retry", online_gc=True) as target,
    ):

        def dependency(identity):
            if api == "publish":
                ref = source.publish([Record(identity, payload)], submission_id=identity)
            else:
                (ref,) = source.write_records([Record(identity, payload)], submission_id=identity)
            source.seal()
            return ref

        original, alternate = dependency("original"), dependency("alternate")
        original_path = root / (original.manifest.segment.path if api == "publish" else original.segment.path)

        def publish(refs):
            if api == "publish":
                return target.publish([Record("target", payload)], submission_id="retry", dependencies=refs)
            return target.record_set(refs, submission_id="retry")

        def uncertain(phase):
            if phase == "after_data_sync":
                events.append("injected_after_data_sync")
                raise OSError(5, "uncertain publication")

        target.backend.fault = uncertain
        try:
            publish([original])
        except StorageUnavailable:
            events.append("uncertain_return")
        else:
            raise AssertionError("Expected uncertain publication")
        assert events == ["injected_after_data_sync", "uncertain_return"]
        target.backend.fault = lambda phase: None
        source.release_publications([original])
        before = source.collect_garbage()
        assert before["reclaimed_files"] == 0 and original_path.is_file()
        events.append("source_released_dependency_still_protected")
        started = time.monotonic()
        try:
            publish([] if conflict == "remove" else [alternate])
        except IdempotencyConflict:
            events.append("conflict_rejected")
        else:
            raise AssertionError("Expected identity conflict")
        timings["conflicting_retry_ms"] = (time.monotonic() - started) * 1000
        after = source.collect_garbage()
        reclaimed += after["reclaimed_files"]
        exists = original_path.is_file()
        assert exists == (expected == "fixed")
        assert after["reclaimed_files"] == (0 if expected == "fixed" else 1)
        events.append("dependency_protected" if exists else "dependency_deleted")
        completion = None
        if expected == "fixed":
            started = time.monotonic()
            result = publish([original])
            timings["correct_retry_ms"] = (time.monotonic() - started) * 1000
            target.seal()
            source.release_publications([alternate])
            extra = source.collect_garbage()
            assert extra["reclaimed_files"] == 1
            reclaimed += extra["reclaimed_files"]
            assert [record.payload for record in target.read(result)] == [payload]
            record = next(source.read(original)) if api == "publish" else source.read_record(original)
            assert record.payload == payload
            events.append("correct_retry_read_and_extra_root_reclaimed")
            target.release_publications([result])
            completion = source.collect_garbage()
            assert completion["reclaimed_files"] == 2 and completion["owners"] == 0
            reclaimed += completion["reclaimed_files"]
            assert not list(root.rglob("*.pack"))
            events.append("final_publication_released_and_reclaimed")
        else:
            # A rejected replace retry can also leave the alternate root held.
            # Do not release private internal owners or manufacture recovery.
            source.release_publications([alternate])
            cleanup = source.collect_garbage()
            reclaimed += cleanup["reclaimed_files"]
    remaining = [str(path.relative_to(root)) for path in root.rglob("*.pack")]
    return {
        "api": api,
        "conflict": conflict,
        "expected": expected,
        "events": events,
        "before_conflict": before,
        "after_conflict": after,
        "original_pack": str(original_path.relative_to(root)),
        "dependency_exists_after_conflict_gc": exists,
        "completion": completion,
        "timing_ms": timings,
        "gc_reclaimed_packs": reclaimed,
        "remaining_private_packs_for_supervisor": remaining,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10, help="Samples per API/conflict combination")
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    args = parser.parse_args()
    if args.samples <= 0:
        parser.error("--samples must be positive")
    args.root.mkdir(parents=True, exist_ok=False)
    cases = [
        case(args.root / f"{api}-{conflict}-{sample}", api, conflict, args.expected)
        for sample in range(args.samples)
        for api in ("publish", "record_set")
        for conflict in ("remove", "replace")
    ]
    report = {
        "passed": True,
        "root": str(args.root),
        "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
        "payload_bytes": 2048,
        "cases": cases,
        "gc_reclaimed_packs": sum(c["gc_reclaimed_packs"] for c in cases),
        "remaining_private_packs": sum(len(c["remaining_private_packs_for_supervisor"]) for c in cases),
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
