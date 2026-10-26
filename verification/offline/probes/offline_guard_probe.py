"""Compare installed offline-cleanup guards on private stopped storage pools.

Exact-path syscall faults require offline_guard_fault.c via LD_PRELOAD. The
baseline can delete checkpoint-owned or other-queue-accepted data in these
private fixtures; that intentional loss is reported separately from normal GC.
Legacy namespaced fixtures have no catalog and use explicit fixture teardown.
"""

import argparse
import hashlib
import json
import os
import shutil
import tempfile
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, TaskSpec, _native
from straw.coordinator import Coordinator
from straw.errors import StorageUnavailable, UnsafeRecovery
from straw.inspect import cleanup_orphans, inspect_run


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def reset():
    for suffix in ("PATH", "ARM", "FAULT"):
        os.environ.pop("STRAW_OFFLINE_" + suffix, None)


def fixture(root, layout, size):
    managed = layout == "catalog"
    store = SharedFilesystemStore(root, "offline-probe", online_gc=managed)
    root_queue = Coordinator(store, exclusive_owner="private stopped-run probe root")
    payload = hashlib.shake_256(str(size).encode()).digest(size)
    ref = store.publish([Record("kept", payload)], submission_id="kept")
    other_journal = None
    if managed:
        store.retain("checkpoint:external", [ref])
        store.release_publications([ref])
    else:
        other = Coordinator(store, queue_id="other", namespace=True, exclusive_owner="private stopped-run probe peer")
        other.submit_tasks("accepted", [TaskSpec("kept", input_ref=ref)])
        other_journal = other.journal.path
        other.close()
    root_queue.close()
    store.close()
    return ref, payload, other_journal


def case(root, traces, layout, size, fault, expected):
    reset()
    ref, payload, other_journal = fixture(root, layout, size)
    marker = root / ("storage.log" if layout == "catalog" else "queues")
    data_path = root / ref.manifest.segment.path
    original_size = data_path.stat().st_size
    journal_before = digest(other_journal) if other_journal else None
    try:
        inspect_run(root)
    except UnsafeRecovery:
        pass
    else:
        raise AssertionError("Normal inspection must refuse this shared layout")
    traces.mkdir(parents=True, exist_ok=False)
    arm = traces / "arm"
    fired = traces / "arm.fired"
    os.environ.update(STRAW_OFFLINE_PATH=str(marker), STRAW_OFFLINE_ARM=str(arm), STRAW_OFFLINE_FAULT=fault)
    observations = []

    def invoke(operation):
        arm.touch(exist_ok=False)
        start = time.perf_counter_ns()
        error = None
        value = None
        try:
            value = operation()
        except (UnsafeRecovery, OSError) as caught:
            error = type(caught).__name__
        elapsed = time.perf_counter_ns() - start
        injected = fired.exists()
        observations.append({"error": error, "injected": injected, "wall_ns": elapsed})
        arm.unlink(missing_ok=True)
        fired.unlink(missing_ok=True)
        return value, error

    report, error = invoke(lambda: inspect_run(root))
    removed = []
    if error is None:
        assert expected == "baseline" and fault == "stat-missing"
        assert report["deletion_allowed"] and ref.manifest.segment.path in report["unaccepted_sealed"]
        removed, error = invoke(lambda: cleanup_orphans(root, report, all_participants_stopped=True))
        assert error is None and ref.manifest.segment.path in removed
    elif expected == "fixed":
        assert error == "UnsafeRecovery"
        # A caller can supply an older unsafe report; cleanup must recheck anyway.
        _, error = invoke(lambda: cleanup_orphans(root, {"deletion_allowed": True}, all_participants_stopped=True))
        assert error == "UnsafeRecovery"
    reset()
    should_lose = expected == "baseline" and fault == "stat-missing"
    assert bool(removed) == should_lose
    expected_injection = fault.startswith("stat-") if expected == "baseline" else fault.startswith("open-")
    assert all(o["injected"] == expected_injection for o in observations)
    assert data_path.exists() != should_lose
    with SharedFilesystemStore(root, "offline-probe", online_gc=layout == "catalog") as observer:
        if should_lose:
            try:
                next(observer.read(ref))
            except StorageUnavailable:
                pass
            else:
                raise AssertionError("Deleted data unexpectedly readable")
        else:
            assert next(observer.read(ref)).payload == payload
        if layout == "catalog":
            protected = observer.collect_garbage()
            assert protected["owners"] == 1 and protected["reclaimed_files"] == 0
            observer.release("checkpoint:external")
            final_gc = observer.collect_garbage()
            assert final_gc["owners"] == final_gc["open_packs"] == final_gc["retained_packs"] == 0
            assert final_gc["reclaimed_files"] == (0 if should_lose else 1)
            assert not data_path.exists()
        else:
            protected = final_gc = None
            assert digest(other_journal) == journal_before  # Accepted peer WAL survives even when data does not.
    remaining_data = [p for p in root.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]
    legacy_teardown = {"files": len(remaining_data), "bytes": sum(p.stat().st_size for p in remaining_data)}
    assert not remaining_data if layout == "catalog" or should_lose else len(remaining_data) == 1
    shutil.rmtree(root)
    shutil.rmtree(traces)
    return {
        "layout": layout,
        "payload_bytes": size,
        "file_bytes": original_size,
        "fault": fault,
        "observations": observations,
        "unsafe_deletion": should_lose,
        "removed": removed,
        "owner_or_peer_journal_preserved": True,
        "protected_gc": protected,
        "final_gc": final_gc,
        "legacy_fixture_teardown": legacy_teardown,
        "private_removed": not root.exists() and not traces.exists(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    private = Path(tempfile.mkdtemp(prefix="straw-offline-guard-", dir=args.parent.resolve()))
    trace_root = Path(tempfile.mkdtemp(prefix="straw-offline-guard-traces-"))
    import straw.inspect as inspector

    report = {
        "passed": False,
        "private": str(private),
        "trace_root": str(trace_root),
        "expected": args.expected,
        "script_sha256": digest(__file__),
        "shim_sha256": digest(os.environ["LD_PRELOAD"]),
        "native_sha256": digest(_native.__file__),
        "inspector_sha256": digest(inspector.__file__),
        "cases": [],
    }
    try:
        for layout in ("catalog", "queues"):
            for size in (2048, 8 * 1024**2):
                for fault in ("stat-missing", "stat-error", "open-error", "open-denied"):
                    name = f"{layout}-{size}-{fault}"
                    report["cases"].append(case(private / name, trace_root / name, layout, size, fault, args.expected))
        private.rmdir()
        trace_root.rmdir()
        report.update(passed=True, private_removed=not private.exists(), traces_removed=not trace_root.exists())
    finally:
        reset()
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                "passed": True,
                "cases": len(report["cases"]),
                "unsafe_deletions": sum(c["unsafe_deletion"] for c in report["cases"]),
            }
        )
    )


if __name__ == "__main__":
    main()
