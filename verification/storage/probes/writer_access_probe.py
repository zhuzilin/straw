"""Exercise a writer accessor while its publication is paused in a callback.

The supervisor owns the watchdog and any cleanup after terminating a stuck
process. A watchdog duration is an observation window, not an I/O deadline.
"""

import argparse
import hashlib
import json
import threading
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import ResourceLimitExceeded


def run(root, operation, mode, size, target):
    assert not root.exists()
    writer = SharedFilesystemStore(root, "writer-access", online_gc=True, segment_target_bytes=target)
    warm = writer.write_records([Record("warm", b"protected prefix")], submission_id="warm")
    prefix = (root / warm[0].segment.path).read_bytes()
    payload = b"x" * size
    entered, proceed = threading.Event(), threading.Event()
    outcomes, errors, refs = [], [], []

    def access():
        print(json.dumps({"phase": "access", "operation": operation, "mode": mode}), flush=True)
        start = time.monotonic()
        try:
            if operation == "metrics":
                writer.metrics
            else:
                getattr(writer, operation)()
        except ResourceLimitExceeded as error:
            outcomes.append({"error": type(error).__name__, "seconds": time.monotonic() - start})
        else:
            raise AssertionError("Busy writer accessor unexpectedly succeeded")

    def hook(phase):
        if phase != "before_footer":
            return
        if mode == "callback":
            access()
        else:
            entered.set()
            # Only the supervisor may terminate a stuck child; this wait does
            # not manufacture storage failure or allow the writer to continue.
            proceed.wait()

    writer.backend.fault = hook

    def publish():
        try:
            refs.extend(writer.write_records([Record("data", payload)], submission_id="data"))
        except BaseException as error:
            errors.append(error)
            entered.set()

    if mode == "callback":
        publish()
    else:
        worker = threading.Thread(target=publish)
        worker.start()
        try:
            if not entered.wait(30):
                raise AssertionError("Writer did not reach the callback")
            if errors:
                raise errors[0]
            access()
        finally:
            proceed.set()
            worker.join()
    if errors:
        raise errors[0]
    assert len(outcomes) == len(refs) == 1
    writer.backend.fault = lambda _: None
    assert writer.read_record(warm[0]).payload == b"protected prefix"
    assert (root / warm[0].segment.path).read_bytes()[: len(prefix)] == prefix
    assert writer.read_record(refs[0]).payload == payload
    # A rejected close/seal must not change the writer's lifecycle.
    extra = writer.write_records([Record("after", b"after")], submission_id="after")
    assert writer.read_record(extra[0]).payload == b"after"
    writer.release_publications([*warm, *refs, *extra])
    writer.close()
    with SharedFilesystemStore(root, "writer-access", online_gc=True) as peer:
        gc = peer.collect_garbage()
    assert gc["owners"] == gc["open_packs"] == gc["retained_packs"] == 0
    assert not [p for p in root.rglob("*") if p.suffix in (".partial", ".pack", ".sealed")]
    return {
        "passed": True,
        "operation": operation,
        "mode": mode,
        "payload_bytes": size,
        "target": target,
        "busy_response": outcomes[0],
        "gc": gc,
        "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--operation", choices=("metrics", "seal", "close"), required=True)
    parser.add_argument("--mode", choices=("callback", "thread"), required=True)
    parser.add_argument("--bytes", type=int, default=2048)
    parser.add_argument("--target", type=int, default=64 * 1024 * 1024)
    args = parser.parse_args()
    print(json.dumps(run(args.root, args.operation, args.mode, args.bytes, args.target)), flush=True)


if __name__ == "__main__":
    main()
