"""Offline roots include recovery metadata and arbitrary-depth dependencies."""

import errno
import hashlib
from dataclasses import asdict

import pytest

import straw.inspect as inspection
from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import UnsafeRecovery
from straw.protocol import RecordSetRef, digest, encode


@pytest.mark.parametrize("metadata", ["run.json", "control/journal.log"])
@pytest.mark.parametrize("suffix", [".partial", ".pack", ".sealed"])
@pytest.mark.parametrize("route", ["file", "backtrack"])
def test_cleanup_preserves_metadata_files_and_resolution_bindings(tmp_path, metadata, suffix, route):
    root = tmp_path / "pool"
    with SharedFilesystemStore(root, "metadata") as store:
        queue = Coordinator(store, exclusive_owner="test owns the coordinator")
        ref = store.publish([Record("kept", b"payload")], submission_id="kept")
        queue.submit_tasks("accepted", [TaskSpec("kept", input_ref=ref)])
        queue.close()
    # Move while stopped, then recover and accept new work before inspection.
    # Bindings stay stable throughout this recovered client's lifetime/cleanup.
    directory = root / "raw/metadata/routes"
    directory.mkdir(parents=True)
    candidate = directory / ("metadata" + suffix)
    if route == "file":
        (root / metadata).rename(candidate)
        (root / metadata).symlink_to(candidate)
    else:
        actual = root / "metadata"
        (actual / "detour").mkdir(parents=True)
        (root / metadata).rename(actual / "contents")
        candidate.symlink_to(actual / "detour", target_is_directory=True)
        (root / metadata).symlink_to(str(candidate) + "/../contents")
    with SharedFilesystemStore(root, "metadata") as store:
        queue = Coordinator(store, recover=True, exclusive_owner="test owns the coordinator")
        queue.submit_tasks("again", [TaskSpec("second", input_ref=ref)])
        orphan = store.publish([Record("orphan", b"orphan")], submission_id="orphan")
        queue.close()
    before = {p: (root / p).read_bytes() for p in ["run.json", "control/journal.log"]}
    report = inspection.inspect_run(root)
    assert report["deletion_allowed"]
    assert (root / metadata).resolve().relative_to(root).as_posix() in report["retained_paths"]
    assert inspection.cleanup_orphans(root, report, all_participants_stopped=True) == [orphan.manifest.segment.path]
    assert all((root / p).read_bytes() == content for p, content in before.items())
    with SharedFilesystemStore(root, "metadata") as store:
        queue = Coordinator(store, recover=True, exclusive_owner="test owns the coordinator")
        assert set(queue.tasks) == {"kept", "second"}
        assert next(store.read(ref)).payload == b"payload"
        queue.close()


@pytest.mark.parametrize("metadata", ["run.json", "control/journal.log"])
def test_unknown_metadata_binding_forbids_deletion(tmp_path, monkeypatch, metadata):
    with SharedFilesystemStore(tmp_path, "metadata") as store:
        queue = Coordinator(store, exclusive_owner="test owns the coordinator")
        orphan = store.publish([Record("orphan", b"orphan")], submission_id="orphan")
        queue.close()
    original = inspection._reference_bindings

    def unavailable(root, relative):
        if relative == metadata:
            raise OSError(errno.EIO, "unknown metadata binding")
        return original(root, relative)

    monkeypatch.setattr(inspection, "_reference_bindings", unavailable)
    report = inspection.inspect_run(tmp_path)
    assert not report["deletion_allowed"] and report["retained_dependency_errors"][0]["root"] == metadata
    with pytest.raises(UnsafeRecovery):
        inspection.cleanup_orphans(tmp_path, report, all_participants_stopped=True)
    assert (tmp_path / orphan.manifest.segment.path).is_file()


def test_deep_accepted_closure_inspection_uses_heap_frontier(tmp_path, monkeypatch):
    with SharedFilesystemStore(tmp_path, "deep") as store:
        queue = Coordinator(store, exclusive_owner="test owns the coordinator")
        leaf = store.publish([Record("leaf", b"x" * 1024)], submission_id="leaf")
        store.seal()
        previous, records = asdict(leaf), []
        for i in range(1100):
            logical = digest({"records": [], "dependencies": [previous["digest"]]})
            manifest = {"version": 2, "records": [], "dependencies": [previous], "digest": logical}
            records.append(Record(f"node-{i}", encode(manifest), "record-set.v2"))
            previous = {"local": i, "digest": logical, "records": 0, "payload_bytes": 1024, "tokens": 0}
        raw = store.write_records(records, submission_id="chain")
        store.seal()
        previous.pop("local")
        previous["manifest"] = asdict(raw[-1])
        ref = RecordSetRef.from_dict(previous)
        queue.submit_tasks("deep", [TaskSpec("deep", input_ref=ref)])
        orphan = store.publish([Record("orphan", b"orphan")], submission_id="orphan")
        queue.close()
    wal = tmp_path / "control/journal.log"
    before = hashlib.sha256(wal.read_bytes()).digest()
    original, calls = SharedFilesystemStore.validate, []

    def checked(self, reference, *args, **kwargs):
        calls.append(reference)
        return original(self, reference, *args, **kwargs)

    monkeypatch.setattr(SharedFilesystemStore, "validate", checked)
    report = inspection.inspect_run(tmp_path)
    assert report["deletion_allowed"] and calls == [ref]
    assert inspection.cleanup_orphans(tmp_path, report, all_participants_stopped=True) == [
        orphan.manifest.segment.path
    ]
    assert hashlib.sha256(wal.read_bytes()).digest() == before
    with SharedFilesystemStore(tmp_path, "deep") as store:
        assert store.validate(ref) == ()
        assert next(store.read(leaf)).payload == b"x" * 1024
