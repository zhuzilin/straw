"""Compare installed wheels on one immutable record, with optional diagnostics.

Plain workers have no instrumentation. Stage workers wrap Python call boundaries;
libc workers additionally preload read_latency_trace.c. Wall minus thread CPU
includes blocking and descheduling, not just storage wait. Repeating bytes and
warm repeated reads are diagnostic evidence, not backend bandwidth measurements.
"""

import argparse
import ctypes
import dataclasses
import hashlib
import json
import os
import resource
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import torch

import straw
import straw.store as api
from straw import SharedFilesystemStore, _native
from straw.protocol import RecordRef, RecordSetRef
from straw.tensor import TensorRef, publish_tensors


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def worker(mode):
    raw = json.loads(sys.stdin.readline())
    raw["shape"] = tuple(raw["shape"])
    raw["record_ref"] = RecordRef.from_dict(raw["record_ref"])
    raw["blob_set"] = RecordSetRef.from_dict(raw["blob_set"])
    ref = TensorRef(**raw)
    points = {}

    def stamp(label):
        points[label] = (time.perf_counter_ns(), time.thread_time_ns())

    original_init, original_helper = SharedFilesystemStore.__init__, api._read_tensor_range

    class Proxy:
        def __init__(self, native):
            self.native = native

        def read_tensor_range(self, *args, **kwargs):
            stamp("native_start")
            result = self.native.read_tensor_range(*args, **kwargs)
            stamp("native_end")
            points["checked_bytes"] = result[2]
            return result

    def traced_init(self, *args, **kwargs):
        stamp("init_start")
        original_init(self, *args, **kwargs)
        stamp("init_end")

    def traced_helper(native, *args, **kwargs):
        stamp("helper_start")
        result = original_helper(Proxy(native), *args, **kwargs)
        stamp("helper_end")
        return result

    shim = None
    if mode != "plain":
        SharedFilesystemStore.__init__ = traced_init
        api._read_tensor_range = traced_helper
    if mode == "libc":
        shim = ctypes.CDLL(None)
        shim.straw_read_profile_begin.argtypes = []
        shim.straw_read_profile_begin.restype = None
        shim.straw_read_profile_end.argtypes = []
        shim.straw_read_profile_end.restype = ctypes.c_char_p
    print(
        json.dumps(
            {
                "native_sha256": digest(_native.__file__),
                "tensor_sha256": digest(Path(straw.__file__).with_name("tensor.py")),
                "store_sha256": digest(Path(straw.__file__).with_name("store.py")),
                "shim_sha256": digest(os.environ["LD_PRELOAD"]) if shim else None,
            }
        ),
        flush=True,
    )
    for line in sys.stdin:
        count = json.loads(line)["count"]
        rows = []
        for _ in range(count):
            points = {}
            usage_before = resource.getrusage(resource.RUSAGE_THREAD)
            if shim:
                shim.straw_read_profile_begin()
            stamp("api_start")
            data = ref[:32]
            stamp("api_end")
            usage_after = resource.getrusage(resource.RUSAGE_THREAD)
            trace = json.loads(shim.straw_read_profile_end()) if shim else None
            assert data.tolist() == list(range(32))

            def span(a, b):
                return {"wall_ns": points[b][0] - points[a][0], "cpu_ns": points[b][1] - points[a][1]}

            row = {
                **span("api_start", "api_end"),
                "voluntary_switches": usage_after.ru_nvcsw - usage_before.ru_nvcsw,
                "involuntary_switches": usage_after.ru_nivcsw - usage_before.ru_nivcsw,
            }
            if mode != "plain":
                boundaries = [
                    ("before_init", "api_start", "init_start"),
                    ("store_init", "init_start", "init_end"),
                    ("before_helper", "init_end", "helper_start"),
                    ("encode", "helper_start", "native_start"),
                    ("native", "native_start", "native_end"),
                    ("decode", "native_end", "helper_end"),
                    ("after_helper", "helper_end", "api_end"),
                ]
                row["stages"] = {name: span(a, b) for name, a, b in boundaries}
                row["checked_bytes"] = points["checked_bytes"]
                assert row["checked_bytes"] == min(ref.nbytes, 4 * 1024**2)
                for clock in ("wall_ns", "cpu_ns"):
                    assert sum(s[clock] for s in row["stages"].values()) == row[clock]
                if trace is not None:
                    assert not trace["overflow"]
                    events = trace["events"]
                    assert events
                    for i, event in enumerate(events):
                        end = event["start_ns"] + event["wall_ns"]
                        assert points["api_start"][0] <= event["start_ns"] <= end <= points["api_end"][0]
                        if i:
                            previous = events[i - 1]
                            assert previous["start_ns"] + previous["wall_ns"] <= event["start_ns"]
                        event["stage"] = next(
                            name
                            for name, a, b in boundaries
                            if points[a][0] <= event["start_ns"] <= end <= points[b][0]
                        )
                        event["start_ns"] -= points["api_start"][0]
                    read_bytes = sum(e["result"] for e in events if e["op"] == "read" and e["result"] > 0)
                    assert read_bytes >= row["checked_bytes"]
                    row["libc_read_bytes"] = read_bytes
                    row["events"] = events
            rows.append(row)
        print(json.dumps(rows), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", choices=("plain", "stages", "libc"))
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--baseline-python", type=Path)
    parser.add_argument("--fixed-python", type=Path)
    parser.add_argument("--shim", type=Path)
    parser.add_argument("--bytes", type=int, default=64 * 1024**2)
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--repetitions", type=int, default=4)
    args = parser.parse_args()
    torch.set_num_threads(1)
    if args.worker:
        worker(args.worker)
        return
    assert all((args.parent, args.output, args.baseline_python, args.fixed_python, args.shim))
    assert 32 <= args.bytes <= 100_000_000_000 and args.count > 0 and args.repetitions > 0
    private = Path(tempfile.mkdtemp(prefix="straw-read-latency-", dir=args.parent.resolve()))
    report = {
        "passed": False,
        "private": str(private),
        "script_sha256": digest(__file__),
        "shim_sha256": digest(args.shim),
        "arguments": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        "workers": {},
        "batches": [],
    }
    children = []
    try:
        with SharedFilesystemStore(
            private / "pool", "read-latency", codecs=("tensor.v1",), online_gc=True, segment_target_bytes=0
        ) as store:
            (ref,) = publish_tensors(store, {"v": torch.arange(args.bytes, dtype=torch.uint8)}, submission_id="source")
            for mode in ("plain", "stages", "libc"):
                pair = {}
                for name, python in (("baseline", args.baseline_python), ("fixed", args.fixed_python)):
                    env = dict(os.environ)
                    env.pop("LD_PRELOAD", None)
                    env.pop("STRAW_READ_PROFILE_ROOT", None)
                    if mode == "libc":
                        env.update(LD_PRELOAD=str(args.shim.resolve()), STRAW_READ_PROFILE_ROOT=str(private))
                    child = subprocess.Popen(
                        [str(python), "-I", "-u", str(Path(__file__).resolve()), "--worker", mode],
                        stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        text=True,
                        env=env,
                    )
                    children.append(child)
                    pair[name] = child
                    child.stdin.write(json.dumps(dataclasses.asdict(ref)) + "\n")
                    child.stdin.flush()
                    report["workers"][mode + "/" + name] = json.loads(child.stdout.readline())
                for repetition in range(args.repetitions):
                    for name in ("baseline", "fixed", "fixed", "baseline"):
                        child = pair[name]
                        child.stdin.write(json.dumps({"count": args.count}) + "\n")
                        child.stdin.flush()
                        answer = child.stdout.readline()
                        assert answer, child.stderr.read()
                        rows = json.loads(answer)
                        assert len(rows) == args.count
                        report["batches"].append(
                            {"mode": mode, "variant": name, "repetition": repetition, "rows": rows}
                        )
                for child in pair.values():
                    child.stdin.close()
                    assert child.wait(timeout=30) == 0, child.stderr.read()
            store.release_publications([ref.blob_set])
            store.seal()
            report["gc"] = store.collect_garbage()
            assert report["gc"]["reclaimed_files"] == 1
            assert report["gc"]["owners"] == report["gc"]["open_packs"] == report["gc"]["retained_packs"] == 0
        assert not [p for p in private.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
        shutil.rmtree(private)
        report["private_removed"] = not private.exists()
        report["passed"] = True
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
                child.wait(timeout=30)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": report["passed"], "output": str(args.output), "gc": report["gc"]}))


if __name__ == "__main__":
    main()
