"""An external process supervisor can own liveness without weakening fencing."""

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import LeaseExpired, StaleAttempt, UnsafeRecovery


@pytest.mark.parametrize("seconds", [300, None])
def test_supervised_lease_survives_slow_storage_but_timed_lease_expires(tmp_path, seconds):
    clock = [0.0]
    with SharedFilesystemStore(tmp_path, "leases") as store:
        with Coordinator(
            store, exclusive_owner="test supervisor", lease_seconds=seconds, clock=lambda: clock[0]
        ) as queue:
            queue.submit_tasks("input", [TaskSpec("task")])
            lease = queue.acquire("worker").assignments[0].lease
            clock[0] = 86400.0
            ref = store.publish(
                [
                    Record(
                        "result",
                        b"ok",
                        metadata={
                            "task_id": lease.task_id,
                            "attempt_id": lease.attempt_id,
                        },
                    )
                ],
                submission_id="output",
            )
            if seconds is None:
                assert queue.acquire("other").status == "empty"
                assert (
                    queue.complete_task(
                        lease, submission_id="result", result_ref=ref, result_digest=ref.digest
                    ).task_id
                    == "task"
                )
            else:
                with pytest.raises(LeaseExpired):
                    queue.complete_task(lease, submission_id="result", result_ref=ref, result_digest=ref.digest)


def test_supervisor_retirement_and_epoch_still_fence_old_attempts(tmp_path):
    with SharedFilesystemStore(tmp_path, "leases") as store:
        queue = Coordinator(store, exclusive_owner="test supervisor", lease_seconds=None)
        queue.submit_tasks("input", [TaskSpec("task")])
        old = queue.acquire("worker").assignments[0].lease
        # Caller has confirmed the worker exited before invoking retirement.
        queue.retire_worker("worker")
        fresh = queue.acquire("replacement").assignments[0].lease
        assert fresh.attempt_id != old.attempt_id
        assert queue.heartbeat([old, fresh]) == ["StaleAttempt", "extended"]
        ref = store.publish(
            [
                Record(
                    "result",
                    b"old",
                    metadata={
                        "task_id": old.task_id,
                        "attempt_id": old.attempt_id,
                    },
                )
            ],
            submission_id="output",
        )
        with pytest.raises(StaleAttempt):
            queue.complete_task(old, submission_id="stale", result_ref=ref, result_digest=ref.digest)
        queue.close()
        with Coordinator(store, exclusive_owner="old owner stopped", lease_seconds=None, recover=True) as recovered:
            assert recovered.heartbeat([fresh]) == ["StaleAttempt"]
            assert recovered.acquire("new owner").assignments[0].lease.coordinator_epoch != fresh.coordinator_epoch


@pytest.mark.parametrize("original,recovered", [(None, 300), (300, None)])
def test_lease_mode_cannot_change_when_recovering_an_existing_queue(tmp_path, original, recovered):
    with SharedFilesystemStore(tmp_path, "leases") as store:
        Coordinator(store, exclusive_owner="test supervisor", lease_seconds=original).close()
        with pytest.raises(UnsafeRecovery, match="configuration mismatch"):
            Coordinator(store, exclusive_owner="test supervisor", lease_seconds=recovered, recover=True)
