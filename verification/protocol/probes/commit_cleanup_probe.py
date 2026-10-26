"""Measure retry/recovery of accepted queue requests after cleanup errors.

Every case has a separate private pool. Leftover data is counted before manual
fixture removal, separately from files reclaimed by Straw GC. No deadline or
bandwidth guarantee is inferred from these small fault-injection samples.
"""

import argparse
import errno
import hashlib
import json
import shutil
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, TaskSpec, _native
from straw.coordinator import Coordinator
from straw.errors import StorageUnavailable


def run(root, size, target, cut, reopen):
    assert not root.exists()
    s = SharedFilesystemStore(root, "commit-cleanup-probe", online_gc=True, segment_target_bytes=target)
    q = Coordinator(s, exclusive_owner="test")
    payload = b"x" * size
    ref = s.publish([Record("input", payload)], submission_id="input")
    s.seal()
    # This owner must survive both the failed release and later queue pruning.
    s.retain("independent-reader", [ref])
    tasks = [TaskSpec("t", input_ref=ref)]
    armed = False

    def fail(at):
        nonlocal armed
        if at == "before_staging_consume":
            armed = True
        if armed and at == cut:
            raise OSError(errno.EIO, "commit cleanup probe")

    s.backend.fault = fail
    try:
        q.submit_tasks("submit", tasks)
    except StorageUnavailable:
        pass
    else:
        raise AssertionError("Fault was not reached")
    s.backend.fault = lambda _: None
    started = time.monotonic()
    if reopen:
        q.close()
        s.close()
        s = SharedFilesystemStore(root, "commit-cleanup-probe", online_gc=True, segment_target_bytes=target)
        q = Coordinator(s, exclusive_owner="previous owner stopped", recover=True)
    assert q.submit_tasks("submit", tasks) == ["t"]
    retry_seconds = time.monotonic() - started
    q.cancel_task("t", request_id="cancel")
    protected = q.collect_garbage()
    assert protected["reclaimed_files"] == 0
    assert next(s.read(ref)).payload == payload
    s.release("independent-reader")
    started = time.monotonic()
    gc = q.collect_garbage()
    gc_seconds = time.monotonic() - started
    q.close()
    s.close()
    leftovers = [p for p in root.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
    result = {
        "payload_bytes": size,
        "target": target,
        "cut": cut,
        "reopen": reopen,
        "retry_seconds": retry_seconds,
        "gc_seconds": gc_seconds,
        "protected_gc": protected,
        "gc": gc,
        "manual_fixture_data_files": len(leftovers),
        "manual_fixture_data_bytes": sum(p.stat().st_size for p in leftovers),
    }
    shutil.rmtree(root)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=1)
    args = parser.parse_args()
    assert not args.root.exists() and args.samples > 0
    results = []
    for size in (2048, 8 * 1024 * 1024):
        for target in (0, 64 * 1024 * 1024):
            for cut in ("before_staging_consume", "after_storage_catalog_part", "after_storage_catalog_sync"):
                for reopen in (False, True):
                    for sample in range(args.samples):
                        result = run(args.root / str(len(results)), size, target, cut, reopen)
                        results.append({**result, "sample": sample})
    args.root.rmdir()
    print(
        json.dumps(
            {
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "root_cleaned": not args.root.exists(),
                "cases": results,
            }
        )
    )


if __name__ == "__main__":
    main()
