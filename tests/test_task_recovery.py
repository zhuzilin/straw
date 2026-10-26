"""Intact WAL framing cannot authorize updates to tasks never submitted."""

import json

import pytest

from straw import SharedFilesystemStore, TaskSpec
from straw.backend import FilesystemBackend
from straw.coordinator import Coordinator
from straw.errors import UnsafeRecovery
from straw.journal import Journal


def unknown_task_events():
    lease = {
        "task_id": "missing",
        "attempt_id": "attempt",
        "generation": 1,
        "worker_id": "worker",
        "coordinator_epoch": "epoch",
        "run_id": "task-replay",
        "queue_id": "rollout",
        "token": "token",
    }
    return [
        {"type": "Epoch", "epoch": "new-epoch", "revoked": ["known", "missing"]},
        {"type": "Leased", "leases": [lease]},
        {"type": "Expired", "task_ids": ["known", "missing"]},
        {"type": "Failed", "task_ids": ["known", "missing"], "retryable": True},
        {"type": "Failed", "task_ids": ["known", "missing"], "retryable": False},
        {"type": "Cancelled", "task_id": "missing"},
        {"type": "Yielded", "task_id": "missing", "input_ref": None},
        {"type": "TaskProgress", "task_id": "missing", "input_ref": None},
        {
            "type": "Completed",
            "receipt": {"task_id": "missing", "submission_id": "submission", "result_ref": None},
            "lease": lease,
        },
    ]


@pytest.mark.parametrize("event", unknown_task_events(), ids=lambda e: f"{e['type']}-{e.get('retryable', '')}")
def test_replay_rejects_unknown_task_before_appending_recovery_epoch(tmp_path, event):
    with SharedFilesystemStore(tmp_path, "task-replay") as store:
        queue = Coordinator(store, exclusive_owner="test")
        queue.submit_tasks("known-task", [TaskSpec("known")])
        queue.close()

    # Malformed-business fixture, not a claim that the valid public API emits
    # these events. The real journal writer supplies correct frame checksums.
    identity = json.loads((tmp_path / "run.json").read_text())
    journal = Journal(FilesystemBackend(tmp_path), identity, recover=True)
    try:
        journal.append([event])
    finally:
        journal.close()
    before = (tmp_path / "control/journal.log").read_bytes()
    with SharedFilesystemStore(tmp_path, "task-replay") as store:
        with pytest.raises(UnsafeRecovery, match="task target"):
            Coordinator(store, exclusive_owner="old owner stopped", recover=True)
    assert (tmp_path / "control/journal.log").read_bytes() == before
