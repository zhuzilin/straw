"""Cross-log fault windows with a dependency and independent long-lived owners."""

import errno

import pytest
from test_catalog_encoding import events

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import IdempotencyConflict, IndeterminateCommit, InvalidReference, StorageUnavailable


def store(root):
    return SharedFilesystemStore(root, "durable-handoff", online_gc=True, segment_target_bytes=0)


def queue(s, name, *, recover=False):
    return Coordinator(s, queue_id=name, namespace=True, exclusive_owner="previous owner closed", recover=recover)


CATALOG_CUTS = [
    ("before_storage_catalog_write", 1),
    ("after_storage_catalog_part", 1),
    ("after_storage_catalog_part", 2),
    ("after_storage_catalog_part", 3),
    ("before_storage_catalog_sync", 1),
    ("after_storage_catalog_sync", 1),
]
CUTS = (
    [("retain", phase, n) for phase, n in CATALOG_CUTS]
    + [("accept", "before_journal_write", 1)]
    + [("accept", "after_journal_part", n) for n in (1, 2, 3)]
    + [
        ("accept", phase, 1)
        for phase in ("before_journal_sync", "after_journal_sync", "before_commit_apply", "after_commit_apply")
    ]
    + [("cleanup", phase, n) for phase, n in CATALOG_CUTS]
    + [("cleanup", "before_staging_consume", 1), ("cleanup", "after_committed_staging_cleanup", 1)]
)


@pytest.mark.parametrize("stage,phase,occurrence", CUTS)
def test_cross_log_recovery_preserves_closure_and_retry_after_gc(tmp_path, stage, phase, occurrence):
    with store(tmp_path) as writer, store(tmp_path) as source_store, queue(source_store, "source") as source:
        child = writer.publish([Record("child", b"dependency")], submission_id="child")
        root = writer.publish([Record("root", b"input")], submission_id="root", dependencies=[child])
        writer.seal()
        paths = {tmp_path / ref.manifest.segment.path for ref in (root, child)}
        assert len(paths) == 2
        source.submit_tasks("source", [TaskSpec("source", input_ref=root)])
        writer.retain("reader:independent", [root])
        writer.retain("checkpoint:independent", [root])
        tasks = [TaskSpec("destination", input_ref=root)]
        current_stage = "retain"
        seen = 0

        def fail(at):
            nonlocal current_stage, seen
            if at == "before_journal_write":
                current_stage = "accept"
            elif at == "before_staging_consume":
                current_stage = "cleanup"
            if current_stage == stage and at == phase:
                seen += 1
                if seen == occurrence:
                    raise OSError(errno.EIO, "cross-log interruption")

        with store(tmp_path) as destination_store, queue(destination_store, "destination") as destination:
            destination_store.backend.fault = fail
            with pytest.raises((IndeterminateCommit, StorageUnavailable)):
                destination.submit_tasks("transfer", tasks)
            assert seen == occurrence
            destination_store.backend.fault = lambda _: None
        # The source remains live until the destination's exact request is
        # recovered/retried successfully. Independent peers also refresh/GC.
        assert source.collect_garbage()["reclaimed_files"] == 0
        with (
            store(tmp_path) as destination_store,
            queue(destination_store, "destination", recover=True) as destination,
        ):
            assert destination.submit_tasks("transfer", tasks) == ["destination"]
            assert destination.submit_tasks("transfer", tasks) == ["destination"]
            source.cancel_task("source", request_id="source-done")
            assert source.collect_garbage()["reclaimed_files"] == 0
            assert next(writer.read(root)).payload == b"input"
            assert next(writer.read(child)).payload == b"dependency"
            destination.cancel_task("destination", request_id="destination-done")
            assert destination.collect_garbage()["reclaimed_files"] == 0
            writer.release("checkpoint:independent")
            assert writer.collect_garbage()["reclaimed_files"] == 0
            assert all(path.exists() for path in paths)
            writer.release("reader:independent")
            result = writer.collect_garbage()
            assert result["reclaimed_files"] == 2
            assert result["owners"] == 0
            assert not any(path.exists() for path in paths)
            with pytest.raises(InvalidReference):
                writer.retain("late-reader", [root])
        # Replaying the original request after physical reclamation must return
        # its recorded result without reopening manifests or recreating owners.
        with store(tmp_path) as final_store, queue(final_store, "destination", recover=True) as recovered:
            catalog_before = (tmp_path / "storage.log").read_bytes()
            effects = []
            final_store.backend.fault = effects.append
            assert recovered.submit_tasks("transfer", tasks) == ["destination"]
            with pytest.raises(IdempotencyConflict):
                recovered.submit_tasks("transfer", [TaskSpec("different", input_ref=root)])
            assert recovered.task_status("destination")["state"] == "cancelled"
            assert sum(event["type"] == "Submitted" for tx in events(recovered.journal.path) for event in tx) == 1
            assert "before_staging_manifest_read" not in effects
            assert "before_storage_catalog_write" not in effects
            assert (tmp_path / "storage.log").read_bytes() == catalog_before


