"""Measure identical logical workloads against an installed straw wheel.

Run alternating versions on the same host/filesystem with fresh roots. These
measurements include warm filesystem caches; no cache eviction or SLA inference
is performed. The native library hash binds each sample to its installed build.
"""

import argparse
import hashlib
import json
import math
import os
import random
import shutil
import struct
import sys
import time
from pathlib import Path

import straw
from straw import Record, SharedFilesystemStore, TaskSpec, _native
from straw.coordinator import Coordinator


def timed(call):
    wall, cpu = time.perf_counter(), time.process_time()
    value = call()
    return value, {"seconds": time.perf_counter() - wall, "cpu_seconds": time.process_time() - cpu}


def distribution(samples):
    ordered = sorted(samples)
    return {
        "count": len(samples),
        "quantile_method": "nearest rank",
        "p50_seconds": ordered[math.ceil(len(ordered) * 0.50) - 1],
        "p99_seconds": ordered[math.ceil(len(ordered) * 0.99) - 1],
        "min_seconds": ordered[0],
        "max_seconds": ordered[-1],
        "samples_seconds": samples,
    }


def data(root, record_bytes, publications, online_gc):
    payload = random.Random(8128).randbytes(record_bytes)
    refs, latencies = [], []
    with SharedFilesystemStore(
        root,
        "performance",
        max_record_bytes=record_bytes,
        max_buffer_bytes=record_bytes + 8 * 1024**2,
        segment_target_bytes=256 * 1024**2,
        online_gc=online_gc,
    ) as writer:

        def publish():
            for index in range(publications):
                before = time.perf_counter()
                refs.append(writer.publish([Record(str(index), payload)], submission_id=str(index)))
                latencies.append(time.perf_counter() - before)

        _, write = timed(publish)
    with SharedFilesystemStore(
        root, "performance", max_record_bytes=record_bytes, max_buffer_bytes=record_bytes + 8 * 1024**2
    ) as reader:

        def read():
            count = 0
            for ref in refs:
                for record in reader.read(ref):
                    assert record.payload == payload
                    count += len(record.payload)
            return count

        amount, read = timed(read)
    assert amount == record_bytes * publications
    return {
        "bytes": amount,
        "publications": publications,
        "write": write,
        "read": read,
        "publication_latency": distribution(latencies),
        "write_mib_s": amount / 1024**2 / write["seconds"],
        "read_mib_s": amount / 1024**2 / read["seconds"],
    }


def journal(root, count, replays):
    callback = lambda phase: None
    identity = '{"probe":"runtime-performance"}'
    encoded = json.dumps([{"type": "Probe", "data": "x" * 128}], separators=(",", ":"))
    handle = _native.NativeJournal(str(root), identity, False, False, callback)
    latencies = []

    def append():
        for _ in range(count):
            before = time.perf_counter()
            handle.append(encoded, callback)
            latencies.append(time.perf_counter() - before)

    _, write = timed(append)
    assert json.loads(handle.info())["sequence"] == count + 1
    handle.close()
    recovery = []
    for _ in range(replays):
        recovered, timing = timed(lambda: _native.NativeJournal(str(root), identity, True, False, callback))
        assert json.loads(recovered.info())["sequence"] == count + 1
        recovered.close()
        recovery.append(timing)
    return {"frames": count, "append": write, "append_latency": distribution(latencies), "recovery": recovery}


def catalog(root, count, replays, encoding, paths):
    # Build identical frames for each version, with the selected path encoding.
    # No clients or background workers access this private fixture while written.
    root.mkdir()
    digest = hashlib.sha256()
    names = ["data/pack"] if paths == 1 else [f"data/pack-{index}" for index in range(paths)]
    with (root / "storage.log").open("wb") as stream:
        for sequence in range(count):
            event = {"run_id": "performance", "owners": {f"owner:{sequence}": names}}
            if encoding == "normalized":
                event["path_encoding"] = 1
            raw = json.dumps(event, separators=(",", ":"), sort_keys=True).encode()
            prefix = b"STRGC001" + struct.pack("<QQ", sequence, len(raw))
            header = prefix + hashlib.sha256(prefix).digest()
            frame = header + raw + hashlib.sha256(header + raw).digest() + b"STRGEND1"
            stream.write(frame)
            digest.update(frame)
        stream.flush()
        os.fsync(stream.fileno())
    recovery = []
    for _ in range(replays):
        with SharedFilesystemStore(root, "performance", online_gc=True) as store:
            result, timing = timed(store.collect_garbage)
            assert result["owners"] == count and result["reclaimed_files"] == 0, result
            recovery.append(timing)
    return {
        "frames": count,
        "paths_per_owner": paths,
        "fixture_sha256": digest.hexdigest(),
        "replay_and_snapshot": recovery,
    }


