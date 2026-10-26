"""Measure raw, checksummed reads after fsync on a private shared-filesystem root.

No fault injection, cache dropping or mount changes. Two append-only packs use
straw's segment framing. Control messages use pipes/SSH, never filesystem flags.
Latency to a verified reply is an upper bound including notification/read/hash/
reply costs, not an estimate of pure cache propagation or a worst-case bound.
"""

import argparse
import collections
import hashlib
import json
import math
import os
import select
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HEADER = b"SLMSEG01\x01\0\0\0"
END = b"SLMEND01"


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def framed_record(sequence, size):
    payload = hashlib.shake_256(str(sequence).encode()).digest(size)
    envelope = {
        "version": 1,
        "record_id": str(sequence),
        "codec": "bytes.v1",
        "metadata": {},
        "tokens": 0,
        "length": size,
        "checksum": hashlib.sha256(payload).hexdigest(),
    }
    meta = encode(envelope)
    frame = len(meta).to_bytes(4, "little") + size.to_bytes(8, "little") + meta + payload
    index = encode(
        {
            "version": 1,
            "run_id": "visibility-probe",
            "segment_id": str(sequence),
            "records": [{"offset": len(HEADER), "envelope": envelope}],
        }
    )
    body = HEADER + frame + index
    checksum = hashlib.sha256(body).digest()
    footer = len(index).to_bytes(8, "little") + hashlib.sha256(index).digest() + checksum + END
    return body + footer, checksum.hex()


def validate_view(data, size, checksum, physical, end):
    failures = []
    if physical < end:
        failures.append("short_stat")
    if len(data) < size:
        failures.append("short_read")
    header = data[:12]
    if header != HEADER:
        if len(header) == 12 and all(a == 0 or a == b for a, b in zip(header, HEADER)):
            failures.append("header_zero" if not any(header) else "header_partial_zero")
        else:
            failures.append("header_other")
    if len(data) == size:
        footer = data[-80:]
        n = int.from_bytes(footer[:8], "little")
        if n > size - 92 or footer[-8:] != END or footer[40:72].hex() != checksum:
            failures.append("footer")
        elif hashlib.sha256(data[size - 80 - n : size - 80]).digest() != footer[8:40]:
            failures.append("index")
        if hashlib.sha256(data[:-80]).hexdigest() != checksum:
            failures.append("body_checksum")
        if header == HEADER and any(f in {"footer", "index", "body_checksum"} for f in failures):
            failures.append("valid_header_with_later_failure")
    return failures


def mount_info(root):
    # Kernel-visible options only: do not read or emit mount credentials/configs.
    candidates = []
    for line in Path("/proc/self/mountinfo").read_text().splitlines():
        fields = line.split()
        target = fields[4].replace("\\040", " ")
        if Path(root).is_relative_to(target):
            sep = fields.index("-")
            candidates.append(
                (
                    len(target),
                    {
                        "mountpoint": target,
                        "device": fields[2],
                        "filesystem": fields[sep + 1],
                        "kernel_options": fields[5],
                        "super_options": fields[sep + 3],
                    },
                )
            )
    return max(candidates, key=lambda item: item[0])[1] if candidates else None


def worker():
    for line in sys.stdin:
        command = json.loads(line)
        if command["op"] == "stop":
            return
        if command["op"] == "hello":
            result = {"hostname": socket.gethostname(), "mount": mount_info(command["root"]), "pid": os.getpid()}
        elif command["op"] == "prime":
            fd = os.open(command["path"], os.O_RDONLY)
            try:
                os.fstat(fd)
                # Prime the existing tail/page, including an EOF observation.
                os.pread(fd, 8192, max(0, command["offset"] - 4096))
            finally:
                os.close(fd)
            result = {"primed": True}
        else:
            started = time.perf_counter_ns()
            deadline = time.monotonic() + command["timeout"]
            counts = collections.Counter()
            attempts = 0
            while True:
                attempts += 1
                fd = None
                try:
                    fd = os.open(command["path"], os.O_RDONLY)
                    physical = os.fstat(fd).st_size
                    # Accumulate short POSIX reads, but do not pad an EOF with zeros.
                    parts = []
                    got = 0
                    while got < command["size"]:
                        part = os.pread(fd, command["size"] - got, command["offset"] + got)
                        if not part:
                            break
                        parts.append(part)
                        got += len(part)
                    failures = validate_view(
                        b"".join(parts),
                        command["size"],
                        command["checksum"],
                        physical,
                        command["offset"] + command["size"],
                    )
                except OSError as error:
                    failures = [f"errno_{error.errno}"]
                finally:
                    if fd is not None:
                        os.close(fd)
                ended = time.perf_counter_ns()
                if not failures or time.monotonic() >= deadline:
                    result = {
                        "ok": not failures,
                        "attempts": attempts,
                        "failures": dict(counts),
                        "final_failures": failures,
                        "probe_ms": (ended - started) / 1e6,
                    }
                    if failures:
                        counts.update(failures)
                        result["failures"] = dict(counts)
                    break
                counts.update(failures)
                time.sleep(command["poll_ms"] / 1000)
        print(json.dumps(result), flush=True)


