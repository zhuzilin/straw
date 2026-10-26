"""Storage ownership survives queue handoff, GC races and interrupted unlink."""

import dataclasses
from concurrent.futures import ThreadPoolExecutor

import pytest
import torch

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import IdempotencyConflict, InvalidReference, StorageUnavailable, UnsafeRecovery
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


@pytest.mark.parametrize("api", ["write_records", "record_set", "publish"])
@pytest.mark.parametrize("target", [0, 1024 * 1024])
def test_publication_finalizes_standalone_files_but_keeps_packs_appendable(tmp_path, api, target):
    with store(tmp_path, segment_target_bytes=target) as s:
        if api == "record_set":
            raw = s.write_records([Record("raw", b"data")], submission_id="raw")
            refs = [s.record_set(raw, submission_id="set")]
        elif api == "write_records":
            refs = s.write_records([Record("raw", b"data")], submission_id="raw")
        else:
            refs = [s.publish([Record("raw", b"data")], submission_id="raw")]
        before = s.collect_garbage()
        assert before["reclaimed_files"] == 0
        assert before["open_packs"] == (0 if target == 0 else 1)
        s.seal()
        s.release_publications(refs)
        assert s.collect_garbage()["reclaimed_files"] == (2 if api == "record_set" and target == 0 else 1)
    with store(tmp_path) as fresh:
        assert fresh.collect_garbage()["open_packs"] == 0
        assert not list(tmp_path.rglob("*.sealed"))
        assert not list(tmp_path.rglob("*.pack"))


@pytest.mark.parametrize("api", ["write_records", "record_set", "publish"])
@pytest.mark.parametrize(
    "failed_phase",
    ["before_publication_seal", "after_publication_seal", "before_storage_catalog_sync", "after_storage_catalog_sync"],
)
def test_publication_seal_failure_retries_without_losing_staging(tmp_path, api, failed_phase):
    with store(tmp_path, segment_target_bytes=0) as s, store(tmp_path) as peer:
        raw = s.write_records([Record("raw", b"data")], submission_id="raw") if api == "record_set" else None

        def publish():
            if api == "record_set":
                return [s.record_set(raw, submission_id="retry")]
            if api == "write_records":
                return s.write_records([Record("r", b"data")], submission_id="retry")
            return [s.publish([Record("r", b"data")], submission_id="retry")]

        armed = False
        fired = False

        def fault(phase):
            nonlocal armed, fired
            if phase == "before_publication_seal":
                armed = True
                # Outside the catalog lock: GC sees staging before seal.
                assert peer.collect_garbage()["reclaimed_files"] == 0
            if armed and phase == failed_phase and not fired:
                fired = True
                raise OSError(5, "uncertain seal")

        s.backend.fault = fault
        with pytest.raises(StorageUnavailable, match="uncertain seal"):
            publish()
        assert fired
        paths = set(tmp_path.rglob("*.sealed"))
        assert peer.collect_garbage()["reclaimed_files"] == 0
        s.backend.fault = lambda phase: None
        refs = publish()
        assert set(tmp_path.rglob("*.sealed")) == paths
        assert peer.collect_garbage()["open_packs"] == 0
        record = s.read_record(refs[0]) if api == "write_records" else next(s.read(refs[0]))
        assert record.payload == b"data"
        s.release_publications(refs)
        assert peer.collect_garbage()["reclaimed_files"] == (2 if api == "record_set" else 1)


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


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("conflict", ["remove", "replace"])
def test_conflicting_retry_preserves_original_publication_dependencies(tmp_path, api, conflict):
    with store(tmp_path) as source, store(tmp_path) as target:

        def dependency(identity):
            if api == "publish":
                ref = source.publish([Record(identity, identity.encode())], submission_id=identity)
            else:
                (ref,) = source.write_records([Record(identity, identity.encode())], submission_id=identity)
            source.seal()
            return ref

        original, alternate = dependency("original"), dependency("alternate")

        def publish(refs):
            if api == "publish":
                return target.publish([Record("target", b"target")], submission_id="retry", dependencies=refs)
            return target.record_set(refs, submission_id="retry")

        def uncertain(phase):
            if phase == "after_data_sync":
                raise OSError(5, "uncertain publication")

        target.backend.fault = uncertain
        with pytest.raises(StorageUnavailable, match="uncertain publication"):
            publish([original])
        target.backend.fault = lambda phase: None
        source.release_publications([original])
        assert source.collect_garbage()["reclaimed_files"] == 0
        with pytest.raises(IdempotencyConflict):
            publish([] if conflict == "remove" else [alternate])
        assert source.collect_garbage()["reclaimed_files"] == 0
        result = publish([original])
        target.seal()
        source.release_publications([alternate])
        # The successful handoff releases additional roots reserved by rejected
        # retries, while staging keeps the original dependency and output live.
        assert source.collect_garbage()["reclaimed_files"] == 1
        assert list(target.read(result))
        original_record = next(source.read(original)) if api == "publish" else source.read_record(original)
        assert original_record.payload == b"original"
        target.release_publications([result])
        assert source.collect_garbage()["reclaimed_files"] == 2


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


