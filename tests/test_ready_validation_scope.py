"""One ready transition shares immutable proofs; readers and later calls do not."""

import json
import pickle
from concurrent.futures import ThreadPoolExecutor

import pytest

from straw import Publication, Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import CorruptData, InvalidReference
from straw.protocol import encode


def queue(store, name="q"):
    return Coordinator(store, namespace=True, queue_id=name, exclusive_owner="one")


def prepared(store, q):
    leaves = store.publish_many([Publication((Record(str(i), b"leaf"),)) for i in range(64)], submission_id="leaves")
    q.submit_tasks("batch", [TaskSpec("batch", control=True, estimated_records=0)])
    lease = q.acquire("manager", control=True).assignments[0].lease
    root = store.publish(
        [Record("converted", b"converted", metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id})],
        submission_id="converted",
        dependencies=leaves,
    )
    return lease, root, leaves[0]


def complete(q, lease, root):
    return q.complete_task(
        lease, submission_id="result:" + lease.attempt_id, result_ref=root, result_digest=root.digest
    )


def flip_index(store, ref):
    segment = ref.manifest.segment
    with (store.backend.root / segment.path).open("r+b") as stream:
        stream.seek(segment.offset + segment.size - 80)
        size = int.from_bytes(stream.read(8), "little")
        offset = segment.offset + segment.size - 80 - size
        stream.seek(offset)
        value = stream.read(1)[0]
        stream.seek(offset)
        stream.write(bytes([value ^ 1]))


def test_ready_checks_shared_dependencies_once_and_new_roots_explicitly(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True) as store:
        q = queue(store)
        try:
            lease, root, leaf = prepared(store, q)
            token = q.open_consumer("training", exclusive_owner="one")
            with store.validation_scope(q) as scope:
                complete(q, lease, root)
                first = json.loads(scope.stats())
                assert first["nodes"] == first["checked_nodes"] == 65
                state, progress = store.publish_many(
                    [
                        Publication((Record("state", b"{}", "json.v1"),), (root,)),
                        Publication(
                            (
                                Record(
                                    "progress",
                                    encode({"version": 1, "processed_positions": [], "finished_batches": []}),
                                    "json.v1",
                                ),
                            ),
                            (0,),
                        ),
                    ],
                    submission_id="state-progress",
                )
                q.save_consumer_state(
                    "training",
                    request_id="ready",
                    token=token,
                    state_ref=state,
                    progress_ref=progress,
                    fetch_cursor=1,
                    processed_cursor=0,
                )
                final = json.loads(scope.stats())
                assert final["nodes"] == 67 and final["published_nodes"] == 2
                # The converted root and two newly published roots still have
                # their own metadata checked. The 64 dependencies do not repeat.
                assert final["checked_nodes"] == first["checked_nodes"] + 3
                assert final["reused_nodes"] == 66
                with pytest.raises(TypeError):
                    pickle.dumps(scope)
            with pytest.raises(RuntimeError, match="closed"):
                scope.stats()
            flip_index(store, leaf)
            with pytest.raises(CorruptData, match="index checksum"):
                q.submit_tasks("later", [TaskSpec("later", input_ref=root)])
        finally:
            q.close()


def test_scoped_root_authorization_failure_closes_scope_immediately(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True) as store:
        q = queue(store)
        try:
            lease, root, _ = prepared(store, q)
            with store.validation_scope(q) as scope:
                complete(q, lease, root)
                q.submit_tasks("other", [TaskSpec("other", control=True, estimated_records=0)])
                other = q.acquire("manager", control=True).assignments[0].lease
                with pytest.raises(InvalidReference, match="task/attempt"):
                    complete(q, other, root)
                with pytest.raises(RuntimeError, match="closed"):
                    scope.stats()
                with pytest.raises(RuntimeError, match="closed"):
                    store.publish([Record("bad", b"bad")], submission_id="after-error")
            assert store.validate(root)
        finally:
            q.close()


def test_explicit_validate_never_uses_scope_proofs(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True) as store:
        q = queue(store)
        try:
            lease, root, leaf = prepared(store, q)
            with store.validation_scope(q) as scope:
                complete(q, lease, root)
                flip_index(store, leaf)
                with pytest.raises(CorruptData, match="index checksum"):
                    store.validate(root)
                with pytest.raises(RuntimeError, match="closed"):
                    scope.stats()
        finally:
            q.close()


def test_failed_publication_discards_the_whole_scope(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True) as store:
        q = queue(store)
        try:
            lease, root, _ = prepared(store, q)
            with store.validation_scope(q) as scope:
                complete(q, lease, root)

                def crash(phase):
                    if phase == "before_footer":
                        raise RuntimeError("write lost")

                store.backend.fault = crash
                with pytest.raises(RuntimeError, match="write lost"):
                    store.publish([Record("state", b"state")], submission_id="state", dependencies=(root,))
                store.backend.fault = lambda _: None
                with pytest.raises(RuntimeError, match="closed"):
                    scope.stats()
            assert store.validate(root)
        finally:
            store.backend.fault = lambda _: None
            q.close()


def test_scope_cannot_resurrect_a_tombstoned_publication(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True, segment_target_bytes=0) as store:
        q = queue(store)
        try:
            with store.validation_scope(q) as scope:
                root = store.publish([Record("root", b"root")], submission_id="root")
                store.release_publications([root])
                store.collect_garbage()
                with pytest.raises(InvalidReference, match="reclaimed"):
                    store.publish([Record("late", b"late")], submission_id="late", dependencies=(root,))
                with pytest.raises(RuntimeError, match="closed"):
                    scope.stats()
        finally:
            q.close()


def test_native_scope_rejects_a_different_store_configuration(tmp_path):
    with (
        SharedFilesystemStore(tmp_path / "a", "scope", online_gc=True) as store,
        SharedFilesystemStore(tmp_path / "b", "scope", online_gc=True) as other,
    ):
        q = queue(store)
        foreign = queue(other)
        try:
            with store.validation_scope(q) as scope:
                with pytest.raises(InvalidReference, match="different storage"):
                    foreign._native.call("metrics", "{}", foreign.clock(), other._fault, scope)
                with pytest.raises(RuntimeError, match="closed"):
                    scope.stats()
        finally:
            q.close()
            foreign.close()


def test_unrelated_producer_thread_does_not_inherit_scope(tmp_path):
    with SharedFilesystemStore(tmp_path, "scope", online_gc=True) as store:
        q = queue(store)
        try:
            with store.validation_scope(q) as scope:
                store.publish([Record("main", b"main")], submission_id="main")
                before = json.loads(scope.stats())

                def produce():
                    assert store._active_validation_scope() is None
                    root = store.publish([Record("producer", b"producer")], submission_id="producer")
                    q.submit_tasks("producer", [TaskSpec("producer", input_ref=root)])

                with ThreadPoolExecutor(max_workers=1) as executor:
                    executor.submit(produce).result()
                assert json.loads(scope.stats()) == before
        finally:
            q.close()