def distribution(values):
    if not values:
        return None
    ordered = sorted(values)
    return {
        "n": len(ordered),
        "p50": ordered[math.ceil(len(ordered) * 0.50) - 1],
        "p95": ordered[math.ceil(len(ordered) * 0.95) - 1],
        "p99": ordered[math.ceil(len(ordered) * 0.99) - 1],
        "max": ordered[-1],
    }


class Reader:
    def __init__(self, host, root, timeout):
        self.host = host
        self.timeout = timeout
        if host is None:
            command = [sys.executable, "-u", str(Path(__file__).resolve()), "--worker"]
        else:
            # Send source over SSH so code visibility/version cannot bias the data test.
            command = [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                host,
                shlex.join(["python3", "-u", "-c", Path(__file__).read_text(), "--worker"]),
            ]
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        try:
            self.send({"op": "hello", "root": str(root)})
            self.metadata = self.receive()
        except Exception:
            self.close()
            raise

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def receive(self):
        if not select.select([self.process.stdout], [], [], self.timeout + 10)[0]:
            raise TimeoutError(f"Reader {self.host} did not reply")
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(f"Reader {self.host} exited before replying")
        return json.loads(line)

    def close(self):
        forced = False
        try:
            if self.process.poll() is None:
                self.send({"op": "stop"})
            self.process.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            forced = True
            self.process.kill()
            self.process.wait(timeout=10)
        return not forced and self.process.returncode == 0


