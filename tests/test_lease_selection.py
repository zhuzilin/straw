"""A filtered acquisition issues at most one current lease per task."""

import pytest

from straw import SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator


@pytest.mark.parametrize("limit", [1, 2, 4])
def test_repeated_task_filter_cannot_consume_multiple_assignment_slots(tmp_path, limit):
    with SharedFilesystemStore(tmp_path, "selection") as store:
        queue = Coordinator(store, exclusive_owner="test")
        try:
            queue.submit_tasks("tasks", [TaskSpec("a", priority=2), TaskSpec("b", priority=1), TaskSpec("c")])
            acquired = queue.acquire("worker", limit, task_ids=["b", "a", "b", "missing", "a", "c", "a"])
            assignments = acquired.assignments
            ids = [a.task.task_id for a in assignments]
            assert ids == ["a", "b", "c"][:limit]
            assert queue.heartbeat([a.lease for a in assignments]) == ["extended"] * len(assignments)
        finally:
            queue.close()


@pytest.mark.parametrize("recover", [False, True])
def test_selection_dedup_preserves_control_prefix_priority_and_replay(tmp_path, recover):
    with SharedFilesystemStore(tmp_path, "selection") as store:
        queue = Coordinator(store, exclusive_owner="test")
        try:
            queue.submit_tasks(
                "tasks",
                [
                    TaskSpec("job:a", priority=1, scheduling_key=3),
                    TaskSpec("job:b", priority=1, scheduling_key=-2),
                    TaskSpec("other", priority=100),
                    TaskSpec("job:control", control=True, estimated_records=0),
                ],
            )
            if recover:
                queue.close()
                queue = Coordinator(store, exclusive_owner="old owner closed", recover=True)
            ids = ["job:a", "job:control", "other", "job:b", "job:a", "job:b", "job:control"]
            acquired = queue.acquire("worker", 3, task_ids=ids, task_prefix="job:")
            assert [a.task.task_id for a in acquired.assignments] == ["job:b", "job:a"]
            control = queue.acquire("control", 3, task_ids=ids, task_prefix="job:", control=True)
            assert [a.task.task_id for a in control.assignments] == ["job:control"]
            assert queue.heartbeat([a.lease for a in acquired.assignments + control.assignments]) == ["extended"] * 3
            assert [t.task_id for t in queue.pending_tasks()] == ["other"]
        finally:
            queue.close()
