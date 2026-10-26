"""Bounded raw filesystem throughput, with per-batch fsync and cross-client reads.

Fresh files; no cache eviction. This measures application-visible throughput,
not backing object-store bandwidth. Remote workers receive this exact source.
"""

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
import resource
import select
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MIB = 1024**2


def worker():
    buffers = []
    for line in sys.stdin:
        command = json.loads(line)
        op = command["op"]
        if op == "stop":
            return
        if op == "prepare":
            buffers = [os.urandom(8 * MIB) for _ in range(command["streams"])]
            result = {"host": socket.gethostname(), "pid": os.getpid()}
        else:
            before_cpu = time.process_time()
            before = time.perf_counter()

            def io_file(item):
                path = item["path"]
                started = time.perf_counter()
                batches = []
                size = item["bytes"]
                if op == "write":
                    buffer = buffers[item["lane"]]
                    # Independent expected checksum is calculated in prepare_hash,
                    # outside the timed write phase. Data generation is also excluded.
                    with open(path, "xb", buffering=0) as stream:
                        last_sync = time.perf_counter()
                        for offset in range(0, size, len(buffer)):
                            data = memoryview(buffer)
                            while data:
                                n = stream.write(data)
                                if not n:
                                    raise OSError("Short write without progress")
                                data = data[n:]
                            if (offset + len(buffer)) % command["sync_bytes"] == 0 or offset + len(buffer) == size:
                                os.fsync(stream.fileno())
                                batches.append(time.perf_counter() - last_sync)
                                last_sync = time.perf_counter()
                    fd = os.open(str(Path(path).parent), os.O_RDONLY | os.O_DIRECTORY)
                    try:
                        os.fsync(fd)
                    finally:
                        os.close(fd)
                else:
                    digest = hashlib.sha256()
                    got = 0
                    with open(path, "rb", buffering=0) as stream:
                        while data := stream.read(8 * MIB):
                            got += len(data)
                            if op == "read_checked":
                                digest.update(data)
                    if got != size:
                        raise RuntimeError(f"Size mismatch: {got} != {size}")
                    if op == "read_checked" and digest.hexdigest() != item["sha256"]:
                        raise RuntimeError("Payload checksum mismatch")
                return {"bytes": size, "seconds": time.perf_counter() - started, "batch_seconds": batches}

            if op == "prepare_hash":
                result = []
                for i, buffer in enumerate(buffers):
                    digest = hashlib.sha256()
                    for _ in range(command["bytes_per_file"] // len(buffer)):
                        digest.update(buffer)
                    result.append(digest.hexdigest())
            else:
                with concurrent.futures.ThreadPoolExecutor(len(command["files"])) as pool:
                    results = list(pool.map(io_file, command["files"]))
                result = {
                    "files": results,
                    "seconds": time.perf_counter() - before,
                    "cpu_seconds": time.process_time() - before_cpu,
                    "rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
                }
        print(json.dumps(result), flush=True)


class Remote:
    def __init__(self, host, source, timeout):
        self.host = host
        self.timeout = timeout
        self.process = subprocess.Popen(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                host,
                shlex.join(["python3", "-u", "-c", source, "--worker"]),
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def send(self, command):
        self.process.stdin.write(json.dumps(command) + "\n")
        self.process.stdin.flush()

    def receive(self):
        if not select.select([self.process.stdout], [], [], self.timeout)[0]:
            raise TimeoutError(f"Worker {self.host} timed out")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(f"Worker {self.host} exited without a result")
        return json.loads(line)

    def close(self):
        try:
            if self.process.poll() is None:
                self.send({"op": "stop"})
            return self.process.wait(timeout=10) == 0
        except (OSError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait(timeout=10)
            return False


def run(args):
    if not 1 <= args.streams <= 16 or min(args.sync_mib, args.mib_per_host, args.timeout) < 1:
        raise ValueError("Use 1..16 streams and positive volume, sync interval and timeout")
    total = args.mib_per_host * MIB * len(args.hosts)
    per_file = args.mib_per_host * MIB // args.streams
    if len(set(args.hosts)) != len(args.hosts) or not 0 < total <= args.max_gib * 1024**3:
        raise ValueError("Distinct hosts and a volume within max-gib are required")
    if per_file < 8 * MIB or per_file % (8 * MIB) or args.mib_per_host * MIB % args.streams or args.sync_mib % 8:
        raise ValueError("Each file and fsync batch must be a positive multiple of 8 MiB")
    source = Path(__file__).read_text()
    root = Path(tempfile.mkdtemp(prefix="straw-bandwidth-", dir=args.parent))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.with_suffix(".source.py").write_text(source)
    report = {
        "root": str(root),
        "parameters": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
        "started_unix_seconds": time.time(),
        "cache_policy": "Fresh paths, cross-client ring reads. No eviction; checked read follows raw read and can be warm.",
        "payload_policy": "Per-stream random 8 MiB buffer repeated. Generation and expected hashing excluded from timing.",
        "throughput_clock": "One orchestrator monotonic clock from dispatch to all replies; includes control overhead.",
        "passed": False,
        "cleaned": False,
        "phases": {},
    }
    workers = []
    try:
        for host in args.hosts:
            workers.append(Remote(host, source, args.timeout))
        for w in workers:
            w.send({"op": "prepare", "streams": args.streams})
        report["workers"] = [w.receive() for w in workers]
        for w in workers:
            w.send({"op": "prepare_hash", "bytes_per_file": per_file})
        hashes = [w.receive() for w in workers]
        files = [
            [
                {"path": str(root / f"host-{i}-lane-{j}"), "lane": j, "bytes": per_file, "sha256": hashes[i][j]}
                for j in range(args.streams)
            ]
            for i in range(len(workers))
        ]
        for phase in ("write", "read_raw", "read_checked"):
            started = time.perf_counter()
            for i, w in enumerate(workers):
                w.send(
                    {
                        "op": phase,
                        "files": files[i if phase == "write" else (i - 1) % len(workers)],
                        "sync_bytes": args.sync_mib * MIB,
                    }
                )
            results = [w.receive() for w in workers]
            elapsed = time.perf_counter() - started
            batches = sorted(t for host in results for f in host["files"] for t in f["batch_seconds"])
            report["phases"][phase] = {
                "bytes": total,
                "seconds": elapsed,
                "mib_s": total / MIB / elapsed,
                "hosts": results,
                "batch_p99_seconds": batches[math.ceil(len(batches) * 0.99) - 1] if batches else None,
            }
        report["passed"] = True
    finally:
        clean = [w.close() for w in workers]
        report["workers_exited_cleanly"] = len(workers) == len(args.hosts) and all(clean)
        if report["passed"] and report["workers_exited_cleanly"]:
            shutil.rmtree(root)
            report["cleaned"] = True
        report["finished_unix_seconds"] = time.time()
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: {x: y for x, y in v.items() if x != "hosts"} for k, v in report["phases"].items()}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--hosts", nargs="+")
    parser.add_argument("--streams", type=int, default=1)
    parser.add_argument("--mib-per-host", type=int, default=1024)
    parser.add_argument("--sync-mib", type=int, default=16)
    parser.add_argument("--max-gib", type=int, default=16)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.worker:
        worker()
    else:
        if not args.parent or not args.hosts or not args.output:
            parser.error("parent, hosts and output are required")
        run(args)


if __name__ == "__main__":
    main()
