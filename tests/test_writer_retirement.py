"""Explicit incarnation retirement, process pauses and durable recovery."""

import errno
import multiprocessing

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import StorageUnavailable, WriterBusy, WriterFenced


def make(root, target=1024 * 1024):
    return SharedFilesystemStore(root, "retirement", online_gc=True, segment_target_bytes=target)


def paused_writer(root, connection, point):
    store = make(root, 0)
    connection.send(store.writer_id)

    def pause(phase):
        if phase == point:
            connection.send("paused")
            connection.recv()

    store.backend.fault = pause
    store.write_records([Record("record", b"unfinished")], submission_id="unfinished")
    connection.send("completed")


@pytest.mark.parametrize("point", ["after_publication_open", "after_payload_chunk"])
def test_paused_process_cannot_be_retired_until_its_gate_is_released(tmp_path, point):
    context = multiprocessing.get_context("spawn")
    parent, child = context.Pipe()
    process = context.Process(target=paused_writer, args=(str(tmp_path), child, point))
    process.start()
    try:
        assert parent.poll(20)
        writer_id = parent.recv()
        assert parent.poll(20)
        assert parent.recv() == "paused"
        peer = make(tmp_path)
        with pytest.raises(WriterBusy):
            peer.retire_writer(writer_id)
        process.kill()
        process.join(20)
        assert not process.is_alive()
        assert peer.retire_writer(writer_id) == []
        assert not list(tmp_path.rglob("*.partial"))
        assert peer.collect_garbage()["open_packs"] == 0
    finally:
        if process.is_alive():
            process.kill()
        process.join(20)
        parent.close()
        child.close()


def test_idle_writer_is_fenced_and_its_acknowledged_roots_survive(tmp_path):
    writer = make(tmp_path)
    ref = writer.publish([Record("record", b"committed")], submission_id="committed")
    peer = make(tmp_path)
    assert peer.retire_writer(writer.writer_id) == []
    with pytest.raises(WriterFenced):
        writer.publish([Record("record", b"resumed")], submission_id="resumed")
    assert peer.read_record(peer.validate(ref)[0]).payload == b"committed"
    assert peer.collect_garbage()["reclaimed_files"] == 0
    peer.release_publications([ref])
    assert peer.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("policy", ["recover", "abandon"])
@pytest.mark.parametrize("lost_reply", [None, "retirement", "staging"])
def test_supervisor_discards_dead_writer_staging_without_releasing_other_owners(tmp_path, policy, lost_reply):
    writer, peer = make(tmp_path, 0), make(tmp_path, 0)
    other = peer.publish([Record("other", b"other")], submission_id="other")
    accepted = writer.publish([Record("accepted", b"accepted")], submission_id="accepted")
    abandoned = writer.publish([Record("abandoned", b"abandoned")], submission_id="abandoned", dependencies=(other,))
    peer.retain("trainer-recovery:accepted", [accepted])
    if lost_reply:
        retired = False

        def fail(at):
            nonlocal retired
            if at == "after_writer_retirement_receipt":
                retired = True
                if lost_reply == "retirement":
                    raise OSError(errno.EIO, "lost retirement reply")
            if retired and at == "after_storage_catalog_sync" and lost_reply == "staging":
                raise OSError(errno.EIO, "lost staging release reply")

        peer.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            peer.retire_writer(writer.writer_id, policy=policy, discard_staging=True)
        peer.backend.fault = lambda _: None
    peer.retire_writer(writer.writer_id, policy=policy, discard_staging=True)
    peer.retire_writer(writer.writer_id, policy=policy, discard_staging=True)
    assert peer.collect_garbage()["reclaimed_files"] == 1
    assert not (tmp_path / abandoned.manifest.segment.path).exists()
    assert peer.read_record(peer.validate(accepted)[0]).payload == b"accepted"
    assert peer.read_record(peer.validate(other)[0]).payload == b"other"
    summary = peer.retention_summary()
    assert summary["categories"]["staged"]["owners"] == 1
    assert summary["categories"]["trainer-recovery"]["owners"] == 1
    peer.release("trainer-recovery:accepted")
    assert peer.collect_garbage()["reclaimed_files"] == 1


def test_registered_empty_writer_can_be_retired(tmp_path):
    writer, peer = make(tmp_path), make(tmp_path)
    writer.seal()
    assert peer.retire_writer(writer.writer_id, policy="abandon", discard_staging=True) == []


@pytest.mark.parametrize(
    "phase",
    [
        "after_writer_fence",
        "after_writer_drain",
        "after_writer_operation_reconcile",
        "after_writer_reconcile",
        "after_writer_seal",
        "after_writer_retirement_receipt",
    ],
)
def test_complete_uncertain_publication_and_retirement_reply_loss(tmp_path, phase):
    writer = make(tmp_path)
    seen = 0

    def lose_stage(at):
        nonlocal seen
        if at == "before_storage_catalog_sync":
            seen += 1
            if seen == 2:
                raise OSError(errno.EIO, "lost staging reply")

    writer.backend.fault = lose_stage
    with pytest.raises(StorageUnavailable):
        writer.write_records([Record("record", b"recoverable")], submission_id="recoverable")
    peer = make(tmp_path)
    fired = False

    def lose_retirement(at):
        nonlocal fired
        if at == phase and not fired:
            fired = True
            raise OSError(errno.EIO, "lost retirement reply")

    peer.backend.fault = lose_retirement
    with pytest.raises(StorageUnavailable):
        peer.retire_writer(writer.writer_id)
    assert fired
    peer.backend.fault = lambda _: None
    refs = peer.retire_writer(writer.writer_id)
    assert len(refs) == 1
    assert peer.read_record(refs[0]).payload == b"recoverable"
    assert peer.retire_writer(writer.writer_id) == refs


@pytest.mark.parametrize("api", ["records", "record_set", "publish"])
def test_durable_done_receipt_is_queryable_after_reply_loss_and_retirement(tmp_path, api):
    writer = make(tmp_path)
    source = writer.write_records([Record("source", b"source")], submission_id="source")
    writer_id = writer.writer_id

    def lost_reply(phase):
        if phase == "after_publication_receipt":
            raise OSError(errno.EIO, "durable Done, reply lost")

    writer.backend.fault = lost_reply
    with pytest.raises(StorageUnavailable):
        if api == "records":
            writer.write_records([Record("result", b"result")], submission_id="lost")
        elif api == "record_set":
            writer.record_set(source, submission_id="lost")
        else:
            writer.publish([Record("result", b"result")], submission_id="lost")
    peer = make(tmp_path)
    query = lambda: peer.publication_receipt(writer_id, api=api, submission_id="lost")
    refs = query()
    assert refs is not None and len(refs) == 1
    assert peer.publication_receipt(writer_id, api=api, submission_id="absent") is None
    assert peer.retire_writer(writer_id) == []
    assert query() == refs
    peer.release_publications([*source, *refs])
    assert peer.collect_garbage()["owners"] == 0
    assert query() == refs
    assert peer.collect_garbage()["owners"] == 0


def test_receipts_bind_publication_api_even_when_request_ids_are_equal(tmp_path):
    writer = make(tmp_path)
    records = writer.write_records([Record("source", b"source")], submission_id="same")
    record_set = writer.record_set(records, submission_id="same")
    published = writer.publish([Record("new", b"new")], submission_id="same")
    peer = make(tmp_path)
    for api, expected in [("records", list(records)), ("record_set", [record_set]), ("publish", [published])]:
        assert peer.publication_receipt(writer.writer_id, api=api, submission_id="same") == expected
