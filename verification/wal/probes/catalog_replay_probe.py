"""Inject failed catalog replay sync, then keep or lose its unconfirmed tail.

Run under the separately compiled catalog_fsync_fault.c LD_PRELOAD shim on Linux.
Only an armed fsync of this probe's private storage.log fails. Rewriting the tail
simulates storage recovery while the client lives; this is not a power cut or a
claim that this deployment loses such tails. Samples time retry release, not the
injected failure or complete GC. All process/import identities are checked.
"""

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable, UnsafeRecovery


def trial(root, lose_tail, expected):
    log, arm = root / "storage.log", root / "arm"
    os.environ["STRAW_FAIL_SYNC_PATH"] = str(log)
    os.environ["STRAW_FAIL_SYNC_ARM"] = str(arm)
    a = SharedFilesystemStore(root, "replay", online_gc=True)
    b = SharedFilesystemStore(root, "replay", online_gc=True)
    try:
        ref = a.publish([Record("data", b"x" * 2048)], submission_id="source")
        a.seal()
        stable = log.read_bytes()

        def fail_before_sync(at):
            if at == "before_storage_catalog_sync":
                raise OSError(5, "uncertain append")

        b.backend.fault = fail_before_sync
        try:
            b.retain("uncertain", [ref])
        except StorageUnavailable:
            pass
        else:
            raise AssertionError("append did not fail")
        complete = log.read_bytes()
        assert len(complete) > len(stable) and complete.startswith(stable)
        arm.write_text("fail next catalog fsync")
        try:
            a.release_publications([ref])
        except StorageUnavailable as error:
            failure = str(error)
        else:
            raise AssertionError("refresh sync did not fail; check LD_PRELOAD")
        assert not arm.exists(), "fault shim did not consume its arm"
        assert log.read_bytes() == complete
        if lose_tail:
            # Preserve every previously confirmed byte; lose only the failed,
            # unconfirmed append. The actual storage service is not crashed.
            with log.open("wb") as stream:
                stream.write(stable)
                stream.flush()
                os.fsync(stream.fileno())
        started = time.monotonic()
        rejected = False
        try:
            a.release_publications([ref])
        except UnsafeRecovery as error:
            assert expected == "legacy" and lose_tail and "truncated" in str(error), str(error)
            rejected = True
        retry_seconds = time.monotonic() - started
        assert rejected == (expected == "legacy" and lose_tail)
        if rejected:
            a.close()
            a = SharedFilesystemStore(root, "replay", online_gc=True)
            a.release_publications([ref])
        result = a.collect_garbage()
        assert result["owners"] == (0 if lose_tail else 1), result
        assert result["reclaimed_files"] == (1 if lose_tail else 0), result
        if not lose_tail:
            assert next(a.read(ref)).payload == b"x" * 2048
            a.release("uncertain")
            result = a.collect_garbage()
            assert result["reclaimed_files"] == 1 and result["owners"] == 0, result
        assert not (root / ref.manifest.segment.path).exists()
        return {
            "lose_tail": lose_tail,
            "stable_bytes": len(stable),
            "unconfirmed_frame_bytes": len(complete) - len(stable),
            "refresh_failure": failure,
            "retry_seconds": retry_seconds,
            "same_instance_rejected": rejected,
            "final_gc": result,
        }
    finally:
        a.close()
        b.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--samples", type=int, default=1, help="Samples per tail outcome")
    parser.add_argument("--expected", choices=("fixed", "legacy"), default="fixed")
    args = parser.parse_args()
    if args.root.exists() or not args.root.is_absolute() or not 1 <= args.samples <= 200:
        parser.error("Use a fresh absolute root and 1..200 samples per outcome")
    native = Path(_native.__file__)
    assert native.is_relative_to(sys.prefix), native
    cases = []
    for index in range(args.samples):
        for lose_tail in (True, False):
            cases.append(trial(args.root / f"{index}-{lose_tail}", lose_tail, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
                "expected": args.expected,
                "cases": cases,
                "reclaimed_packs": len(cases),
            }
        )
    )


if __name__ == "__main__":
    main()
