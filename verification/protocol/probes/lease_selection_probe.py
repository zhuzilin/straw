"""Measure filtered acquisition on a fresh pool; no payload data or p99 guarantee.

Run once per host with a distinct private --root. Alternate unique and repeated
filters, verify every returned lease, release tasks, then recover the same WAL.
The supervisor owns cleanup after all processes exit. This measures metadata
control operations, not tensor bandwidth or cold-cache behavior.
"""

import argparse
import hashlib
import json
import math
import random
import socket
import time
from pathlib import Path

from straw import SharedFilesystemStore, TaskSpec, _native
from straw.coordinator import Coordinator


def quantiles(values):
    values = sorted(values)
    return {
        "n": len(values),
        "p50_ms": values[math.ceil(len(values) * 0.50) - 1],
        "p95_ms": values[math.ceil(len(values) * 0.95) - 1],
        "p99_ms": values[math.ceil(len(values) * 0.99) - 1],
        "max_ms": values[-1],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--tasks", type=int, default=1024)
    parser.add_argument("--samples", type=int, default=250)
    parser.add_argument("--batch", type=int, default=16)
    parser.add_argument("--repeat", type=int, default=4)
    args = parser.parse_args()
    if not (1 <= args.batch <= args.tasks <= 16384 and 1 <= args.samples <= 1000 and 2 <= args.repeat <= 8):
        parser.error("Require 1 <= batch <= tasks <= 16384, 1..1000 samples, 2..8 repeats")
    if args.root.exists():
        parser.error("Use a fresh, nonexistent root")
    args.root.mkdir(parents=True)
    ids = [f"task:{i:05d}" for i in range(args.tasks)]
    shuffled = ids * args.repeat
    random.Random(41).shuffle(shuffled)
    samples = {"unique": [], "repeated": []}
    report = {
        "hostname": socket.gethostname(),
        "tasks": args.tasks,
        "batch": args.batch,
        "repeat": args.repeat,
        "samples_per_case": args.samples,
        "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
        "passed": False,
        "scope": "metadata acquisition including filter selection, lease WAL append/fsync, and Python/native boundaries",
    }
    with SharedFilesystemStore(args.root, "lease-selection-probe") as store:
        owner = Coordinator(store, exclusive_owner="private probe root")
        try:
            owner.submit_tasks("tasks", [TaskSpec(i) for i in ids])
            started = time.monotonic()
            for index in range(args.samples):
                cases = [("unique", ids), ("repeated", shuffled)]
                if index % 2:
                    cases.reverse()
                for name, filtered in cases:
                    before = time.monotonic_ns()
                    assignments = owner.acquire("worker", args.batch, task_ids=filtered).assignments
                    elapsed_ms = (time.monotonic_ns() - before) / 1_000_000
                    assert len(assignments) == args.batch
                    assert len({a.task.task_id for a in assignments}) == args.batch
                    leases = [a.lease for a in assignments]
                    assert owner.heartbeat(leases) == ["extended"] * args.batch
                    assert (
                        owner.release_tasks(leases, request_id=f"release:{index}:{name}") == ["released"] * args.batch
                    )
                    samples[name].append(elapsed_ms)
            report["round_trip_seconds"] = time.monotonic() - started
            assert len(owner.pending_tasks()) == args.tasks
            journal = owner.journal.path
        finally:
            owner.close()
        recovered = Coordinator(store, exclusive_owner="old owner closed", recover=True)
        try:
            assert len(recovered.pending_tasks()) == args.tasks
        finally:
            recovered.close()
        report["wal_bytes"] = journal.stat().st_size
    report["acquire_ms"] = {name: quantiles(values) for name, values in samples.items()}
    report["samples_ms"] = samples
    report["passed"] = True
    print(json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
