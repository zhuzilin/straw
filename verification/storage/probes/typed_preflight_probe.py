"""Measure typed rejection before payload work using installed wheels.

Fresh worker processes isolate reader RSS from tensor construction. rchar deltas
include metadata and /proc instrumentation; they are not physical backend I/O.
Repeating payloads are API/order evidence, not a filesystem bandwidth benchmark.
"""

import argparse
import dataclasses
import hashlib
import json
import resource
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import torch

import straw
from straw import SharedFilesystemStore, _native
from straw.protocol import RecordRef, RecordSetRef
from straw.tensor import TensorRef, publish_tensors


def rchar():
    return int(
        next(line.split()[1] for line in Path("/proc/self/io").read_text().splitlines() if line.startswith("rchar:"))
    )


def worker(fixed):
    raw = json.loads(sys.stdin.read())
    raw["record_ref"] = RecordRef.from_dict(raw["record_ref"])
    raw["blob_set"] = RecordSetRef.from_dict(raw["blob_set"])
    raw["shape"] = tuple(raw["shape"])
    ref = TensorRef(**raw)
    wrong = dataclasses.replace(ref, shape=(ref.nbytes // 2, 2))
    rss_before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    rows = []
    for mode, count in [("bad_full", 50), ("bad_small", 100), ("bad_empty", 100), ("valid_small", 100)]:
        for trial in range(count):
            before = rchar()
            start = time.monotonic()
            try:
                if mode == "bad_full":
                    value = wrong.load()
                elif mode == "bad_small":
                    value = wrong[:32]
                elif mode == "bad_empty":
                    value = wrong[:0]
                else:
                    value = ref[:32]
                outcome = "returned"
            except Exception as error:
                outcome = type(error).__name__
            seconds = time.monotonic() - start
            characters = rchar() - before
            expected = "returned" if mode == "valid_small" or (mode == "bad_empty" and not fixed) else "CorruptData"
            assert outcome == expected, (mode, outcome, expected)
            if mode == "valid_small":
                assert value.tolist() == list(range(32))
            if fixed and mode.startswith("bad") and ref.nbytes >= 4 * 1024**2:
                assert characters < ref.nbytes, (mode, characters, ref.nbytes)
            rows.append(
                {"mode": mode, "trial": trial, "seconds": seconds, "rchar_delta": characters, "outcome": outcome}
            )
    return {
        "passed": True,
        "payload_bytes": ref.nbytes,
        "rss_before_kib": rss_before,
        "rss_after_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
        "rows": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--fixed", action="store_true")
    parser.add_argument("--worker", action="store_true")
    args = parser.parse_args()
    torch.set_num_threads(1)
    if args.worker:
        print(json.dumps(worker(args.fixed)))
        return
    assert args.parent is not None and args.output is not None
    private = Path(tempfile.mkdtemp(prefix="straw-typed-preflight-", dir=args.parent))
    report = {"passed": False, "fixed": args.fixed, "private": str(private), "cases": []}
    try:
        with SharedFilesystemStore(
            private / "pool", "typed-probe", codecs=("tensor.v1",), online_gc=True, segment_target_bytes=0
        ) as store:
            references = []
            file_bytes = 0
            for length in (4096, 4 * 1024**2, 64 * 1024**2):
                (ref,) = publish_tensors(
                    store, {"data": torch.arange(length, dtype=torch.uint8)}, submission_id=str(length)
                )
                references.append(ref.blob_set)
                file_bytes += Path(ref.path).stat().st_size
                result = subprocess.run(
                    [
                        sys.executable,
                        "-I",
                        str(Path(__file__).resolve()),
                        "--worker",
                        *(["--fixed"] if args.fixed else []),
                    ],
                    input=json.dumps(dataclasses.asdict(ref)),
                    capture_output=True,
                    text=True,
                    check=True,
                )
                report["cases"].append(json.loads(result.stdout))
            store.release_publications(references)
            store.seal()
            gc = store.collect_garbage()
            assert gc["reclaimed_files"] == 3 and gc["reclaimed_bytes"] == file_bytes
            assert gc["owners"] == gc["open_packs"] == gc["retained_packs"] == 0
            report["gc"] = gc
        assert not [p for p in private.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]
        shutil.rmtree(private)
        report.update(
            passed=True,
            private_removed=not private.exists(),
            native_sha256=hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
            tensor_sha256=hashlib.sha256(Path(straw.__file__).with_name("tensor.py").read_bytes()).hexdigest(),
            probe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        )
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "cases"}))


if __name__ == "__main__":
    main()
