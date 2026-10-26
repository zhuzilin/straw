"""Check bounded name lookup and visibility retries on private files.

Injected mode uses the installed Straw API and an exact-path libc shim. It
never changes on-disk bytes or sleep behavior. Worker mode separately measures
natural cross-client read-only opens after file/directory fsync; its raw files
are not Straw publications. Neither measurement gives a propagation deadline.
"""

import argparse
import ctypes
import errno
import hashlib
import json
import os
import shutil
import socket
import struct
import tempfile
import time
from pathlib import Path


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def worker():
    for line in __import__("sys").stdin:
        command = json.loads(line)
        op = command["op"]
        if op == "stop":
            return
        if op == "hello":
            result = {"hostname": socket.gethostname(), "pid": os.getpid(), "script_sha256": digest(__file__)}
        elif op == "prime":
            try:
                fd = os.open(command["path"], os.O_RDONLY)
            except FileNotFoundError:
                result = {"errno": errno.ENOENT}
            else:
                os.close(fd)
                raise AssertionError("Expected a new private name")
        elif op == "publish":
            data = hashlib.shake_256(command["seed"].encode()).digest(command["size"])
            path = Path(command["path"])
            start = time.perf_counter_ns()
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            try:
                remaining = memoryview(data)
                while remaining:
                    n = os.write(fd, remaining)
                    assert n > 0
                    remaining = remaining[n:]
                os.fsync(fd)
            finally:
                os.close(fd)
            directory = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
            result = {
                "sha256": hashlib.sha256(data).hexdigest(),
                "size": len(data),
                "publish_ns": time.perf_counter_ns() - start,
            }
        elif op == "read":
            started = time.perf_counter_ns()
            attempts, data = [], bytearray()
            for retry in range(5):
                before = time.perf_counter_ns()
                try:
                    fd = os.open(command["path"], os.O_RDONLY)
                except OSError as error:
                    attempts.append({"ns": time.perf_counter_ns() - before, "errno": error.errno})
                    if error.errno != errno.ENOENT or retry == 4:
                        raise
                    time.sleep((10 << retry) / 1000)
                    continue
                attempts.append({"ns": time.perf_counter_ns() - before, "errno": 0})
                try:
                    while block := os.read(fd, 1024 * 1024):
                        data.extend(block)
                finally:
                    os.close(fd)
                break
            assert len(data) == command["size"] and hashlib.sha256(data).hexdigest() == command["sha256"]
            result = {"attempts": attempts, "read_verify_ns": time.perf_counter_ns() - started, "size": len(data)}
        else:
            raise ValueError(op)
        print(json.dumps(result), flush=True)


