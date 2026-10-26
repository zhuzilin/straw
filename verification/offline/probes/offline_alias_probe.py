"""Stopped private-pool alias cleanup comparison; no filesystem fault injection."""

import argparse
import hashlib
import json
import shutil
import tempfile
import time
from dataclasses import replace
from pathlib import Path

from straw import Record, SharedFilesystemStore, TaskSpec, _native
from straw.coordinator import Coordinator
from straw.errors import QueueError
from straw.inspect import cleanup_orphans, inspect_run
from straw.protocol import RecordRef

MODES = ("manifest_file", "partial_file", "partial_directory", "sealed_directory", "backtrack", "record", "dependency")


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def fixture(root, mode, size, canonical):
    payload = hashlib.shake_256(str(size).encode()).digest(size)
    with SharedFilesystemStore(
        root, "offline-alias", segment_target_bytes=0 if mode == "manifest_file" else 1
    ) as store:
        queue = Coordinator(store, exclusive_owner="private stopped-run alias probe")
        ref = store.publish([Record("kept", payload)], submission_id="kept")
        physical = root / ref.manifest.segment.path
        directory = root / "raw/aliases/routes"
        directory.mkdir(parents=True)
        route = directory / ("route.sealed" if mode == "sealed_directory" else "route.partial")
        alias_path = "alias" + physical.suffix
        if mode in ("manifest_file", "record", "dependency"):
            (root / alias_path).symlink_to(ref.manifest.segment.path)
        elif mode == "partial_file":
            route.symlink_to(physical)
            (root / alias_path).symlink_to(route.relative_to(root))
        else:
            target = physical.parent
            if mode == "backtrack":
                target = target / "unused"
                target.mkdir()
            route.symlink_to(target, target_is_directory=True)
            if mode == "backtrack":
                (root / alias_path).symlink_to(str(route.relative_to(root)) + "/../" + physical.name)
            else:
                alias_path = (route / physical.name).relative_to(root).as_posix()
        alias = replace(ref, manifest=replace(ref.manifest, segment=replace(ref.manifest.segment, path=alias_path)))
        assert next(store.read(alias)).payload == payload
        accepted = alias
        if mode == "record":
            record = RecordRef.from_dict(store.manifest(ref)["records"][0])
            accepted = store.record_set(
                [replace(record, segment=replace(record.segment, path=alias_path))], submission_id="record"
            )
        elif mode == "dependency":
            accepted = store.publish([Record("child", b"child")], submission_id="child", dependencies=[alias])
        roots = [TaskSpec("kept", input_ref=accepted)]
        if canonical:
            roots.append(TaskSpec("canonical", input_ref=ref))
        queue.submit_tasks("accepted", roots)
        orphan = store.publish([Record("orphan", b"independent orphan")], submission_id="orphan")
        journal = queue.journal.path
        queue.close()
    # Independent orphan alias points at a real file outside raw enumeration.
    # Cleanup must unlink this lexical name, never its target.
    (root / "unreferenced.bin").write_bytes(b"unreferenced target")
    (directory / "orphan.partial").symlink_to(root / "unreferenced.bin")
    (directory / "dangling.partial").symlink_to(root / "missing.bin")
    return ref, alias, payload, orphan, journal


def case(root, mode, size, canonical, expected):
    ref, alias, payload, orphan, journal = fixture(root, mode, size, canonical)
    physical = root / ref.manifest.segment.path
    before = digest(journal)
    file_bytes = physical.stat().st_size
    start = time.perf_counter_ns()
    report = inspect_run(root)
    assert report["deletion_allowed"]
    removed = cleanup_orphans(root, report, all_participants_stopped=True)
    wall = time.perf_counter_ns() - start
    assert orphan.manifest.segment.path in removed
    assert (root / "unreferenced.bin").read_bytes() == b"unreferenced target"
    assert "raw/aliases/routes/orphan.partial" in removed
    assert "raw/aliases/routes/dangling.partial" in removed
    error = None
    with SharedFilesystemStore(root, "offline-alias") as observer:
        try:
            assert next(observer.read(alias)).payload == payload
        except QueueError as caught:
            error = type(caught).__name__
    # Canonical references prevent physical loss in the old implementation, but
    # cannot preserve route aliases. Dependency/record cases without routes do
    # stay readable when a separate canonical reference exists.
    broken = expected == "baseline" and (
        not canonical or mode in ("partial_file", "partial_directory", "sealed_directory", "backtrack")
    )
    assert bool(error) == broken, (mode, canonical, error, removed)
    exists = physical.exists()
    assert exists == (expected == "fixed" or canonical)
    assert digest(journal) == before
    row = dict(
        mode=mode,
        payload_bytes=size,
        canonical_root=canonical,
        retained=report["retained_paths"],
        removed=removed,
        read_error=error,
        physical_exists=exists,
        file_bytes=file_bytes,
        journal_preserved=True,
        wall_ns=wall,
        intended_orphan_files=1,
        unsafe_deleted_bytes=0 if exists else file_bytes,
    )
    shutil.rmtree(root)
    row["private_removed"] = not root.exists()
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="straw-offline-alias-", dir=args.parent) as private:
        rows = [
            case(Path(private) / str(i), mode, size, canonical, args.expected)
            for i, (mode, size, canonical) in enumerate(
                (m, s, c) for m in MODES for s in (2048, 8 * 1024**2) for c in (False, True)
            )
        ]
    import straw.inspect

    result = dict(
        passed=True,
        expected=args.expected,
        cases=rows,
        private=private,
        private_removed=not Path(private).exists(),
        native_sha256=digest(_native.__file__),
        inspector_sha256=digest(straw.inspect.__file__),
        script_sha256=digest(__file__),
        scope="No fault injection; stable aliases precede WAL acceptance. Offline orphan cleanup and manual private fixture teardown are not catalog GC.",
    )
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": True, "cases": len(rows), "broken_reads": sum(bool(r["read_error"]) for r in rows)}))


if __name__ == "__main__":
    main()
