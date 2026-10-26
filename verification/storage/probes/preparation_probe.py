"""Compare uncertain registration cleanup and inject real setup syscall errors.

Raw latencies include different cleanup work in old/new binaries. They provide
observations, not a storage bound. Successful retries write real 2 KiB/8 MiB
records; failed setup has written no payload. Each case owns its private pool.
"""

import argparse
import errno
import hashlib
import json
import os
import shutil
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable

PACKED = 64 * 1024 * 1024


def make(root, target):
    return SharedFilesystemStore(root, "preparation", online_gc=True, segment_target_bytes=target)


def clear():
    for key in ("ROOT", "KIND", "MODE", "ARM", "LOG"):
        os.environ.pop("STRAW_PREP_" + key, None)


def publish(writer, size):
    return writer.write_records([Record("data", b"x" * size)], submission_id="data")


def case(root, size, target, phase, expected, injection=None, traces=None):
    clear()
    root.mkdir(parents=True, exist_ok=False)
    writer = make(root, target)
    attempts, observed, trace = [], [], []
    for attempt in range(2 if phase != "healthy" and injection is None else int(injection is not None)):
        seen = 0

        def fault(at):
            nonlocal seen
            observed.append(at)
            if at == phase:
                seen += 1
                if seen == (3 if phase == "after_storage_catalog_part" else 1):
                    raise OSError(errno.EIO, "private registration response lost")

        writer.backend.fault = fault
        if injection:
            kind, mode = injection.split("_")
            traces.mkdir(parents=True, exist_ok=False)
            arm, log = traces / "arm", traces / "trace.tsv"
            os.environ.update(
                STRAW_PREP_ROOT=str(root),
                STRAW_PREP_KIND=kind,
                STRAW_PREP_MODE=mode,
                STRAW_PREP_ARM=str(arm),
                STRAW_PREP_LOG=str(log),
            )
            arm.touch()
        start = time.monotonic()
        try:
            publish(writer, size)
        except StorageUnavailable as error:
            attempts.append({"elapsed_ms": (time.monotonic() - start) * 1000, "error": str(error)})
        else:
            raise AssertionError("Missing setup failure")
        if injection:
            assert not arm.exists()
            trace = [line.split("\t") for line in log.read_text().splitlines()]
            assert len(trace) == 1 and trace[0][:2] == [kind, mode]
            actual = int(trace[0][2])
            assert actual == -2 if mode == "before" else actual >= 0
            clear()
    writer.backend.fault = lambda _: None
    with make(root, target) as peer:
        before = peer.collect_garbage()
    expected_open = 0 if expected == "fixed" or phase == "healthy" else len(attempts)
    assert before["open_packs"] == expected_open and before["owners"] == 0, before
    assert "after_segment_header" not in observed
    started = time.monotonic()
    refs = publish(writer, size)
    publication_ms = (time.monotonic() - started) * 1000
    assert writer.read_record(refs[0]).payload == b"x" * size
    ancestor_edges = len((root / refs[0].segment.path).parent.resolve().parents)
    writer.release_publications(refs)
    writer.close()
    with make(root, target) as peer:
        after = peer.collect_garbage()
    assert after["open_packs"] == expected_open and after["owners"] == 0
    assert after["reclaimed_files"] == 1
    payloads = [p for p in root.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]
    assert not payloads, payloads
    result = {
        "size": size,
        "target": target,
        "phase": phase,
        "injection": injection,
        "attempts": attempts,
        "before_retry_gc": before,
        "publication_ms": publication_ms,
        "after_close_gc": after,
        "trace": trace,
        "cleanup_parent_repairs": observed.count("before_incomplete_parent_repair"),
        "directory_sync_hook_calls": observed.count("before_directory_sync"),
        "ancestor_edges": ancestor_edges,
        "manual_fixture_open_reservations": after["open_packs"],
        "manual_fixture_data_files": 0,
    }
    shutil.rmtree(root)
    result["private_pool_removed"] = not root.exists()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--inject", action="store_true")
    parser.add_argument("--traces", type=Path)
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=False)
    native = Path(_native.__file__).resolve()
    assert native.is_relative_to(Path(sys.prefix).resolve())
    if args.inject:
        assert args.expected == "fixed" and args.traces is not None
        configurations = [(target, mode) for target in (0, PACKED) for mode in ("open_before", "open_after")] + [
            (PACKED, mode) for mode in ("seek_before", "seek_after")
        ]
        cases = [
            case(args.root / str(i), 8388608, target, "syscall", args.expected, mode, args.traces / str(i))
            for i, (target, mode) in enumerate(configurations)
        ]
    else:
        cases = [
            case(args.root / f"{target}-{size}-{phase}-{i}", size, target, phase, args.expected)
            for target in (0, PACKED)
            for size in (2048, 8388608)
            for phase in (
                "healthy",
                "after_storage_catalog_part",
                "before_storage_catalog_sync",
                "after_storage_catalog_sync",
            )
            for i in range(5)
        ]
    print(
        json.dumps(
            {"passed": True, "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(), "cases": cases},
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