def commits(root, count):
    """Exercise retain/WAL/apply/cleanup and retirement against real data."""
    with SharedFilesystemStore(root, "performance", online_gc=True, segment_target_bytes=0) as store:
        reference = store.publish([Record("input", b"retained input")], submission_id="input")
        with Coordinator(store, exclusive_owner="private probe owns coordinator lifetime") as queue:
            latencies = []

            def submit():
                for index in range(count):
                    before = time.perf_counter()
                    assert queue.submit_tasks(str(index), [TaskSpec(str(index), input_ref=reference)]) == [str(index)]
                    latencies.append(time.perf_counter() - before)

            _, accepted = timed(submit)
            assert len(queue.pending_tasks()) == count

            def retire():
                for index in range(count):
                    queue.cancel_task(str(index), request_id=f"cancel:{index}")

            _, retired = timed(retire)
            result, collected = timed(queue.collect_garbage)
            assert result["reclaimed_files"] == 1 and result["owners"] == 0, result
    return {
        "tasks": count,
        "accept": accepted,
        "accept_latency": distribution(latencies),
        "retire": retired,
        "collect": collected,
        "reclaimed_files": result["reclaimed_files"],
    }


def gc(root, writers, packs, rounds):
    """Time collection after preparing several packs per writer directory."""
    samples = []
    for trial in range(rounds):
        directory = root / str(trial)
        for writer in range(writers):
            with SharedFilesystemStore(directory, "performance", online_gc=True, segment_target_bytes=0) as store:
                references = [
                    store.publish([Record(str(pack), b"garbage")], submission_id=f"{writer}:{pack}")
                    for pack in range(packs)
                ]
                store.seal()
                store.release_publications(references)
        with SharedFilesystemStore(directory, "performance", online_gc=True) as store:
            result, timing = timed(store.collect_garbage)
            assert result["reclaimed_files"] == writers * packs, result
            assert result["owners"] == result["open_packs"] == result["retained_packs"] == 0, result
            assert result["tombstones"] == writers * packs, result
            samples.append(timing)
    return {"writers": writers, "packs_per_writer": packs, "rounds": rounds, "collection": samples}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--label", required=True)
    parser.add_argument("--phase", choices=("all", "data", "journal", "catalog", "commits", "gc"), default="all")
    parser.add_argument("--online-gc", action="store_true", help="Use the ownership catalog during data publication")
    parser.add_argument("--cpu", type=int, help="Pin this process to one allowed CPU for comparisons")
    parser.add_argument("--record-mib", type=int, default=32)
    parser.add_argument("--publications", type=int, default=32)
    parser.add_argument("--wal-frames", type=int, default=500)
    parser.add_argument("--catalog-frames", type=int, default=2000)
    parser.add_argument("--catalog-paths", type=int, default=1)
    parser.add_argument("--commit-tasks", type=int, default=64)
    parser.add_argument("--gc-writers", type=int, default=3)
    parser.add_argument("--gc-packs", type=int, default=8)
    parser.add_argument("--gc-rounds", type=int, default=3)
    parser.add_argument("--replays", type=int, default=5)
    parser.add_argument("--catalog-encoding", choices=("legacy", "normalized"), default="legacy")
    args = parser.parse_args()
    if args.root.exists() or not args.root.is_absolute() or args.report.resolve().is_relative_to(args.root):
        parser.error("Use a fresh absolute root and an external report")
    if (
        min(
            args.record_mib,
            args.publications,
            args.wal_frames,
            args.catalog_frames,
            args.catalog_paths,
            args.replays,
            args.commit_tasks,
            args.gc_writers,
            args.gc_packs,
            args.gc_rounds,
        )
        < 1
    ):
        parser.error("All workload counts must be positive")
    if args.cpu is not None:
        if not hasattr(os, "sched_getaffinity") or args.cpu not in os.sched_getaffinity(0):
            parser.error("Requested CPU affinity is unavailable")
        os.sched_setaffinity(0, {args.cpu})
    native = Path(_native.__file__)
    assert native.is_relative_to(sys.prefix), native
    args.root.mkdir(parents=True)
    result = {
        "label": args.label,
        "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
        "parameters": {k: v for k, v in vars(args).items() if k not in ("root", "report")},
        "scope": "One host, sequential durable publication and authenticated reads, synchronous WAL appends, warm replay. No global cache eviction; catalog replay includes root/candidate snapshots and a recovery barrier. GC times collection with fresh multi-writer fixtures; fixture publication and release are outside its timer.",
    }
    if args.phase in ("all", "data"):
        result["data"] = data(args.root / "data", args.record_mib * 1024**2, args.publications, args.online_gc)
    if args.phase in ("all", "journal"):
        result["journal"] = journal(args.root / "journal", args.wal_frames, args.replays)
    if args.phase in ("all", "catalog"):
        result["catalog"] = catalog(
            args.root / "catalog", args.catalog_frames, args.replays, args.catalog_encoding, args.catalog_paths
        )
    if args.phase in ("all", "commits"):
        result["commits"] = commits(args.root / "commits", args.commit_tasks)
    if args.phase in ("all", "gc"):
        result["gc"] = gc(args.root / "gc", args.gc_writers, args.gc_packs, args.gc_rounds)
    shutil.rmtree(args.root)
    result["temporary_root_removed"] = not args.root.exists()
    result["passed"] = True
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"label": args.label, "passed": True, "data": result.get("data")}))


if __name__ == "__main__":
    main()
