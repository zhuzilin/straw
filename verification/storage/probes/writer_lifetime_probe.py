"""Observe GC around an actually stopped or killed writer in a private pool.

Stop after the header, outside the catalog lock and inside an authorized write.
Three collector calls are observations, never a failure-detector time bound.
Only stopped writers resume normally; killed-writer leftovers are explicitly
counted and manually removed as private experiment fixtures, not by Straw GC.
"""

import argparse
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native


def make(root):
    return SharedFilesystemStore(root, "writer-lifetime", online_gc=True, segment_target_bytes=64 * 1024 * 1024)


def child(root, size):
    with make(root) as writer:
        stopped = False

        def pause(phase):
            nonlocal stopped
            if phase == "after_segment_header" and not stopped:
                stopped = True
                os.kill(os.getpid(), signal.SIGSTOP)

        writer.backend.fault = pause
        refs = writer.write_records([Record("data", b"x" * size)], submission_id="data")
        assert stopped and writer.read_record(refs[0]).payload == b"x" * size
        writer.release_publications(refs)
    print(json.dumps({"completed": True, "payload_bytes_verified": size}), flush=True)


def collect(root):
    started = time.monotonic()
    with make(root) as collector:
        result = collector.collect_garbage()
    return {"elapsed_ms": (time.monotonic() - started) * 1000, "result": result}


def case(root, size, mode):
    root.mkdir(parents=True, exist_ok=False)
    stdout, stderr = root.parent / (root.name + ".stdout"), root.parent / (root.name + ".stderr")
    with stdout.open("w") as out, stderr.open("w") as err:
        worker = subprocess.Popen(
            [sys.executable, "-I", str(Path(__file__).resolve()), "--child", "--root", str(root), "--size", str(size)],
            stdout=out,
            stderr=err,
        )
        try:
            deadline = time.monotonic() + 60
            while True:
                pid, status = os.waitpid(worker.pid, os.WNOHANG | os.WUNTRACED)
                if pid:
                    assert os.WIFSTOPPED(status) and os.WSTOPSIG(status) == signal.SIGSTOP, (pid, status)
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError("Writer did not reach the header stop; no recovery conclusion")
                time.sleep(0.01)
            (pack,) = root.rglob("*.pack")
            before = pack.read_bytes()
            observations = [collect(root) for _ in range(3)]
            assert all(c["result"]["open_packs"] == 1 and c["result"]["reclaimed_files"] == 0 for c in observations)
            assert pack.read_bytes() == before
            if mode == "paused":
                worker.send_signal(signal.SIGCONT)
                assert worker.wait(timeout=90) == 0, stderr.read_text()
                completed = json.loads(stdout.read_text())
                assert completed == {"completed": True, "payload_bytes_verified": size}
                retired = collect(root)
                assert retired["result"]["open_packs"] == 0 and retired["result"]["reclaimed_files"] == 1
                assert not pack.exists()
                manual_files, manual_bytes, after_death = 0, 0, []
            else:
                worker.kill()
                assert worker.wait(timeout=30) == -signal.SIGKILL
                after_death = [collect(root) for _ in range(3)]
                assert all(c["result"]["open_packs"] == 1 and c["result"]["reclaimed_files"] == 0 for c in after_death)
                assert pack.read_bytes() == before
                completed, retired = None, None
                manual_files, manual_bytes = 1, len(before)
            result = {
                "mode": mode,
                "requested_payload_bytes": size,
                "at_stop_file_bytes": len(before),
                "at_stop_sha256": hashlib.sha256(before).hexdigest(),
                "stopped_gc": observations,
                "after_death_gc": after_death,
                "normal_completion": completed,
                "after_close_gc": retired,
                "manual_fixture_data_files": manual_files,
                "manual_fixture_data_bytes": manual_bytes,
            }
        finally:
            if worker.poll() is None:
                worker.kill()
                worker.wait(timeout=30)
    # This exact directory was exclusively created above; preserve it on failure.
    shutil.rmtree(root)
    result["private_pool_removed"] = not root.exists()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--child", action="store_true")
    parser.add_argument("--size", type=int, default=2048)
    args = parser.parse_args()
    if args.child:
        child(args.root, args.size)
        return
    args.root.mkdir(parents=True, exist_ok=False)
    native = Path(_native.__file__).resolve()
    assert native.is_relative_to(Path(sys.prefix).resolve()), native
    cases = [
        case(args.root / f"{mode}-{size}", size, mode) for mode in ("paused", "killed") for size in (2048, 8388608)
    ]
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
                "cases": cases,
                "scope": "Independent processes on one host; real SIGSTOP/SIGCONT/SIGKILL after header. Storage remains available. Three attempts are finite observations, not a leak proof or a safe timeout. No partition, lock-session expiry, or power loss injected.",
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
