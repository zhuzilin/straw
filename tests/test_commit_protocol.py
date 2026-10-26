"""Crash boundaries of the production retain/WAL/apply/consume controller."""

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import UnsafeRecovery


@pytest.mark.parametrize("phase", ["before_commit_apply", "after_commit_apply", "before_staging_consume"])
def test_durable_completion_replays_once_and_stays_owned(tmp_path, phase):
    store = SharedFilesystemStore(tmp_path, "commit", online_gc=True)
    queue = Coordinator(store, exclusive_owner="test")
    queue.submit_tasks("tasks", [TaskSpec("t")])
    lease = queue.acquire("worker").assignments[0].lease
    ref = store.publish(
        [Record("r", b"retained result", metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id})],
        submission_id="physical",
    )
    store.seal()

    def crash(at):
        if at == phase:
            raise RuntimeError("commit interrupted")

    store.backend.fault = crash
    with pytest.raises(RuntimeError, match="commit interrupted"):
        queue.complete_task(lease, submission_id="completion", result_ref=ref, result_digest=ref.digest)
    store.backend.fault = lambda _: None
    if phase != "before_staging_consume":
        # State application may be partial. The old owner must not continue
        # from an in-memory state whose WAL is already ahead of it.
        with pytest.raises(UnsafeRecovery):
            queue.acquire("other")
    queue.close()
    store.close()

    with SharedFilesystemStore(tmp_path, "commit", online_gc=True) as fresh:
        recovered = Coordinator(fresh, exclusive_owner="old owner stopped", recover=True)
        try:
            receipt = recovered.complete_task(
                lease, submission_id="completion", result_ref=ref, result_digest=ref.digest
            )
            assert recovered.read_commits().commits == (receipt,)
            assert recovered.collect_garbage()["reclaimed_files"] == 0
            assert next(fresh.read(ref)).payload == b"retained result"
        finally:
            recovered.close()
