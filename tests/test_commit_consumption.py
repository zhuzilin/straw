"""Durable queue acceptance must finish its exact staging handoff on retry."""

import errno

import pytest
from test_catalog_encoding import events, write_events

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import CorruptData, IndeterminateCommit, StorageUnavailable

PACKED = 64 * 1024 * 1024


def store(root, target=0):
    return SharedFilesystemStore(root, "commit-consumption", online_gc=True, segment_target_bytes=target)


def data_files(root):
    return [p for p in root.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]


@pytest.mark.parametrize(
    "cut", ["before_journal_write", "after_journal_part", "before_journal_sync", "after_journal_sync"]
)
def test_staging_remains_until_queue_wal_is_confirmed(tmp_path, cut):
    with store(tmp_path) as s, Coordinator(s, exclusive_owner="test") as q:
        ref = s.publish([Record("input", b"protected input")], submission_id="input")
        s.seal()

        def fail(at):
            if at == cut:
                raise OSError(errno.EIO, "queue confirmation interrupted")

        s.backend.fault = fail
        with pytest.raises(IndeterminateCommit):
            q.submit_tasks("submit", [TaskSpec("t", input_ref=ref)])
        s.backend.fault = lambda _: None
        # Even a complete uncertain queue frame cannot authorize an early
        # staging receipt. Recovery, rather than this failed call, owns cleanup.
        receipts = [
            receipt for event in events(tmp_path / "storage.log") for receipt in event.get("consumed_handoffs", [])
        ]
        assert receipts == []
        assert next(s.read(ref)).payload == b"protected input"


@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize("reopen", [False, True])
@pytest.mark.parametrize(
    "cut,occurrence",
    [
        ("before_staging_consume", 1),
        ("before_staging_manifest_read", 1),
        ("after_storage_catalog_part", 1),
        ("after_storage_catalog_part", 3),
        ("after_storage_catalog_sync", 1),
    ],
)
def test_retry_of_committed_request_finishes_cleanup(tmp_path, target, reopen, cut, occurrence):
    s = store(tmp_path, target)
    q = Coordinator(s, exclusive_owner="test")
    child = s.publish([Record("child", b"dependency")], submission_id="child")
    ref = s.publish([Record("input", b"x" * 2048)], submission_id="input", dependencies=[child])
    s.seal()
    tasks = [TaskSpec("t", input_ref=ref)]
    armed = False
    seen = 0

    def fail(at):
        nonlocal armed, seen
        if at == "before_staging_consume":
            armed = True
        if armed and at == cut:
            seen += 1
            if seen == occurrence:
                raise OSError(errno.EIO, "committed cleanup interrupted")

    s.backend.fault = fail
    with pytest.raises(StorageUnavailable, match="committed cleanup interrupted"):
        q.submit_tasks("submit", tasks)
    s.backend.fault = lambda _: None
    if reopen:
        q.close()
        s.close()
        s = store(tmp_path, target)
        q = Coordinator(s, exclusive_owner="old owner stopped", recover=True)
    try:
        assert q.submit_tasks("submit", tasks) == ["t"]
        assert next(s.read(child)).payload == b"dependency"
        q.cancel_task("t", request_id="cancel")
        gc = q.collect_garbage()
        assert gc["owners"] == gc["open_packs"] == gc["retained_packs"] == 0
        assert not data_files(tmp_path)
        receipts = [r for e in events(tmp_path / "storage.log") for r in e.get("consumed_handoffs", [])]
        assert len(receipts) == len(set(receipts)) == 1
    finally:
        q.close()
        s.close()


@pytest.mark.parametrize("phase", ["before_commit_apply", "after_commit_apply", "after_journal_sync"])
def test_recovery_finds_handoff_even_if_live_state_never_applied(tmp_path, phase):
    with store(tmp_path) as s:
        q = Coordinator(s, exclusive_owner="test")
        ref = s.publish([Record("input", b"input")], submission_id="input")
        s.seal()

        def fail(at):
            if at == phase:
                raise OSError(errno.EIO, "acceptance response lost")

        s.backend.fault = fail
        error = IndeterminateCommit if phase == "after_journal_sync" else StorageUnavailable
        with pytest.raises(error):
            q.submit_tasks("submit", [TaskSpec("t", input_ref=ref)])
        s.backend.fault = lambda _: None
        q.close()
    with store(tmp_path) as s, Coordinator(s, exclusive_owner="old owner stopped", recover=True) as q:
        assert q.submit_tasks("submit", [TaskSpec("t", input_ref=ref)]) == ["t"]
        q.cancel_task("t", request_id="cancel")
        assert q.collect_garbage()["owners"] == 0
        assert not data_files(tmp_path)


