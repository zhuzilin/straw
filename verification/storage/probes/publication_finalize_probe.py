"""Measure standalone publication sealing, release/GC and uncertain seal retries.

Healthy samples use 2 KiB and 8 MiB payloads. Baseline mode expects write_records
and record_set to remain catalog-open after release/close/reopen. Their private
fixtures are preserved for explicitly separate supervisor cleanup. Fixed mode
also injects before/after seal and catalog-sync errors and retries the same ID.
"""

import argparse
import hashlib
import json
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable


def make_store(root):
    return SharedFilesystemStore(root, "finalize", online_gc=True, segment_target_bytes=0)


def invoke(store, api, payload, raw):
    if api == "record_set":
        return [store.record_set(raw, submission_id="result")]
    if api == "write_records":
        return store.write_records([Record("r", payload)], submission_id="result")
    return [store.publish([Record("r", payload)], submission_id="result")]


def check_read(store, api, refs, payload):
    record = store.read_record(refs[0]) if api == "write_records" else next(store.read(refs[0]))
    assert record.payload == payload


def healthy(root, api, size, expected):
    root.mkdir(parents=True, exist_ok=False)
    payload = b"x" * size
    with make_store(root) as store:
        raw = store.write_records([Record("raw", payload)], submission_id="raw") if api == "record_set" else None
        start = time.monotonic()
        refs = invoke(store, api, payload, raw)
        publish_ms = (time.monotonic() - start) * 1000
        before = store.collect_garbage()
        count = 2 if api == "record_set" else 1
        broken = expected == "baseline" and api != "publish"
        assert before["open_packs"] == (count if broken else 0) and before["reclaimed_files"] == 0
        check_read(store, api, refs, payload)
        store.seal()
        store.release_publications(refs)
        start = time.monotonic()
        collected = store.collect_garbage()
        gc_ms = (time.monotonic() - start) * 1000
        assert collected["owners"] == 0 and collected["reclaimed_files"] == (0 if broken else count)
    with make_store(root) as fresh:
        reopened = fresh.collect_garbage()
        assert reopened["open_packs"] == (count if broken else 0)
        assert reopened["reclaimed_files"] == 0
    remaining = [str(p.relative_to(root)) for p in root.rglob("*.sealed")]
    assert len(remaining) == (count if broken else 0)
    return {
        "kind": "healthy",
        "api": api,
        "payload_bytes": size,
        "before_release": before,
        "gc": collected,
        "reopened": reopened,
        "publication_ms": publish_ms,
        "gc_ms": gc_ms,
        "remaining_private_files": remaining,
    }


def uncertain(root, api, phase):
    root.mkdir(parents=True, exist_ok=False)
    payload = b"x" * 2048
    with make_store(root) as store, make_store(root) as peer:
        raw = store.write_records([Record("raw", payload)], submission_id="raw") if api == "record_set" else None
        armed, fired = False, False
        events = []

        def fault(at):
            nonlocal armed, fired
            if at == "before_publication_seal":
                armed = True
                before = peer.collect_garbage()
                assert before["reclaimed_files"] == 0
                events.append({"phase": at, "gc": before})
            if armed and at == phase and not fired:
                fired = True
                events.append({"injected": at})
                raise OSError(5, "uncertain seal")

        store.backend.fault = fault
        try:
            invoke(store, api, payload, raw)
        except StorageUnavailable:
            pass
        else:
            raise AssertionError("Expected seal error")
        assert fired
        paths = set(root.rglob("*.sealed"))
        after = peer.collect_garbage()
        assert after["reclaimed_files"] == 0
        store.backend.fault = lambda at: None
        start = time.monotonic()
        refs = invoke(store, api, payload, raw)
        retry_ms = (time.monotonic() - start) * 1000
        assert set(root.rglob("*.sealed")) == paths
        assert peer.collect_garbage()["open_packs"] == 0
        check_read(store, api, refs, payload)
        store.release_publications(refs)
        collected = peer.collect_garbage()
        assert collected["reclaimed_files"] == (2 if api == "record_set" else 1) and collected["owners"] == 0
        assert not list(root.rglob("*.sealed"))
    return {
        "kind": "fault",
        "api": api,
        "failed_phase": phase,
        "events": events,
        "after_failure": after,
        "gc": collected,
        "retry_ms": retry_ms,
        "remaining_private_files": [],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    args = parser.parse_args()
    if not 1 <= args.samples <= 1000:
        parser.error("--samples must be 1..1000")
    args.root.mkdir(parents=True, exist_ok=False)
    cases = [
        healthy(args.root / f"{api}-{size}-{sample}", api, size, args.expected)
        for sample in range(args.samples)
        for api in ("write_records", "record_set", "publish")
        for size in (2048, 8 * 1024 * 1024)
    ]
    if args.expected == "fixed":
        cases.extend(
            uncertain(args.root / f"{api}-{phase}", api, phase)
            for api in ("write_records", "record_set", "publish")
            for phase in (
                "before_publication_seal",
                "after_publication_seal",
                "before_storage_catalog_sync",
                "after_storage_catalog_sync",
            )
        )
    print(
        json.dumps(
            {
                "passed": True,
                "root": str(args.root),
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                "cases": cases,
                "gc_reclaimed_files": sum(c["gc"]["reclaimed_files"] for c in cases),
                "remaining_private_files": sum(len(c["remaining_private_files"]) for c in cases),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