def injected(parent, output, interrupts_only=False):
    from straw import Record, SharedFilesystemStore, _native

    shim = ctypes.CDLL(None)
    shim.straw_lookup_begin.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint64, ctypes.c_uint]
    shim.straw_lookup_begin.restype = ctypes.c_int
    shim.straw_lookup_end.argtypes = []
    shim.straw_lookup_end.restype = ctypes.c_char_p
    private = Path(tempfile.mkdtemp(prefix="straw-read-lookup-", dir=parent.resolve()))
    report = {
        "passed": False,
        "hostname": socket.gethostname(),
        "private": str(private),
        "script_sha256": digest(__file__),
        "shim_sha256": digest(os.environ["LD_PRELOAD"]),
        "native_sha256": digest(_native.__file__),
        "cases": [],
        "interrupts_only": interrupts_only,
    }
    # name, open schedule, corrupted successful opens, expected open calls,
    # expected successful descriptors, final exception class, requested sleep ms.
    scenarios = [
        ("present", "", 0, 1, 1, None, 0),
        ("missing_once", "N", 0, 2, 1, None, 10),
        ("missing_four", "NNNN", 0, 5, 1, None, 150),
        ("missing_five", "NNNNN", 0, 5, 0, "StorageUnavailable", 150),
        ("io_error", "E", 0, 1, 0, "StorageUnavailable", 0),
        ("permission", "A", 0, 1, 0, "StorageUnavailable", 0),
        ("quota", "Q", 0, 1, 0, "QuotaExceeded", 0),
        ("space", "F", 0, 1, 0, "QuotaExceeded", 0),
        ("error_after_missing", "NNNE", 0, 4, 0, "StorageUnavailable", 70),
        ("visibility", "P", 1, 2, 2, None, 100),
        ("combined_recover", "NNNNP" * 6, 5, 30, 6, None, 4750),
        ("combined_exhaust", "NNNNP" * 6, 6, 30, 6, "CorruptData", 4750),
        ("missing_after_visibility", "PNNNNN", 1, 6, 1, "StorageUnavailable", 250),
        ("error_after_visibility", "PE", 1, 2, 1, "StorageUnavailable", 100),
    ]
    if interrupts_only:
        scenarios = [
            ("interrupt_once", "I", 0, 2, 1, None, 0),
            ("interrupt_forty", "I" * 40, 0, 41, 1, None, 0),
            ("interrupt_then_missing", "I" * 40 + "NNNNP", 0, 45, 1, None, 150),
            ("interrupt_then_exhaust", "I" * 40 + "NNNNN", 0, 45, 0, "StorageUnavailable", 150),
            ("interrupt_then_error", "I" * 40 + "E", 0, 41, 0, "StorageUnavailable", 0),
        ]
    try:
        with SharedFilesystemStore(private / "pool", "lookup", online_gc=True, segment_target_bytes=0) as store:
            refs = []
            for size in (4096, 64 * 1024**2):
                expected = hashlib.shake_256(str(size).encode()).digest(size)
                (ref,) = store.write_records([Record(str(size), expected)], submission_id=str(size))
                refs.append(ref)
                target = private / "pool" / ref.segment.path
                with target.open("rb") as file:
                    file.seek(ref.segment.offset + 12)
                    metadata_length = struct.unpack("<I", file.read(4))[0]
                payload_offset = ref.segment.offset + 24 + metadata_length
                for mode in ("ordinary", "cold_session", "warm_session"):
                    for name, schedule, corrupt, calls, successes, error_kind, sleep_ms in scenarios:
                        context = store.read_session() if mode != "ordinary" else None
                        reader = context.__enter__() if context else store
                        try:
                            if mode == "warm_session":
                                assert reader.read_record(ref).payload == expected
                            assert (
                                shim.straw_lookup_begin(
                                    str(target).encode(), schedule.encode(), payload_offset, corrupt
                                )
                                == 0
                            )
                            started = time.perf_counter_ns()
                            error = None
                            try:
                                record = reader.read_record(ref)
                            except Exception as caught:
                                error = type(caught).__name__
                            elapsed = time.perf_counter_ns() - started
                            trace = json.loads(shim.straw_lookup_end())
                            assert error == error_kind, (name, mode, error, error_kind)
                            if error is None:
                                assert record.payload == expected
                            assert trace["calls"] == calls and trace["successes"] == successes, (name, trace)
                            assert trace["closes"] == successes and trace["open_fd"] == -1 and not trace["overflow"]
                            assert trace["corruptions"] == corrupt
                            # Every failed data descriptor closes before the next name lookup.
                            live = False
                            for event in trace["events"]:
                                if event["op"] == "open":
                                    assert not live
                                    live = event["result"] >= 0
                                elif event["op"] == "close":
                                    assert live and event["result"] == 0
                                    live = False
                            assert not live
                            assert reader.read_record(ref).payload == expected  # unarmed recovery, unchanged disk
                            report["cases"].append(
                                {
                                    "size": size,
                                    "mode": mode,
                                    "scenario": name,
                                    "schedule": schedule,
                                    "error": error,
                                    "wall_ns": elapsed,
                                    "requested_sleep_ms": sleep_ms,
                                    "trace": trace,
                                }
                            )
                        finally:
                            if context:
                                context.__exit__(None, None, None)
            store.release_publications(refs)
            report["gc"] = store.collect_garbage()
            assert report["gc"]["reclaimed_files"] == 2
            assert report["gc"]["owners"] == report["gc"]["open_packs"] == report["gc"]["retained_packs"] == 0
        assert not [p for p in private.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
        shutil.rmtree(private)
        report.update(passed=True, private_removed=not private.exists())
    finally:
        output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": True, "cases": len(report["cases"]), "gc": report["gc"]}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--interrupts-only", action="store_true")
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.worker:
        worker()
    else:
        assert args.parent and args.output
        injected(args.parent, args.output, args.interrupts_only)


if __name__ == "__main__":
    main()
