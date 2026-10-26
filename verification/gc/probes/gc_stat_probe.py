"""Inject one stale stat ENOENT on a private sealed, unowned native pack.

Run with LD_PRELOAD pointing to gc_stat_fault.c's compiled shared library.
The unlink response is real; this does not emulate a stale unlink result.
"""

import argparse
import hashlib
import json
import os
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=10)
    parser.add_argument("--expected", choices=("legacy", "fixed"), required=True)
    args = parser.parse_args()
    if not args.root.is_absolute() or args.root.exists() or not 1 <= args.samples <= 200:
        parser.error("Use a fresh absolute private root and 1..200 samples")
    if "LD_PRELOAD" not in os.environ:
        parser.error("The stat fault shim must be preloaded")
    args.root.mkdir(parents=True)
    cases = []
    for sample in range(args.samples):
        root = args.root / str(sample)
        with SharedFilesystemStore(root, "stale-stat", online_gc=True) as store:
            reference = store.publish([Record("payload", b"x" * 2048)], submission_id="payload")
            store.seal()
            path = root / reference.manifest.segment.path
            size = path.stat().st_size
            store.release_publications([reference])
            arm = root / "stat-arm"
            marker = arm.with_name(arm.name + ".fired")
            os.environ["STRAW_STALE_STAT_PATH"] = str(path)
            os.environ["STRAW_STALE_STAT_ARM"] = str(arm)
            effects = []

            def inject(phase):
                effects.append(phase)
                if phase == "before_gc_stat":
                    arm.touch(exist_ok=False)

            store.backend.fault = inject
            start = time.monotonic()
            result = store.collect_garbage()
            elapsed = time.monotonic() - start
            store.backend.fault = lambda _: None
            assert marker.exists() and not arm.exists(), "stat injection did not fire"
            remaining = path.exists()
            assert result["tombstones"] == 1 and result["owners"] == 0
            assert result["reclaimed_files"] == (1 if args.expected == "fixed" else 0)
            assert result["reclaimed_bytes"] == 0  # no successful pre-unlink size observation
            assert remaining == (args.expected == "legacy")
            repeated = store.collect_garbage()
            assert repeated["reclaimed_files"] == 0 and path.exists() == remaining
        with SharedFilesystemStore(root, "stale-stat", online_gc=True) as recovered:
            replayed = recovered.collect_garbage()
            assert replayed["reclaimed_files"] == 0 and path.exists() == remaining
        # A legacy receipt has already retired the deletion obligation. Clean
        # this known orphan explicitly and report it separately from Straw GC.
        if remaining:
            path.unlink()
        assert not path.exists()
        cases.append(
            {
                "result": result,
                "seconds": elapsed,
                "actual_pack_bytes": size,
                "orphan_after_receipt_and_replay": remaining,
                "external_orphan_cleanup": remaining,
                "effects": effects,
            }
        )
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