@pytest.mark.parametrize("operation", ["release", "replace"])
def test_public_api_cannot_change_a_live_queue_owner(tmp_path, operation):
    from straw.protocol import digest

    s = store(tmp_path)
    ref = s.publish([Record("r", b"data")], submission_id="source")
    q = queue(s, "a")
    q.submit_tasks("task", [TaskSpec("t", input_ref=ref)])
    s.seal()
    owner = f"queue:{digest(str(q.journal.path.parent.parent))}:{digest(dataclasses.asdict(ref))}"
    with pytest.raises(ValueError, match="reserved"):
        if operation == "release":
            s.release(owner)
        else:
            s.retain(owner, [])
    assert q.collect_garbage()["reclaimed_files"] == 0
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


def test_missing_tombstone_still_retries_failed_directory_sync(tmp_path):
    s = store(tmp_path)
    ref = s.publish([Record("r", b"garbage")], submission_id="write")
    s.seal()
    s.release_publications([ref])
    path = tmp_path / ref.manifest.segment.path
    sync_attempts = []

    def failed_sync(phase):
        if phase == "before_gc_directory_sync":
            sync_attempts.append(phase)
            raise OSError(5, "injected directory sync failure")

    s.backend.fault = failed_sync
    # Both calls must attempt the durability barrier, even though the first
    # unlink has already made the name absent by the time the second GC starts.
    for _ in range(2):
        with pytest.raises(StorageUnavailable, match="directory sync failure"):
            s.collect_garbage()
        assert not path.exists()
    assert len(sync_attempts) == 2
    s.backend.fault = lambda phase: sync_attempts.append(phase) if phase == "before_gc_directory_sync" else None
    result = s.collect_garbage()
    assert len(sync_attempts) == 3
    assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
    with pytest.raises(InvalidReference, match="reclaimed"):
        s.retain("late-owner", [ref])


@pytest.mark.parametrize("failed_barrier", [None, 1, 2])
def test_gc_batches_directory_barriers_and_retries_absent_names(tmp_path, failed_barrier):
    paths = []
    for writer in range(2):
        with store(tmp_path) as s:
            refs = []
            for index in range(3):
                ref = s.publish([Record(str(index), b"garbage")], submission_id=f"{writer}:{index}")
                s.seal()
                refs.append(ref)
                paths.append(tmp_path / ref.manifest.segment.path)
            s.release_publications(refs)
    parents = {p.parent for p in paths}
    assert len(paths) == 6 and len(parents) == 2
    expected_bytes = sum(p.stat().st_size for p in paths)
    with store(tmp_path) as collector:
        barriers = []

        def barrier(phase):
            if phase == "before_gc_directory_sync":
                # Every member's unlink precedes the shared directory barrier.
                assert all(not p.exists() for p in paths)
                barriers.append(phase)
                if len(barriers) == failed_barrier:
                    raise OSError(5, "injected batch sync failure")

        collector.backend.fault = barrier
        if failed_barrier:
            with pytest.raises(StorageUnavailable, match="batch sync failure"):
                collector.collect_garbage()
            assert len(barriers) == failed_barrier
            barriers.clear()
            collector.backend.fault = lambda phase: (
                barriers.append(phase) if phase == "before_gc_directory_sync" else None
            )
            result = collector.collect_garbage()
            assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
        else:
            result = collector.collect_garbage()
            assert result["reclaimed_files"] == len(paths)
            assert result["reclaimed_bytes"] == expected_bytes
        assert len(barriers) == len(parents)
        barriers.clear()
        assert collector.collect_garbage()["reclaimed_files"] == 0
        # A durable completion receipt retires retry I/O, not the tombstones.
        assert not barriers


