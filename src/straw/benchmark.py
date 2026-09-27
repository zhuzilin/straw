"""Local or multi-host queue benchmark with durable control and checked payload I/O.

Run `python -m straw.benchmark run --help`. No application or GPU dependencies.
A profile is a JSON list of record-size lists, e.g. [[12582912,4194304,4194304]].
`profile` extracts the observed publication distribution from STRAW_TRACE_DIR.
"""

import argparse
import concurrent.futures
import json
import os
import random
import resource
import secrets
import shlex
import shutil
import socket
import subprocess
import sys
import tarfile
import threading
import time
from collections import defaultdict
from pathlib import Path

from . import Record, RecordSetRef, SharedFilesystemStore, TaskSpec
from .coordinator import Coordinator
from .protocol import Lease, Limits, encode
from .reporting import write_report
from .rpc import QueueClient, serve


def percentile(values, q):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(q * (len(ordered) - 1)))] if ordered else 0


def completion_peak(events, window, *, checked=False):
    if checked and any(e["op"] == "read_range" and "checked_bytes" not in e for e in events):
        return None
    buckets = defaultdict(int)
    for event in events:
        buckets[int(event["time"] // window)] += (
            event.get("checked_bytes", event["bytes"]) if checked else event["bytes"]
        )
    return max(buckets.values(), default=0) / window / 1024**2


def profile(args):
    events = [json.loads(line) for path in Path(args.trace).glob("*.jsonl") for line in path.read_text().splitlines()]
    writes = [e for e in events if e["op"] == "write"]
    reads = [e for e in events if e["op"] in ("read", "read_range")]
    payload_writes = [
        {**e, "bytes": sum(r["bytes"] for r in e["records"] if not r["codec"].startswith("record-set."))}
        for e in writes
    ]
    sizes = [[r["bytes"] for r in e["records"] if not r["codec"].startswith("record-set.")] for e in writes]
    sizes = [s for s in sizes if s and sum(s) > 0]
    if not sizes:
        raise ValueError("Trace contains no payload publications")
    write_bytes = sum(e["bytes"] for e in writes)
    read_bytes = sum(e["bytes"] for e in reads)
    checked_bytes = (
        sum(e.get("checked_bytes", e["bytes"]) for e in reads)
        if all(e["op"] != "read_range" or "checked_bytes" in e for e in reads)
        else None
    )
    write_report(
        args.output,
        {
            "version": 1,
            "publications": sizes,
            "source_trace": str(Path(args.trace).resolve()),
            "observed_write_bytes": write_bytes,
            "observed_write_payload_bytes": sum(e["bytes"] for e in payload_writes),
            "write_bytes_semantics": "Published extent bytes including framing/manifests; excludes journal I/O",
            "observed_read_bytes": read_bytes,
            "read_bytes_semantics": "Returned payload bytes; excludes index and journal I/O",
            "checked_read_bytes": checked_bytes,
            "observed_seconds": max(e["time"] for e in events) - min(e["time"] for e in events),
            "hosts": sorted({e["host"] for e in events}),
            "read_to_extent_write_ratio": read_bytes / max(1, write_bytes),
            "checked_read_amplification": (
                checked_bytes / read_bytes if checked_bytes is not None and read_bytes else None
            ),
            "write_p99_seconds": percentile([e["seconds"] for e in writes], 0.99),
            "payload_completion_rates": [
                {
                    "window_seconds": window,
                    "write_peak_mib_s": completion_peak(payload_writes, window),
                    "extent_write_peak_mib_s": completion_peak(writes, window),
                    "read_peak_mib_s": completion_peak(reads, window),
                    "checked_read_peak_mib_s": completion_peak(reads, window, checked=True),
                }
                for window in (1, 10)
            ],
            "completion_rates_semantics": (
                "Payload bytes credited at operation completion in wall-clock-aligned windows; "
                "requires aligned host clocks. Not physical filesystem or object-store bandwidth."
            ),
            "record_shapes": [r for e in writes for r in e["records"] if "shape" in r],
        },
    )


def load_sizes(args):
    if args.profile:
        data = json.loads(Path(args.profile).read_text())
        publications = list(data["publications"])
        random.Random(0).shuffle(publications)
        width = args.publications_per_task
        return [
            [size for publication in publications[i : i + width] for size in publication]
            for i in range(0, len(publications), width)
        ]
    if args.record_bytes:
        return [args.record_bytes]
    # Qwen3-30B-A3B-sized routes: token rows x 48 layers x 8 experts x int32.
    # SC: response rows x 128 candidates x (int32 IDs + float32 log-probs).
    # This is an explicit shape model, not a claim about measured training traffic.
    rows = args.tokens
    return [[rows * 48 * 8 * 4, rows * 128 * 4, rows * 128 * 4] * args.samples]


def stop_remote_worker(host, python, root):
    """Confirm the owned remote worker is gone before deleting its shared root.

    SSH exiting does not prove its child exited. Match the unique fresh root
    and module argv; never trust a stale PID alone or stop another run.
    """
    script = """
import os, signal, sys, time
from pathlib import Path
root = sys.argv[1]
def owned():
    result = []
    for path in Path('/proc').glob('[0-9]*/cmdline'):
        try:
            args = path.read_bytes().decode().split('\\0')
        except (FileNotFoundError, ProcessLookupError, PermissionError, UnicodeError):
            continue
        if 'straw.benchmark' in args and 'worker' in args and root in args:
            result.append(int(path.parent.name))
    return result
deadline = time.monotonic() + 30
while True:
    pids = owned()
    if not pids:
        break
    if time.monotonic() > deadline:
        raise RuntimeError('Owned benchmark workers did not stop: ' + str(pids))
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    time.sleep(.1)
"""
    subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, shlex.join([python, "-c", script, str(root)])],
        check=True,
        timeout=40,
    )


