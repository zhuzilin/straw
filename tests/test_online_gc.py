"""Storage ownership survives queue handoff, GC races and interrupted unlink."""

import dataclasses
from concurrent.futures import ThreadPoolExecutor

import pytest
import torch

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import InvalidReference, StorageUnavailable, UnsafeRecovery
from straw.inspect import inspect_run
from straw.tensor import TensorRef, publish_tensors


def store(root, **kwargs):
    return SharedFilesystemStore(root, "pool", online_gc=True, codecs=("bytes.v1", "json.v1", "tensor.v1"), **kwargs)


def queue(s, name, **kwargs):
    return Coordinator(s, queue_id=name, namespace=True, exclusive_owner="one owner per queue", **kwargs)


def test_two_queues_share_tensor_until_both_release_and_checkpoint_is_dropped(tmp_path):
    writer = store(tmp_path)
    original = publish_tensors(writer, {"routes": torch.arange(24).reshape(6, 4)}, submission_id="tensor")[0]
    a = queue(store(tmp_path), "rollout-to-rollout")
    b = queue(store(tmp_path), "rollout-to-train")
    dependency, ordinal = original.share(writer, submission_id="share")
    a.submit_tasks("a", [TaskSpec("a", input_ref=dependency)])
    b.submit_tasks("b", [TaskSpec("b", input_ref=dependency)])
    writer.seal()
    writer.retain("checkpoint:1", [dependency])
    a.cancel_task("a", request_id="finished-a")
    assert a.collect_garbage()["reclaimed_files"] == 0
    torch.testing.assert_close(TensorRef.from_record_set(writer, dependency, ordinal).load(), original.load())
    b.cancel_task("b", request_id="finished-b")
    assert b.collect_garbage()["reclaimed_files"] == 0
    writer.release("checkpoint:1")
    assert writer.collect_garbage()["reclaimed_files"] == 1
    assert not list(tmp_path.rglob("*.pack"))
    assert a.journal.path != b.journal.path
    with pytest.raises(InvalidReference):
        writer.retain("late-reader", [dependency])
    a.close()
    b.close()


def test_copy_on_write_preserves_source_and_reuses_unchanged_tensor(tmp_path):
    s = store(tmp_path)
    source = publish_tensors(s, {"routes": torch.arange(16).reshape(4, 4)}, submission_id="source")[0]
    updated = source.updated(s, torch.tensor([[90, 91, 92, 93]]), rows=slice(1, 2), submission_id="update")
    assert updated.record_ref != source.record_ref
    torch.testing.assert_close(source.load(), torch.arange(16).reshape(4, 4))
    assert updated.load()[1].tolist() == [90, 91, 92, 93]
    before = s.metrics["payload_bytes"]
    ref, ordinal = source.share(s, submission_id="unchanged")
    assert ref == source.blob_set and ordinal == source.blob_ordinal
    assert s.metrics["payload_bytes"] == before
    assert len(list(tmp_path.rglob("*.pack"))) == 1


def test_staging_and_open_pack_prevent_collection(tmp_path):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="write")
    assert s.collect_garbage()["reclaimed_files"] == 0
    s.seal()
    assert s.collect_garbage()["reclaimed_files"] == 0
    s.release_publications([ref])
    assert s.collect_garbage()["reclaimed_files"] == 1


def test_reader_pin_and_mixed_pack_hold_whole_file(tmp_path):
    s = store(tmp_path)
    refs = [s.publish([Record(str(i), b"data")], submission_id=str(i)) for i in range(2)]
    s.seal()
    other = store(tmp_path)
    with s.pin([refs[0]]):
        s.release_publications(refs)
        with ThreadPoolExecutor() as pool:
            assert pool.submit(other.collect_garbage).result()["reclaimed_files"] == 0
        assert next(s.read(refs[0])).payload == b"data"
    assert other.collect_garbage()["reclaimed_files"] == 1


def test_gc_tombstone_is_durable_before_unlink_and_recovery_finishes_delete(tmp_path):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="write")
    s.seal()
    s.release_publications([ref])

    def crash(phase):
        if phase == "after_gc_tombstone":
            raise RuntimeError("GC process lost")

    s.backend.fault = crash
    with pytest.raises(RuntimeError, match="process lost"):
        s.collect_garbage()
    assert list(tmp_path.rglob("*.pack"))
    reader = store(tmp_path)
    with pytest.raises(InvalidReference, match="reclaimed"):
        reader.retain("late-owner", [ref])
    assert reader.collect_garbage()["reclaimed_files"] == 1
    assert reader.collect_garbage()["reclaimed_files"] == 0


def test_incomplete_destination_publication_keeps_dependency(tmp_path):
    source = store(tmp_path)
    ref = source.publish([Record("r", b"data")], submission_id="source")
    source.seal()
    target = store(tmp_path)

    def crash(phase):
        if phase == "before_footer":
            raise RuntimeError("writer lost")

    target.backend.fault = crash
    with pytest.raises(RuntimeError):
        target.publish([Record("new", b"new")], submission_id="target", dependencies=[ref])
    source.release_publications([ref])
    assert source.collect_garbage()["reclaimed_files"] == 0
    assert next(source.read(ref)).payload == b"data"


