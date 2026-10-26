"""Offline deletion must not bypass shared ownership on advisory stat errors."""

import errno
import os
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore
from straw.coordinator import Coordinator
from straw.errors import UnsafeRecovery
from straw.inspect import cleanup_orphans, inspect_run


def stopped_pool(root, managed=False):
    store = SharedFilesystemStore(root, "offline", online_gc=managed)
    queue = Coordinator(store, exclusive_owner="test owns the only coordinator")
    ref = store.publish([Record("data", b"checkpoint payload")], submission_id="data")
    if managed:
        store.retain("checkpoint:external", [ref])
        store.release_publications([ref])
    queue.close()
    store.close()
    return ref


@pytest.mark.parametrize("operation", ["inspect", "cleanup"])
def test_false_stat_absence_cannot_delete_catalog_owned_data(tmp_path, monkeypatch, operation):
    ref = stopped_pool(tmp_path, managed=True)
    original = Path.exists
    stat_calls = []

    def stale(path):
        if path == tmp_path / "storage.log":
            stat_calls.append(path)
            return False
        return original(path)

    with monkeypatch.context() as context:
        context.setattr(Path, "exists", stale)
        with pytest.raises(UnsafeRecovery, match="catalog"):
            if operation == "inspect":
                inspect_run(tmp_path)
            else:
                # Even a previously supplied unsafe report must pass a new guard.
                cleanup_orphans(tmp_path, {"deletion_allowed": True}, all_participants_stopped=True)
        assert not stat_calls
    with SharedFilesystemStore(tmp_path, "offline", online_gc=True) as observer:
        assert next(observer.read(ref)).payload == b"checkpoint payload"
        protected = observer.collect_garbage()
        assert protected["owners"] == 1 and protected["reclaimed_files"] == 0
        observer.release("checkpoint:external")
        assert observer.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("operation", ["inspect", "cleanup"])
def test_false_stat_absence_cannot_hide_queue_namespace(tmp_path, monkeypatch, operation):
    ref = stopped_pool(tmp_path)
    (tmp_path / "queues").mkdir()
    original = Path.exists

    def stale(path):
        return False if path == tmp_path / "queues" else original(path)

    with monkeypatch.context() as context:
        context.setattr(Path, "exists", stale)
        with pytest.raises(UnsafeRecovery, match="catalog"):
            if operation == "inspect":
                inspect_run(tmp_path)
            else:
                cleanup_orphans(tmp_path, {"deletion_allowed": True}, all_participants_stopped=True)
    with SharedFilesystemStore(tmp_path, "offline") as observer:
        assert next(observer.read(ref)).payload == b"checkpoint payload"


@pytest.mark.parametrize("marker", ["storage.log", "queues"])
@pytest.mark.parametrize("error_number", [errno.EIO, errno.EACCES, errno.ESTALE, errno.ENOTDIR])
@pytest.mark.parametrize("operation", ["inspect", "cleanup"])
def test_unknown_storage_layout_refuses_inspection_and_cleanup(tmp_path, monkeypatch, marker, error_number, operation):
    ref = stopped_pool(tmp_path)
    original = os.open

    def unavailable(path, flags, *args, **kwargs):
        if Path(path) == tmp_path / marker:
            raise OSError(error_number, os.strerror(error_number), str(path))
        return original(path, flags, *args, **kwargs)

    with monkeypatch.context() as context:
        context.setattr(os, "open", unavailable)
        with pytest.raises(UnsafeRecovery, match="Cannot establish standalone") as caught:
            if operation == "inspect":
                inspect_run(tmp_path)
            else:
                cleanup_orphans(tmp_path, {"deletion_allowed": True}, all_participants_stopped=True)
        assert isinstance(caught.value.__cause__, OSError)
        assert caught.value.__cause__.errno == error_number
    with SharedFilesystemStore(tmp_path, "offline") as observer:
        assert next(observer.read(ref)).payload == b"checkpoint payload"


@pytest.mark.parametrize("marker", ["storage.log", "queues"])
def test_cleanup_rechecks_shared_marker_after_earlier_report(tmp_path, marker):
    ref = stopped_pool(tmp_path)
    report = inspect_run(tmp_path)
    assert report["deletion_allowed"] and ref.manifest.segment.path in report["unaccepted_sealed"]
    if marker == "queues":
        (tmp_path / marker).mkdir()
    else:
        (tmp_path / marker).touch()
    with pytest.raises(UnsafeRecovery, match="catalog"):
        cleanup_orphans(tmp_path, report, all_participants_stopped=True)
    assert (tmp_path / ref.manifest.segment.path).is_file()