def worker(args):
    root = Path(args.root)
    write_report(root / f"worker-{args.index}.pid.json", {"pid": os.getpid()})
    token = (root / "rpc.token").read_text()
    client = QueueClient(args.address, token=token)
    sizes = load_sizes(args)
    max_record = max(max(s) for s in sizes)
    max_batch = max(sum(s) for s in sizes) + 8 * 1024**2
    start_at = args.start_at
    cpu = time.process_time()
    while time.time() < start_at:
        time.sleep(min(0.1, start_at - time.time()))
    started = time.monotonic()
    totals = {"written_bytes": 0, "read_bytes": 0, "publications": 0, "reads": 0}
    write_latencies = []
    read_latencies = []
    commit_latencies = []
    lock = threading.Lock()
    failed = threading.Event()

    def writer(rank):
        store = SharedFilesystemStore(
            root / "queue",
            "benchmark",
            max_record_bytes=max_record,
            max_buffer_bytes=max_batch,
            segment_target_bytes=args.segment_mib * 1024**2,
            online_gc=args.online_gc,
        )
        rng = random.Random(args.index * 1000 + rank)
        # Bound reusable input buffers to one publication per writer; no zero-fill shortcut.
        cache = {}
        prefix = f"{args.index}:{rank}:"
        done = 0
        try:
            while not failed.is_set() and not (root / "STOP").exists():
                page = client.call(
                    "acquire",
                    worker_id=f"host-{args.index}-writer-{rank}",
                    max_tasks=1,
                    task_prefix=prefix,
                )
                if not page["assignments"]:
                    break
                assignment = page["assignments"][0]
                lease = Lease(**assignment["lease"])
                shape = sizes[done % len(sizes)]
                cache = {n: cache[n] if n in cache else rng.randbytes(n) for n in set(shape)}
                before = time.monotonic()
                ref = store.publish(
                    [
                        Record(
                            f"{lease.task_id}:{i}",
                            cache[n],
                            metadata={
                                "task_id": lease.task_id,
                                "attempt_id": lease.attempt_id,
                            },
                        )
                        for i, n in enumerate(shape)
                    ],
                    submission_id=lease.task_id,
                )
                durable = time.monotonic()
                client.call(
                    "complete_task",
                    lease=lease,
                    submission_id=lease.task_id,
                    result_ref=ref,
                    result_digest=ref.digest,
                )
                after = time.monotonic()
                with lock:
                    totals["written_bytes"] += sum(shape)
                    totals["publications"] += 1
                    write_latencies.append(durable - before)
                    commit_latencies.append(after - durable)
                done += 1
        finally:
            store.close()

    def reader(rank):
        store = SharedFilesystemStore(
            root / "queue",
            "benchmark",
            max_record_bytes=max_record,
            max_buffer_bytes=max_batch,
        )
        cursor = 0
        while not failed.is_set() and not (root / "STOP").exists():
            page = client.call("read_commits", cursor=cursor, limit=64)
            for receipt in page["commits"]:
                producer = int(receipt["task_id"].split(":")[0])
                # Ring readers always consume a different client's writes.
                if (args.fanout == args.host_count or producer == (args.index - 1) % args.host_count) and receipt[
                    "position"
                ] % args.readers == rank:
                    before = time.monotonic()
                    ref = RecordSetRef.from_dict(receipt["result_ref"])
                    amount = sum(len(record.payload) for record in store.read(ref))
                    with lock:
                        totals["read_bytes"] += amount
                        totals["reads"] += 1
                        read_latencies.append(time.monotonic() - before)
            cursor = page["cursor"]
            if page["end_of_input"]:
                break
            if not page["commits"]:
                time.sleep(0.02)

    with concurrent.futures.ThreadPoolExecutor(args.writers + args.readers) as pool:
        futures = [pool.submit(writer, i) for i in range(args.writers)] + [
            pool.submit(reader, i) for i in range(args.readers)
        ]
        for future in concurrent.futures.as_completed(futures):
            try:
                future.result()
            except BaseException:
                failed.set()
                raise
    seconds = time.monotonic() - started
    write_report(
        root / f"worker-{args.index}.json",
        {
            **totals,
            "host": socket.gethostname(),
            "index": args.index,
            "seconds": seconds,
            "cpu_seconds": time.process_time() - cpu,
            "rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            "write_p50_seconds": percentile(write_latencies, 0.5),
            "write_p99_seconds": percentile(write_latencies, 0.99),
            "read_p50_seconds": percentile(read_latencies, 0.5),
            "read_p99_seconds": percentile(read_latencies, 0.99),
            "commit_p99_seconds": percentile(commit_latencies, 0.99),
        },
    )


