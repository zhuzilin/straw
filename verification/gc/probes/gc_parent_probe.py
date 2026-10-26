"""Measure GC completion with empty reservations and test parent syscall errors.

No payload is written into failed reservations. Actual retained/garbage records
exercise ownership and deletion. Baseline missing-parent fixtures require an
explicit, separately reported manual directory repair before normal GC finishes.
"""

import argparse
import hashlib
import json
import os
import struct
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import StorageUnavailable


def make(root, target=0):
    return SharedFilesystemStore(root, "parent-probe", online_gc=True, segment_target_bytes=target)


def events(root):
    raw = (root / "storage.log").read_bytes()
    offset, result = 0, []
    while offset < len(raw):
        length = struct.unpack_from("<Q", raw, offset + 16)[0]
        result.append(json.loads(raw[offset + 56 : offset + 56 + length]))
        offset += 96 + length
    return result


def reservation(root):
    blocked = False
    writer = make(root, 64 * 1024 * 1024)

    def obstruct(phase):
        nonlocal blocked
        if phase == "after_storage_catalog_sync" and not blocked:
            blocked = True
            (root / "raw").write_bytes(b"private mkdir obstruction")

    writer.backend.fault = obstruct
    try:
        try:
            writer.write_records([Record("unwritten", b"x" * 2048)], submission_id="failed")
        except StorageUnavailable:
            pass
        else:
            raise AssertionError("Expected mkdir failure")
        assert blocked
    finally:
        (root / "raw").unlink(missing_ok=True)
        writer.backend.fault = lambda _: None
        writer.close()
    (relative,) = {name for event in events(root) for name in event.get("packs", {})}
    return root / relative


def reset():
    for key in ("TARGET", "MODE", "LOG", "OPEN_ARM", "EFFECT_ARM"):
        os.environ.pop("STRAW_PARENT_" + key, None)


def collect(store):
    start = time.monotonic()
    try:
        result = store.collect_garbage()
    except StorageUnavailable as error:
        result = {"error": str(error)}
    return {"ms": (time.monotonic() - start) * 1000, "result": result}


def no_revisit(phase):
    assert phase not in {"before_gc_stat", "before_gc_unlink", "before_gc_directory_sync", "before_gc_receipt"}


def case(root, size, missing, expected, mode=None, logs=None):
    reset()
    root.mkdir(parents=True, exist_ok=False)
    orphan = reservation(root) if missing else None
    if orphan:
        orphan.parents[missing].mkdir(parents=True, exist_ok=True)
    with make(root) as writer:
        kept = writer.write_records([Record("kept", b"keep")], submission_id="kept")
        garbage = writer.write_records([Record("garbage", b"x" * size)], submission_id="garbage")
        writer.release_publications(garbage)
        parent = orphan.parent if orphan else (root / garbage[0].segment.path).parent
        missing_count, cursor = 0, parent
        while not cursor.exists():
            missing_count += 1
            cursor = cursor.parent
        phases, attempts, trace = [], [], []
        with make(root) as collector:
            collector.backend.fault = phases.append
            if mode:
                logs.mkdir(parents=True, exist_ok=False)
                open_arm, effect_arm, log = logs / "open.arm", logs / "effect.arm", logs / "trace.tsv"
                os.environ.update(
                    STRAW_PARENT_TARGET=str(parent),
                    STRAW_PARENT_MODE=mode,
                    STRAW_PARENT_LOG=str(log),
                    STRAW_PARENT_OPEN_ARM=str(open_arm),
                    STRAW_PARENT_EFFECT_ARM=str(effect_arm),
                )
                if mode.startswith(("open_", "sync_")):
                    open_arm.touch()
                if mode.startswith(("sync_", "mkdir_")):
                    effect_arm.touch()
            attempts.append(collect(collector))
            if mode:
                assert not open_arm.exists() and not effect_arm.exists()
                trace = [line.split("\t") for line in log.read_text().splitlines()]
                expected_trace = [["open", "-2"]] if mode.startswith(("open_", "sync_")) else []
                if mode.startswith(("sync_", "mkdir_")):
                    expected_trace.append(
                        ["fsync" if mode.startswith("sync_") else "mkdir", "0" if mode.endswith("after") else "-2"]
                    )
                assert trace == expected_trace, trace
                assert ("error" in attempts[0]["result"]) == (mode not in ("open_missing", "mkdir_after"))
                if "error" in attempts[0]["result"]:
                    assert not any("reclaimed" in e for e in events(root))
                    attempts.append(collect(collector))
                assert "error" not in attempts[-1]["result"]
                if mode == "open_error":
                    assert "before_gc_parent_repair" not in phases
            elif missing and expected == "baseline":
                attempts.extend(collect(collector) for _ in range(2))
                assert all("error" in attempt["result"] for attempt in attempts)
                assert not any("reclaimed" in e for e in events(root))
            else:
                assert "error" not in attempts[0]["result"]
            assert writer.read_record(kept[0]).payload == b"keep"
            assert not (root / garbage[0].segment.path).exists()
            manual_repair = bool(not mode and missing and expected == "baseline")
            if manual_repair:
                parent.mkdir(parents=True, exist_ok=True)
                assert "error" not in collect(collector)["result"]
            reset()
        with make(root) as restarted:
            restarted.backend.fault = no_revisit
            assert restarted.collect_garbage()["reclaimed_files"] == 0
        writer.release_publications(kept)
        assert writer.collect_garbage()["reclaimed_files"] == 1
        assert not [p for p in root.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
    return {
        "size": size,
        "requested_missing": missing,
        "missing_at_gc": missing_count,
        "mode": mode,
        "attempts": attempts,
        "trace": trace,
        "manual_directory_repair": manual_repair,
        "repair_calls": phases.count("before_gc_parent_repair"),
        "repair_directory_barriers": phases.count("before_directory_sync"),
        "actual_data_files_removed_by_gc_calls": 2,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--traces", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--inject", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.samples <= 1000:
        parser.error("samples must be 1..1000")
    args.root.mkdir(parents=True, exist_ok=False)
    cases = []
    if args.inject:
        assert args.expected == "fixed"
        for mode in ("open_missing", "open_error", "sync_before", "sync_after", "mkdir_before", "mkdir_after"):
            cases.append(
                case(args.root / mode, 2048, int(mode.startswith("mkdir_")), args.expected, mode, args.traces / mode)
            )
    else:
        for sample in range(args.samples):
            for size in (2048, 8 * 1024 * 1024):
                for missing in (0, 1, 2):
                    cases.append(case(args.root / f"{sample}-{size}-{missing}", size, missing, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "cases": cases,
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
