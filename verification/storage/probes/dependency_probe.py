"""Valid deep inline-manifest chains; validation and GC on isolated pools."""

import argparse
import hashlib
import json
import math
import resource
import socket
import sys
import time
from dataclasses import asdict
from pathlib import Path

import straw._native
from straw import Record, SharedFilesystemStore
from straw.protocol import RecordSetRef, digest, encode


def prepare(root, depth, *, corrupt_leaf=False):
    if depth < 1:
        raise ValueError("depth must be positive")
    store = SharedFilesystemStore(root, "dependency-probe", online_gc=True)
    leaf = store.publish([Record("leaf", b"x" * 1024)], submission_id="leaf")
    store.seal()
    previous = asdict(leaf)
    if corrupt_leaf:
        previous["digest"] = "0" * 64
    records = []
    for i in range(depth):
        logical = digest({"records": [], "dependencies": [previous["digest"]]})
        manifest = {"version": 2, "records": [], "dependencies": [previous], "digest": logical}
        records.append(Record(f"node-{i}", encode(manifest), "record-set.v2"))
        previous = {"local": i, "digest": logical, "records": 0, "payload_bytes": 1024, "tokens": 0}
    raw = store.write_records(records, submission_id="chain")
    store.seal()
    previous.pop("local")
    previous["manifest"] = asdict(raw[-1])
    return store, RecordSetRef.from_dict(previous), leaf, raw


def percentiles(samples):
    ordered = sorted(samples)
    return {
        "n": len(samples),
        **{f"p{p}_ms": ordered[math.ceil(len(ordered) * p / 100) - 1] for p in (50, 95, 99)},
        "max_ms": max(ordered),
    }


def run(root, depth, mode, samples, stack_kib):
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    store, reference, leaf, raw = prepare(root, depth)
    report = {
        "hostname": socket.gethostname(),
        "depth": depth,
        "native_sha256": hashlib.sha256(Path(straw._native.__file__).read_bytes()).hexdigest(),
    }
    if stack_kib:
        _, hard = resource.getrlimit(resource.RLIMIT_STACK)
        resource.setrlimit(resource.RLIMIT_STACK, (stack_kib * 1024, hard))
    print(f"operation_started={mode}", file=sys.stderr, flush=True)
    try:
        if mode in ("validate", "measure"):
            latencies = []
            for _ in range(samples if mode == "measure" else 1):
                before = time.perf_counter_ns()
                assert store.validate(reference) == ()
                latencies.append((time.perf_counter_ns() - before) / 1e6)
            report["validate_ms"] = percentiles(latencies)
            report["samples_ms"] = latencies
        if mode in ("retain", "measure"):
            before = time.perf_counter_ns()
            store.retain("root", [reference])
            report["first_retain_ms"] = (time.perf_counter_ns() - before) / 1e6
            store.release_publications([*raw, leaf])
            assert store.collect_garbage()["reclaimed_files"] == 0
            assert list(store.read(leaf))[0].payload == b"x" * 1024
            store.release("root")
            assert store.collect_garbage()["reclaimed_files"] == 2
        report["passed"] = True
        return report
    finally:
        store.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--depth", type=int, default=2048)
    parser.add_argument("--mode", choices=("validate", "retain", "measure"), default="measure")
    parser.add_argument("--samples", type=int, default=50)
    parser.add_argument("--stack-kib", type=int, default=0)
    args = parser.parse_args()
    print(json.dumps(run(args.root, args.depth, args.mode, args.samples, args.stack_kib), sort_keys=True))


if __name__ == "__main__":
    main()