def run(args):
    args.hosts = ["localhost"] if args.local else args.hosts
    host_count = len(args.hosts)
    if len(set(args.hosts)) != host_count:
        raise ValueError("SSH hosts must be distinct")
    if args.fanout not in (1, host_count):
        raise ValueError("fanout must be 1 (ring) or the number of hosts (all readers)")
    if min(args.writers, args.readers, args.timeout, args.max_files, args.segment_mib) <= 0:
        raise ValueError("Worker counts, timeout, file budget and pack size must be positive")
    if not 0 < args.gib <= args.max_gib:
        raise ValueError("Requested GiB must be positive and fit max-gib")
    root = Path(args.root).resolve()
    report_path = Path(args.report).resolve()
    if report_path.is_relative_to(root.resolve()):
        raise ValueError("Report must live outside the temporary root")
    if args.publications_per_task < 1:
        raise ValueError("publications-per-task must be positive")
    sizes = load_sizes(args)
    if not sizes or any(not row or any(type(n) is not int or n <= 0 for n in row) for row in sizes):
        raise ValueError("Workload must contain nonempty publications of positive integer byte sizes")
    average = sum(map(sum, sizes)) / len(sizes)
    count = max(1, int(args.gib * 1024**3 / (host_count * args.writers * average)))
    volume = host_count * args.writers * sum(sum(sizes[i % len(sizes)]) for i in range(count))
    files_per_writer, current = 0, 0
    for i in range(count):
        publication = sum(sizes[i % len(sizes)]) + 4096 * (len(sizes[i % len(sizes)]) + 1)
        if not current or current + publication > args.segment_mib * 1024**2:
            files_per_writer += 1
            current = 0
        current += publication
    if volume > args.max_gib * 1024**3 or files_per_writer * host_count * args.writers + 32 > args.max_files:
        raise ValueError("Requested volume exceeds the benchmark storage/file budget")
    root.mkdir(parents=True, exist_ok=False)
    token = secrets.token_hex(32)
    (root / "rpc.token").write_text(token)
    (root / "rpc.token").chmod(0o600)
    # Store limits are tied to this finite test volume. GC remains opt-in.
    limits = Limits(
        pending_tasks=max(10000, host_count * args.writers * count),
        inflight_tasks=256,
        accepted_bytes=int(args.gib * 2 * 1024**3 + 4 * 1024**3),
        accepted_records=10000000,
        max_result_bytes=max(sum(s) for s in sizes) + 8 * 1024**2,
    )
    store = SharedFilesystemStore(
        root / "queue",
        "benchmark",
        max_record_bytes=max(max(s) for s in sizes),
        max_buffer_bytes=max(sum(s) for s in sizes) + 8 * 1024**2,
        online_gc=args.online_gc,
    )
    coordinator = Coordinator(
        store,
        limits=limits,
        exclusive_owner="benchmark orchestrator owns coordinator lifecycle",
    )
    ready = threading.Event()
    address = []

    def callback(value):
        address.append(value)
        ready.set()

    server = threading.Thread(
        target=serve,
        args=(coordinator, (args.bind, 0)),
        kwargs={"token": token, "ready": callback},
        daemon=True,
    )
    server.start()
    ready.wait(30)
    if not address:
        raise RuntimeError("Coordinator did not bind")
    endpoint = f"http://{args.advertise_host or args.hosts[0]}:{address[0][1]}"
    for host in range(host_count):
        for writer_id in range(args.writers):
            for start in range(0, count, 128):
                coordinator.submit_tasks(
                    f"submit-{host}-{writer_id}-{start}",
                    [
                        TaskSpec(
                            f"{host}:{writer_id}:{i}",
                            estimated_bytes=sum(sizes[i % len(sizes)]),
                        )
                        for i in range(start, min(count, start + 128))
                    ],
                )
    coordinator.seal_input("all-inputs")
    handles = []
    start_at = time.time() + 5
    completed = False
    try:
        for index, host in enumerate(args.hosts):
            command = [
                sys.executable if args.local else args.python,
                "-m",
                "straw.benchmark",
                "worker",
                "--root",
                str(root),
                "--address",
                endpoint,
                "--index",
                str(index),
                "--host-count",
                str(host_count),
                "--start-at",
                str(start_at),
                "--writers",
                str(args.writers),
                "--readers",
                str(args.readers),
                "--fanout",
                str(args.fanout),
                "--tokens",
                str(args.tokens),
                "--samples",
                str(args.samples),
                "--segment-mib",
                str(args.segment_mib),
                "--publications-per-task",
                str(args.publications_per_task),
            ]
            if args.online_gc:
                command += ["--online-gc"]
            if args.profile:
                command += ["--profile", str(Path(args.profile).resolve())]
            if args.record_bytes:
                command += ["--record-bytes", *map(str, args.record_bytes)]
            if args.source_dir:
                command = ["env", f"PYTHONPATH={Path(args.source_dir).resolve()}", *command]
            log = (root / f"worker-{index}.log").open("w")
            process = subprocess.Popen(
                command if args.local else ["ssh", "-o", "BatchMode=yes", host, shlex.join(command)],
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            handles.append((host, index, process, log))
        deadline = time.monotonic() + args.timeout
        next_gc = time.monotonic() + 5
        concurrent_gc = []
        while any(process.poll() is None for _, _, process, _ in handles):
            if args.online_gc and time.monotonic() >= next_gc:
                concurrent_gc.append(coordinator.collect_garbage())
                next_gc = time.monotonic() + 5
            files = [p for p in root.rglob("*") if p.is_file()]
            if len(files) > args.max_files or sum(p.stat().st_size for p in files) > args.max_gib * 1024**3:
                raise RuntimeError("Benchmark temporary storage limit exceeded")
            if time.monotonic() > deadline:
                raise TimeoutError("Benchmark workers exceeded the deadline")
            for _, _, process, log in handles:
                if process.poll() not in (None, 0):
                    raise RuntimeError(f"Worker failed ({process.returncode}): {log.name}")
            time.sleep(1)
        for _, _, process, log in handles:
            if process.returncode:
                raise RuntimeError(f"Worker failed ({process.returncode}): {log.name}")
        reports = [json.loads((root / f"worker-{i}.json").read_text()) for i in range(host_count)]
        files = [p for p in (root / "queue").rglob("*") if p.is_file()]
        duration = max(r["seconds"] for r in reports)
        written = sum(r["written_bytes"] for r in reports)
        read = sum(r["read_bytes"] for r in reports)
        if sum(r["publications"] for r in reports) != host_count * args.writers * count:
            raise AssertionError("Workers stopped before completing the planned task population")
        if read != written * args.fanout:
            raise AssertionError(f"Read volume mismatch: {read} != {written} x {args.fanout}")
        report = {
            "version": 1,
            "status": "passed",
            "hosts": args.hosts,
            "workers": reports,
            "elapsed_seconds": duration,
            "written_bytes": written,
            "read_bytes": read,
            "write_mib_s": written / 1024**2 / duration,
            "read_mib_s": read / 1024**2 / duration,
            "file_count": len(files),
            "data_files": sum(p.suffix in (".sealed", ".pack") for p in files),
            "physical_bytes": sum(p.stat().st_size for p in files),
            "publications": sum(r["publications"] for r in reports),
            "coordinator": coordinator.metrics(),
            "parameters": vars(args),
            "cache_policy": "Fresh paths; same-client reads in local mode, ring reads across remote workers otherwise. No cache eviction; not a cold backing-store certificate.",
            "profile_source": (
                "measured trace"
                if args.profile
                else "explicit record sizes" if args.record_bytes else "Qwen3 R3 + SC size model"
            ),
        }
        if args.online_gc:
            started_gc = time.monotonic()
            token = coordinator.open_consumer("training", exclusive_owner="all benchmark readers joined")
            progress = store.publish(
                [
                    Record(
                        "progress",
                        encode({"version": 1, "processed_positions": [], "finished_batches": []}),
                        "json.v1",
                    )
                ],
                submission_id="readers-finished",
            )
            coordinator.save_consumer_state(
                "training",
                token=token,
                request_id="all-readers-finished",
                state_ref=progress,
                progress_ref=progress,
                fetch_cursor=report["publications"],
                processed_cursor=report["publications"],
            )
            store.seal()
            report["gc"] = coordinator.collect_garbage()
            report["gc_seconds"] = time.monotonic() - started_gc
            report["concurrent_gc_passes"] = concurrent_gc
            if report["gc"]["reclaimed_bytes"] < written:
                raise AssertionError("Acknowledged benchmark packs were not reclaimed")
        write_report(root / "report.json", report)
        write_report(report_path, report)
        print(
            json.dumps(
                {k: v for k, v in report.items() if k not in ("workers", "coordinator", "parameters")},
                indent=2,
            )
        )
        completed = True
    finally:
        (root / "STOP").touch()
        for host, index, process, log in handles:
            if not args.local:
                # Failure to inspect/stop the remote host preserves this root.
                stop_remote_worker(host, args.python, root)
            if process.poll() is None:
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.terminate()  # Remote workers are already confirmed stopped.
                    process.wait(timeout=30)
            log.close()
        coordinator.close()
        report_path.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(report_path.with_suffix(".logs.tar.gz"), "w:gz") as archive:
            for path in root.iterdir():
                if path.suffix in {".log", ".json"}:
                    archive.add(path, arcname=path.name)
        files = [p for p in root.rglob("*") if p.is_file()]
        write_report(
            report_path.with_suffix(".cleanup.json"),
            {
                "root": str(root),
                "files": len(files),
                "bytes": sum(p.stat().st_size for p in files),
                "workers_stopped": True,
                "removed": completed,
            },
        )
        if completed:
            shutil.rmtree(root)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="role", required=True)
    extract = sub.add_parser("profile")
    extract.add_argument("--trace", required=True)
    extract.add_argument("--output", required=True)
    for name in ("run", "worker"):
        r = sub.add_parser(name)
        r.add_argument("--root", required=True)
        r.add_argument("--online-gc", action="store_true")
        workload = r.add_mutually_exclusive_group()
        workload.add_argument("--profile", help="JSON workload from the profile command")
        workload.add_argument("--record-bytes", nargs="+", type=int, help="Byte sizes of records in each publication")
        r.add_argument("--publications-per-task", type=int, default=1)
        r.add_argument("--writers", type=int, default=2)
        r.add_argument("--readers", type=int, default=2)
        r.add_argument("--fanout", type=int, default=1)
        r.add_argument("--tokens", type=int, default=8192)
        r.add_argument("--samples", type=int, default=4)
        r.add_argument("--segment-mib", type=int, default=256)
        if name == "run":
            location = r.add_mutually_exclusive_group(required=True)
            location.add_argument("--hosts", nargs="+", help="Distinct SSH hosts; launch on the first host")
            location.add_argument("--local", action="store_true", help="Run one local worker process without SSH")
            r.add_argument(
                "--python", default="python3", help="Python executable on remote hosts with the wheel installed"
            )
            r.add_argument(
                "--source-dir", help="Optional shared src directory for development; wheels need no source tree"
            )
            r.add_argument(
                "--advertise-host", help="Reachable address of this coordinator, if different from the first SSH host"
            )
            r.add_argument("--gib", type=float, default=32)
            r.add_argument("--report", required=True)
            r.add_argument("--max-files", type=int, default=256)
            r.add_argument("--max-gib", type=float, default=64)
            r.add_argument("--bind", default="0.0.0.0")
            r.add_argument("--timeout", type=int, default=1800)
        else:
            r.add_argument("--address", required=True)
            r.add_argument("--index", type=int, required=True)
            r.add_argument("--host-count", type=int, default=4)
            r.add_argument("--start-at", type=float, required=True)
    args = p.parse_args()
    {"run": run, "worker": worker, "profile": profile}[args.role](args)


if __name__ == "__main__":
    main()