def test_gc_batch_retries_partial_unlinks_before_any_barrier(tmp_path):
    with store(tmp_path) as s:
        refs = []
        for index in range(4):
            refs.append(s.publish([Record(str(index), b"garbage")], submission_id=str(index)))
            s.seal()
        s.release_publications(refs)
        paths = [tmp_path / ref.manifest.segment.path for ref in refs]
        unlinks = []
        barriers = []

        def interrupt(phase):
            if phase == "before_gc_unlink":
                unlinks.append(phase)
                if len(unlinks) == 3:
                    raise OSError(5, "injected third unlink failure")
            if phase == "before_gc_directory_sync":
                barriers.append(phase)

        s.backend.fault = interrupt
        with pytest.raises(StorageUnavailable, match="third unlink failure"):
            s.collect_garbage()
        assert sum(not p.exists() for p in paths) == 2
        assert not barriers
        s.backend.fault = lambda phase: barriers.append(phase) if phase == "before_gc_directory_sync" else None
        assert s.collect_garbage()["reclaimed_files"] == 2
        assert all(not p.exists() for p in paths)
        assert len(barriers) == 1


def test_auto_discovery_cannot_downgrade_after_catalog_disappears(tmp_path):
    with store(tmp_path) as source, SharedFilesystemStore(tmp_path, "pool") as target:
        ref = source.publish([Record("r", b"keep")], submission_id="discovery")
        source.seal()
        target.retain("destination", [ref])
        log = tmp_path / "storage.log"
        saved = tmp_path / "storage.saved"
        log.rename(saved)
        try:
            with pytest.raises(StorageUnavailable):
                target.retain("second", [ref])
            assert not log.exists(), "Do not create a replacement catalog"
        finally:
            saved.rename(log)
        target.retain("second", [ref])
        source.release_publications([ref])
        target.release("destination")
        assert source.collect_garbage()["reclaimed_files"] == 0
        assert next(target.read(ref)).payload == b"keep"
        target.release("second")
        assert source.collect_garbage()["reclaimed_files"] == 1


def test_auto_discovery_lookup_error_does_not_acknowledge_retain(tmp_path):
    with store(tmp_path) as source, SharedFilesystemStore(tmp_path, "pool") as target:
        ref = source.publish([Record("r", b"keep")], submission_id="discovery")
        source.seal()
        log = tmp_path / "storage.log"
        saved = tmp_path / "storage.saved"
        log.rename(saved)
        log.symlink_to(log.name)  # ELOOP, not a legitimate absent catalog.
        try:
            with pytest.raises(StorageUnavailable):
                target.retain("destination", [ref])
            assert log.is_symlink()
        finally:
            log.unlink()
            saved.rename(log)
        target.retain("destination", [ref])
        source.release_publications([ref])
        assert source.collect_garbage()["reclaimed_files"] == 0
        assert next(target.read(ref)).payload == b"keep"
        target.release("destination")
        assert source.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("ending", ["cancel", "expire", "recover"])
@pytest.mark.parametrize("acknowledgement", ["release", "retire"])
def test_all_independent_progress_roots_survive_revocation_until_reader_ack(tmp_path, ending, acknowledgement):
    s = store(tmp_path, segment_target_bytes=0)
    refs = [
        s.publish([Record(name, name.encode())], submission_id=name)
        for name in ("initial", "middle", "latest", "peer")
    ]
    now = [0.0]
    q = queue(s, "queue", clock=lambda: now[0], lease_seconds=1)
    q.submit_tasks(
        "inputs",
        [TaskSpec("one", input_ref=refs[0], max_attempts=1), TaskSpec("two", input_ref=refs[3], max_attempts=1)],
    )
    first = q.acquire("first", task_ids=["one"]).assignments[0].lease
    peer = q.acquire("second", task_ids=["two"]).assignments[0].lease
    for request, ref in zip(("middle", "latest", "same-latest"), (refs[1], refs[2], refs[2])):
        q.save_task_progress(first, request_id=request, input_ref=ref)
    # These publications have no dependency edges: retaining only the latest
    # continuation cannot incidentally keep the earlier reads alive.
    readers = q._call("state", {})["readers"]
    assert len(readers[first.attempt_id]["roots"]) == 3
    s.seal()
    if ending == "cancel":
        q.cancel_task("one", request_id="cancel-one")
        q.cancel_task("two", request_id="cancel-two")
    elif ending == "expire":
        now[0] = 2.0
        assert not q.acquire("probe").assignments
    else:
        q.close()
        q = queue(s, "queue", clock=lambda: now[0], lease_seconds=1, recover=True)
    assert q.collect_garbage()["reclaimed_files"] == 0
    for ref, name in zip(refs, ("initial", "middle", "latest", "peer")):
        assert next(s.read(ref)).payload == name.encode()
    if acknowledgement == "release":
        q.release_task_reads([first, first])
    else:
        q.retire_worker("first")
    assert q.outstanding_reads() == (peer,)
    assert q.collect_garbage()["reclaimed_files"] == 3
    assert next(s.read(refs[3])).payload == b"peer"
    q.release_task_reads([peer])
    assert q.collect_garbage()["reclaimed_files"] == 1
    q.close()
    s.close()