@pytest.mark.parametrize("replacement", ["absent", "directory"])
def test_tombstone_unlink_handles_stale_attributes_but_preserves_other_errors(tmp_path, replacement):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"garbage")], submission_id="write")
    s.seal()
    s.release_publications([ref])
    path = tmp_path / ref.manifest.segment.path

    def stale_stat(phase):
        if phase == "before_gc_unlink":
            # Model a successful remote unlink after the local cached stat.
            path.unlink()
            if replacement == "directory":
                path.mkdir()

    s.backend.fault = stale_stat
    if replacement == "directory":
        with pytest.raises(StorageUnavailable):
            s.collect_garbage()
        path.rmdir()
    else:
        result = s.collect_garbage()
        assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
        assert result["tombstones"] == 1
    s.backend.fault = lambda phase: None
    assert s.collect_garbage()["reclaimed_files"] == 0
    with pytest.raises(InvalidReference, match="reclaimed"):
        s.retain("late-reader", [ref])


def test_queue_recovery_preserves_other_queue_roots(tmp_path):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="source")
    a = queue(store(tmp_path), "a")
    a.submit_tasks("task", [TaskSpec("t", input_ref=ref)])
    s.seal()
    a.close()
    recovered = queue(store(tmp_path), "a", recover=True)
    assert recovered.collect_garbage()["reclaimed_files"] == 0
    assert next(s.read(ref)).payload == b"data"
    recovered.cancel_task("t", request_id="done")
    assert recovered.collect_garbage()["reclaimed_files"] == 1
    recovered.close()


def test_recovered_gc_uses_durable_owners_without_reopening_live_payloads(tmp_path):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="source")
    q = queue(s, "a")
    q.submit_tasks("task", [TaskSpec("t", input_ref=ref)])
    s.seal()
    q.close()
    recovered = queue(store(tmp_path), "a", recover=True)
    path = tmp_path / ref.manifest.segment.path
    unavailable = path.with_suffix(".temporarily-unavailable")
    path.rename(unavailable)
    try:
        # Collection is an ownership operation, not a scrub of live data. This
        # also forces cold descriptor caches after coordinator recovery.
        assert recovered.collect_garbage()["reclaimed_files"] == 0
    finally:
        unavailable.rename(path)
        recovered.close()
    assert next(s.read(ref)).payload == b"data"


@pytest.mark.parametrize("damage", ["missing", "empty"])
def test_gc_fails_before_unlink_when_a_live_root_loses_its_catalog_owner(tmp_path, damage):
    from straw.protocol import digest

    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="source")
    q = queue(s, "a")
    q.submit_tasks("task", [TaskSpec("t", input_ref=ref)])
    s.seal()
    owner = f"queue:{digest(str(q.journal.path.parent.parent))}:{digest(dataclasses.asdict(ref))}"
    # Deliberately violate the internal owner namespace to model inconsistent
    # catalog/WAL recovery. Normal applications must not mutate queue owners.
    if damage == "missing":
        s.release(owner)
    else:
        s.retain(owner, [])
    with pytest.raises(UnsafeRecovery, match="Live queue root"):
        q.collect_garbage()
    assert next(s.read(ref)).payload == b"data"
    assert list(tmp_path.rglob("*.pack"))
    q.close()


def test_offline_inspector_cannot_delete_shared_queue_storage(tmp_path):
    s = store(tmp_path)
    s.publish([Record("r", b"data")], submission_id="source")
    with pytest.raises(UnsafeRecovery, match="catalog"):
        inspect_run(tmp_path)


def test_discarded_tensor_publications_release_staging_but_keep_other_owners(tmp_path):
    import torch

    from straw.tensor import publish_tensors, release_tensor_publications

    s = store(tmp_path)
    discarded = publish_tensors(s, {"routes": torch.arange(8)}, submission_id="discarded")[0]
    s.seal()
    shared = publish_tensors(s, {"a": torch.arange(4), "b": torch.arange(3)}, submission_id="shared")
    dependency, _ = shared[0].share(s, submission_id="shared-owner")
    s.retain("other-queue", [dependency])
    s.seal()
    release_tensor_publications(s, [discarded, *shared])
    assert s.collect_garbage()["reclaimed_files"] == 1
    assert shared[1].load().tolist() == [0, 1, 2]
    s.release("other-queue")
    assert s.collect_garbage()["reclaimed_files"] == 1


def test_shared_descriptor_cannot_publish_a_reclaimed_dependency(tmp_path):
    from straw.tensor import release_tensor_publications

    s = store(tmp_path)
    tensor = publish_tensors(s, {"routes": torch.arange(4)}, submission_id="tensor")[0]
    s.seal()
    release_tensor_publications(s, [tensor])
    assert s.collect_garbage()["reclaimed_files"] == 1
    dependency, _ = tensor.share(s, submission_id="descriptor-only")
    with pytest.raises(InvalidReference, match="reclaimed"):
        s.publish([Record("destination", b"metadata")], submission_id="late-publication", dependencies=[dependency])


@pytest.mark.parametrize("ending", ["cancel", "expire", "recover"])
def test_revoking_authorization_does_not_reclaim_a_reader_still_using_input(tmp_path, ending):
    s = store(tmp_path)
    ref = s.publish([Record("input", b"still reading")], submission_id="input")
    now = [0.0]
    q = queue(s, "queue", clock=lambda: now[0], lease_seconds=1)
    q.submit_tasks("input", [TaskSpec("task", input_ref=ref, max_attempts=1)])
    lease = q.acquire("reader").assignments[0].lease
    s.seal()
    if ending == "cancel":
        q.cancel_task("task", request_id="cancel")
    elif ending == "expire":
        now[0] = 2.0
        q.acquire("other")
    else:
        q.close()
        q = queue(s, "queue", clock=lambda: now[0], lease_seconds=1, recover=True)
    assert q.collect_garbage()["reclaimed_files"] == 0
    assert next(s.read(ref)).payload == b"still reading"
    assert q.outstanding_reads() == (lease,)
    q.release_task_reads([lease])
    assert q.outstanding_reads() == ()
    assert q.collect_garbage()["reclaimed_files"] == 1
    q.close()