def run(args):
    parent = args.parent.resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix="straw-visibility-", dir=parent))
    started = time.time()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    snapshot = args.output.with_suffix(".source.py")
    snapshot.write_bytes(Path(__file__).read_bytes())
    readers = []
    report = {
        "root": str(root),
        "started_unix_seconds": started,
        "source_snapshot": str(snapshot),
        "hostname": socket.gethostname(),
        "mount": mount_info(root),
        "samples_per_mode": args.samples,
        "payload_bytes": args.payload_bytes,
        "start_offset": args.start_offset,
        "initial_sparse_hole_bytes_per_pack": args.start_offset,
        "write_budget_bytes": args.max_write_bytes,
        "poll_ms": args.poll_ms,
        "timeout_seconds": args.timeout,
        "metric": "fsync return to validated reply; includes notification, open/read/hash and reply",
        "quantiles": "nearest rank; successful reads only; failures reported separately",
        "client_configuration": "JuiceFS version/writeback/open-cache unknown unless independently supplied",
        "source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "reader_scope": "remote SSH readers" if args.hosts else "same mount, separate local reader processes",
        "raw_samples": [],
        "summary": {},
        "cleaned": False,
    }
    fd = None
    try:
        for host in args.hosts or [None] * args.readers:
            readers.append(Reader(host, root, args.timeout))
        report["readers"] = [r.metadata for r in readers]
        for mode in ("fsync_open", "close_reopen"):
            path = root / (mode + ".pack")
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            if args.start_offset:
                os.ftruncate(fd, args.start_offset)
                os.lseek(fd, args.start_offset, os.SEEK_SET)
            os.fsync(fd)
            directory = os.open(root, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
            offset = args.start_offset
            for sample in range(args.samples):
                data, checksum = framed_record(sample, args.payload_bytes)
                for reader in readers:
                    reader.send({"op": "prime", "path": str(path), "offset": offset})
                for reader in readers:
                    assert reader.receive()["primed"]
                if fd is None:
                    fd = os.open(path, os.O_WRONLY | os.O_APPEND)
                start = time.perf_counter_ns()
                remaining = memoryview(data)
                while remaining:
                    n = os.write(fd, remaining)
                    if n <= 0:
                        raise OSError("write made no progress")
                    remaining = remaining[n:]
                before_sync = time.perf_counter_ns()
                os.fsync(fd)
                synced = time.perf_counter_ns()
                if mode == "close_reopen":
                    os.close(fd)
                    fd = None
                published = time.perf_counter_ns()
                command = {
                    "op": "read",
                    "path": str(path),
                    "offset": offset,
                    "size": len(data),
                    "checksum": checksum,
                    "timeout": args.timeout,
                    "poll_ms": args.poll_ms,
                }
                for reader in readers:
                    reader.send(command)
                observations = []
                for reader in readers:
                    result = reader.receive()
                    # One coordinator clock; valid across hosts without clock sync.
                    result["fsync_to_ack_ms"] = (time.perf_counter_ns() - synced) / 1e6
                    observations.append(result)
                report["raw_samples"].append(
                    {
                        "mode": mode,
                        "sample": sample,
                        "offset": offset,
                        "extent_bytes": len(data),
                        "write_ms": (before_sync - start) / 1e6,
                        "fsync_ms": (synced - before_sync) / 1e6,
                        "close_ms": (published - synced) / 1e6,
                        "readers": observations,
                    }
                )
                offset += len(data)
                if (sample + 1) % 100 == 0:
                    print(f"{mode}: {sample + 1}/{args.samples}", flush=True)
                if any(not observation["ok"] for observation in observations):
                    raise RuntimeError("A read did not validate within the observation window")
            if fd is not None:
                os.close(fd)
                fd = None
        report["passed"] = True
    except Exception as error:
        report["passed"] = False
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        if fd is not None:
            os.close(fd)
        stopped_cleanly = [reader.close() for reader in readers]
        report["workers_exited_cleanly"] = all(stopped_cleanly)
        if not report["workers_exited_cleanly"]:
            report["passed"] = False
        # Only this invocation's new directory, after all owned local workers stop.
        # SSH disconnect cannot prove a remote syscall stopped; retain on failure.
        if report.get("passed") or not args.hosts:
            shutil.rmtree(root)
            report["cleaned"] = True
        for mode in ("fsync_open", "close_reopen"):
            rows = [r for r in report["raw_samples"] if r["mode"] == mode]
            observations = [r for row in rows for r in row["readers"]]
            failures = collections.Counter()
            for observation in observations:
                failures.update(observation["failures"])
            report["summary"][mode] = {
                "publications": len(rows),
                "reads": len(observations),
                "reads_with_failed_attempts": sum(r["attempts"] > 1 or not r["ok"] for r in observations),
                "failed_reads": sum(not r["ok"] for r in observations),
                "failure_observations": dict(failures),
                "fsync_ms": distribution([r["fsync_ms"] for r in rows]),
                "fsync_to_ack_ms": distribution([r["fsync_to_ack_ms"] for r in observations if r["ok"]]),
                "reader_probe_ms": distribution([r["probe_ms"] for r in observations if r["ok"]]),
            }
        report["finished_unix_seconds"] = time.time()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "raw_samples"}, indent=2))
    return report["passed"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--samples", type=int, default=1000)
    parser.add_argument("--payload-bytes", type=int, default=65536)
    parser.add_argument("--start-offset", type=int, default=0, help="Initial sparse hole; not dense-file throughput")
    parser.add_argument("--max-write-bytes", type=int, default=512 * 1024 * 1024, help="Explicit total write budget")
    parser.add_argument("--readers", type=int, default=2)
    parser.add_argument("--hosts", nargs="+", help="SSH hosts with the same parent path; overrides --readers")
    parser.add_argument("--poll-ms", type=float, default=2)
    parser.add_argument("--timeout", type=float, default=10)
    args = parser.parse_args()
    if args.worker:
        worker()
        return
    if args.parent is None or args.output is None:
        parser.error("--parent and --output are required")
    if not (1 <= args.samples <= 10000 and 1024 <= args.payload_bytes <= 128 * 1024 * 1024):
        parser.error("samples must be 1..10000; payload bytes 1024..134217728")
    if not 0 <= args.start_offset <= 8 * 1024**4 or args.max_write_bytes <= 0:
        parser.error("start offset must be 0..8 TiB; write budget must be positive")
    if 2 * args.samples * (args.payload_bytes + 4096) > args.max_write_bytes:
        parser.error("run exceeds --max-write-bytes")
    if not (
        1 <= args.readers <= 4
        and len(args.hosts or []) <= 4
        and 0.1 <= args.poll_ms <= 1000
        and 1 <= args.timeout <= 60
    ):
        parser.error("invalid reader count or timing bounds")
    raise SystemExit(0 if run(args) else 1)


if __name__ == "__main__":
    main()
