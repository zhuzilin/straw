"""Test automatic GC discovery and ownership handoff on private native pools.

Requires catalog_discovery_fault.c via LD_PRELOAD. Stat faults are defensive
injections, not evidence of a natural deployment fault. Initial open NotFound
and concurrent migration from a legacy pool are outside this probe's contract.
"""

import argparse
import hashlib
import json
import os
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable


def case(root, warm, kind, expected):
    with (
        SharedFilesystemStore(root, "discovery", online_gc=True) as source,
        SharedFilesystemStore(root, "discovery") as target,
    ):
        ref = source.publish([Record("payload", b"x" * 2048)], submission_id="source")
        source.seal()
        source.retain("source", [ref])
        source.release_publications([ref])
        if warm:
            assert target.collect_garbage()["owners"] == 1
        arm = root / "arm"
        marker = root / "arm.fired"
        os.environ["STRAW_DISCOVERY_PATH"] = str(root / "storage.log")
        os.environ["STRAW_DISCOVERY_ARM"] = str(arm)
        os.environ["STRAW_DISCOVERY_FAULT"] = kind
        arm.touch()
        error = None
        started = time.monotonic()
        try:
            target.retain("destination", [ref])
        except StorageUnavailable as e:
            error = type(e).__name__
        seconds = time.monotonic() - started
        injected = marker.exists()
        assert injected == (expected == "legacy" or kind == "open-error")
        if arm.exists():
            arm.unlink()  # New stat-free discovery must leave stat faults unused.
        if kind == "open-error":
            assert expected == "fixed" and error == "StorageUnavailable"
            protected = source.collect_garbage()
            assert protected["owners"] == 1 and protected["reclaimed_files"] == 0
            assert next(source.read(ref)).payload == b"x" * 2048
            target.retain("destination", [ref])
        else:
            assert error is None
        source.release("source")
        collected = source.collect_garbage()
        if expected == "legacy":
            assert collected["reclaimed_files"] == 1 and collected["owners"] == 0
            assert not (root / ref.manifest.segment.path).exists()
            final = collected
        else:
            assert collected["reclaimed_files"] == 0 and collected["owners"] == 1
            assert next(target.read(ref)).payload == b"x" * 2048
            target.release("destination")
            final = source.collect_garbage()
            assert final["reclaimed_files"] == 1 and final["owners"] == 0
        return {
            "warm": warm,
            "fault": kind,
            "injected": injected,
            "first_retain_error": error,
            "first_retain_seconds": seconds,
            "gc_after_source_release": collected,
            "final_gc": final,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10)
    parser.add_argument("--expected", choices=("legacy", "fixed"), required=True)
    args = parser.parse_args()
    if not args.root.is_absolute() or args.root.exists() or not 1 <= args.samples <= 100:
        parser.error("Use a fresh absolute private root and 1..100 samples")
    if "LD_PRELOAD" not in os.environ:
        parser.error("Preload catalog_discovery_fault.c's compiled shim")
    args.root.mkdir(parents=True)
    scenarios = [(warm, kind) for warm in (False, True) for kind in ("stat-missing", "stat-error")]
    if args.expected == "fixed":
        scenarios.append((False, "open-error"))
    cases = []
    for sample in range(args.samples):
        for warm, kind in scenarios:
            private = args.root / f"{sample}-{warm}-{kind}"
            cases.append(case(private, warm, kind, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "expected": args.expected,
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                "cases": cases,
            }
        )
    )


if __name__ == "__main__":
    main()