def test_multiple_queue_requests_recover_without_consuming_restage(tmp_path):
    with store(tmp_path) as writer, store(tmp_path) as left_store, store(tmp_path) as right_store:
        child = writer.publish([Record("child", b"shared dependency")], submission_id="child")
        root = writer.publish([Record("root", b"shared input")], submission_id="root", dependencies=[child])
        left_tasks = [TaskSpec("left", input_ref=root)]
        right_tasks = [TaskSpec("right", input_ref=root)]
        later_tasks = [TaskSpec("later", input_ref=child)]
        with queue(left_store, "left") as left, queue(right_store, "right") as right:
            assert left.submit_tasks("same-id", left_tasks) == ["left"]
            consuming = False

            def lose_cleanup_reply(at):
                nonlocal consuming
                if at == "before_staging_consume":
                    consuming = True
                if consuming and at == "after_storage_catalog_sync":
                    raise OSError(errno.EIO, "durable receipt reply lost")

            right_store.backend.fault = lose_cleanup_reply
            with pytest.raises(StorageUnavailable, match="durable receipt reply lost"):
                right.submit_tasks("same-id", right_tasks)
            right_store.backend.fault = lambda _: None

        # A fresh staging lifetime begins after the uncertain but durable
        # cleanup. Neither queue's recovery or cached request may consume it.
        assert writer.publish([Record("root", b"shared input")], submission_id="root", dependencies=[child]) == root
        writer.seal()
        writer.retain("reader:composition", [root])
        with queue(left_store, "left", recover=True) as left, queue(right_store, "right", recover=True) as right:
            before = (tmp_path / "storage.log").read_bytes()
            assert left.submit_tasks("same-id", left_tasks) == ["left"]
            assert right.submit_tasks("same-id", right_tasks) == ["right"]
            with pytest.raises(IdempotencyConflict):
                right.submit_tasks("same-id", left_tasks)
            assert (tmp_path / "storage.log").read_bytes() == before
            assert left.submit_tasks("later-id", later_tasks) == ["later"]
            left.cancel_task("left", request_id="retire-left")
            left.cancel_task("later", request_id="retire-later")
            assert left.collect_garbage()["reclaimed_files"] == 0
            assert right.task_status("right")["state"] == "pending"
            right.cancel_task("right", request_id="retire-right")
            assert right.collect_garbage()["reclaimed_files"] == 0
            writer.release("reader:composition")
            assert writer.collect_garbage()["reclaimed_files"] == 0
            assert next(writer.read(root)).payload == b"shared input"
            assert next(writer.read(child)).payload == b"shared dependency"
            writer.release_publications([root])
            assert writer.collect_garbage()["reclaimed_files"] == 2

        for _ in range(2):
            with queue(left_store, "left", recover=True) as left, queue(right_store, "right", recover=True) as right:
                before = (tmp_path / "storage.log").read_bytes()
                assert left.submit_tasks("same-id", left_tasks) == ["left"]
                assert left.submit_tasks("later-id", later_tasks) == ["later"]
                assert right.submit_tasks("same-id", right_tasks) == ["right"]
                assert (tmp_path / "storage.log").read_bytes() == before
