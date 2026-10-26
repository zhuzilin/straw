"""Measure catalog retain/release pairs beside a marked reactivation scan.

Markers coordinate a deployment experiment; they are not a synchronized clock
or a proof of visibility latency. Source owners live until explicit cleanup.
"""

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("run", "cleanup"))
    parser.add_argument("root", type=Path)
    parser.add_argument("identity")
    args = parser.parse_args()
    native = Path(_native.__file__)
    assert native.is_relative_to(sys.prefix), native
    source, target = f"latency-source:{args.identity}", f"latency-target:{args.identity}"
    markers = args.root / "markers"
    with SharedFilesystemStore(args.root, "reactivation-probe", online_gc=True) as store:
        if args.mode == "cleanup":
            store.release(target)
            store.release(source)
            result = store.collect_garbage()
            assert result["reclaimed_files"] == 1, result
            print(json.dumps({"passed": True, "gc": result}))
            return
        ref = store.publish([Record(args.identity, b"x" * 1024)], submission_id=args.identity)
        store.seal()
        store.retain(source, [ref])
        store.release_publications([ref])
        (markers / f"ready-{args.identity}").write_text("ready")
        deadline = time.monotonic() + 900
        while not (markers / "started").exists():
            if (markers / "abort").exists() or time.monotonic() > deadline:
                raise RuntimeError("Scan did not start")
            time.sleep(0.01)
        samples = []
        observed_before_finish = []
        while not (markers / "finished").exists():
            if (markers / "abort").exists() or time.monotonic() > deadline:
                raise RuntimeError("Scan did not finish")
            started = time.monotonic()
            store.retain(target, [ref])
            store.release(target)
            samples.append(time.monotonic() - started)
            observed_before_finish.append(not (markers / "finished").exists())
            time.sleep(0.01)
        print(
            json.dumps(
                {
                    "passed": True,
                    "identity": args.identity,
                    "seconds": samples,
                    "observed_before_finish": observed_before_finish,
                    "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
                }
            )
        )


if __name__ == "__main__":
    main()
