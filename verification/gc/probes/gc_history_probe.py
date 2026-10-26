"""Measure repeated GC calls as already-deleted tombstones accumulate.

Each host uses its own fresh store. Setup and first deletion are outside the
timed samples. No payload bandwidth or cold-cache claim; the supervisor cleans
only this private root after workers finish.
"""

import argparse
import hashlib
import json
import math
import socket
import time
from collections import Counter
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native


def quantiles(values):
    values = sorted(values)
    return {
        "n": len(values),
        **{f"p{p}_ms": values[math.ceil(len(values) * p / 100) - 1] for p in (50, 95, 99)},
        "max_ms": values[-1],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--counts", type=int, nargs="+", default=[0, 32, 128])
    parser.add_argument("--samples", type=int, default=100)
    parser.add_argument("--expected-barriers", choices=["receipt", "per-directory", "per-pack"], default="receipt")
    args = parser.parse_args()
    if not (1 <= args.samples <= 1000) or args.counts != sorted(set(args.counts)):
        parser.error("Require 1..1000 samples and strictly increasing counts")
    if not args.counts or args.counts[0] < 0 or args.counts[-1] > 4096:
        parser.error("Require counts in 0..4096")
    if args.root.exists():
        parser.error("Use a fresh, nonexistent root")
    args.root.mkdir(parents=True)
    report = {
        "hostname": socket.gethostname(),
        "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
        "scope": "steady repeated GC over historical tombstones, including catalog lock/refresh and directory sync",
        "cases": [],
        "expected_barriers": args.expected_barriers,
        "passed": False,
    }
    with SharedFilesystemStore(args.root, "gc-history-probe", online_gc=True) as store:
        count = 0
        parents = set()
        for target in args.counts:
            refs = []
            paths = []
            for index in range(count, target):
                ref = store.publish([Record(str(index), bytes(1024))], submission_id=str(index))
                store.seal()
                refs.append(ref)
                paths.append(args.root / ref.manifest.segment.path)
            assert len(set(paths)) == target - count
            parents.update(path.parent for path in paths)
            store.release_publications(refs)
            catalog_before = (args.root / "storage.log").stat().st_size
            initial_started = time.monotonic_ns()
            first = store.collect_garbage()
            initial_ms = (time.monotonic_ns() - initial_started) / 1_000_000
            catalog_growth = (args.root / "storage.log").stat().st_size - catalog_before
            assert first["reclaimed_files"] == target - count
            assert first["tombstones"] == target
            assert all(not path.exists() for path in paths)
            # Check barrier dispatch separately; callbacks do not affect timings.
            effects = Counter()
            original_fault = store.backend.fault
            store.backend.fault = lambda phase: effects.update([phase])
            try:
                result = store.collect_garbage()
            finally:
                store.backend.fault = original_fault
            assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
            expected = {"receipt": 0, "per-directory": len(parents), "per-pack": target}[args.expected_barriers]
            assert effects["before_gc_directory_sync"] == expected
            if args.expected_barriers == "receipt":
                assert effects["before_gc_stat"] == effects["before_gc_receipt"] == 0
            assert effects["before_gc_unlink"] == 0
            samples = []
            for _ in range(args.samples):
                started = time.monotonic_ns()
                result = store.collect_garbage()
                elapsed = (time.monotonic_ns() - started) / 1_000_000
                assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
                assert result["tombstones"] == target
                samples.append(elapsed)
            report["cases"].append(
                {
                    "tombstones": target,
                    "parent_directories": len(parents),
                    "directory_syncs_per_call": effects["before_gc_directory_sync"],
                    "payload_stat_attempts_per_call": (
                        effects["before_gc_stat"] if args.expected_barriers == "receipt" else None
                    ),
                    "initial_collect_ms": initial_ms,
                    "initial_collect_catalog_growth_bytes": catalog_growth,
                    "gc_ms": quantiles(samples),
                    "samples_ms": samples,
                }
            )
            count = target
    report["passed"] = True
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