def test_cleanup_failure_fences_cached_result_cancel_and_gc(tmp_path):
    with store(tmp_path) as s, Coordinator(s, exclusive_owner="test") as q:
        ref = s.publish([Record("input", b"input")], submission_id="input")
        s.seal()
        tasks = [TaskSpec("t", input_ref=ref)]

        def fail(at):
            if at in {"before_staging_consume", "before_committed_staging_cleanup"}:
                raise OSError(errno.EIO, "cleanup unavailable")

        s.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            q.submit_tasks("submit", tasks)
        before = q.journal.path.read_bytes()
        for call in [
            lambda: q.submit_tasks("submit", tasks),
            lambda: q.cancel_task("t", request_id="cancel"),
            q.collect_garbage,
        ]:
            with pytest.raises(StorageUnavailable, match="cleanup unavailable"):
                call()
            assert q.journal.path.read_bytes() == before
            assert q.tasks["t"]["state"] == "pending"
        s.backend.fault = lambda _: None
        q.cancel_task("t", request_id="cancel")
        assert q.collect_garbage()["owners"] == 0


@pytest.mark.parametrize("lost_response", [False, True])
@pytest.mark.parametrize("reopen", [False, True])
def test_receipt_does_not_consume_staging_created_after_handoff(tmp_path, lost_response, reopen):
    with store(tmp_path) as writer, store(tmp_path) as s:
        q = Coordinator(s, exclusive_owner="test")
        ref = writer.publish([Record("input", b"input")], submission_id="input")
        tasks = [TaskSpec("t", input_ref=ref)]
        armed = False

        def fail(at):
            nonlocal armed
            if at == "before_staging_consume":
                armed = True
            if armed and at == "after_storage_catalog_sync":
                raise OSError(errno.EIO, "receipt response lost")

        if lost_response:
            s.backend.fault = fail
            with pytest.raises(StorageUnavailable, match="receipt response lost"):
                q.submit_tasks("submit", tasks)
            s.backend.fault = lambda _: None
        else:
            q.submit_tasks("submit", tasks)
        # The original writer's acknowledged cache recreates the same staging.
        assert writer.publish([Record("input", b"input")], submission_id="input") == ref
        writer.seal()
        if reopen:
            q.close()
            q = Coordinator(s, exclusive_owner="old owner stopped", recover=True)
        try:
            assert q.submit_tasks("submit", tasks) == ["t"]
            q.cancel_task("t", request_id="cancel")
            gc = q.collect_garbage()
            assert gc["owners"] == 1
            assert gc["reclaimed_files"] == 0
            assert next(s.read(ref)).payload == b"input"
            s.release_publications([ref])
            assert q.collect_garbage()["owners"] == 0
            assert not data_files(tmp_path)
        finally:
            q.close()


@pytest.mark.parametrize("count", [1, 32])
def test_completed_history_recovery_never_rereads_collected_manifests(tmp_path, count):
    with store(tmp_path) as s, Coordinator(s, exclusive_owner="test") as q:
        for i in range(count):
            ref = s.publish([Record(str(i), b"data")], submission_id=str(i))
            q.submit_tasks(str(i), [TaskSpec(str(i), input_ref=ref)])
            q.cancel_task(str(i), request_id="cancel-" + str(i))
        s.seal()
        assert q.collect_garbage()["owners"] == 0
        assert not data_files(tmp_path)
    before = (tmp_path / "storage.log").read_bytes()
    with store(tmp_path) as s:
        phases = []
        s.backend.fault = phases.append
        with Coordinator(s, exclusive_owner="old owner stopped", recover=True):
            pass
        assert "before_staging_manifest_read" not in phases
        assert "before_storage_catalog_write" not in phases
    assert (tmp_path / "storage.log").read_bytes() == before


@pytest.mark.parametrize(
    "receipt,owners",
    [
        (None, {}),
        ("a" * 64, {}),
        (["short"], {}),
        (["A" * 64], {}),
        ([42], {}),
        (["a" * 64], None),
        (["a" * 64], {"new-owner": []}),
    ],
)
def test_invalid_receipt_cannot_suppress_cleanup(tmp_path, receipt, owners):
    with store(tmp_path) as s:
        s.collect_garbage()
    log = tmp_path / "storage.log"
    values = events(log)
    values.append({"run_id": "commit-consumption", "owners": owners, "consumed_handoffs": receipt})
    write_events(log, values)
    before = log.read_bytes()
    with pytest.raises(CorruptData, match="handoff receipts"):
        with store(tmp_path) as s:
            s.collect_garbage()
    assert log.read_bytes() == before
