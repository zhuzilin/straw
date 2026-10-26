"""Valid framing must not manufacture a missing batch during business replay."""

import json

import pytest

from straw import SharedFilesystemStore
from straw.backend import FilesystemBackend
from straw.coordinator import Coordinator
from straw.errors import UnsafeRecovery
from straw.journal import Journal


@pytest.mark.parametrize("prior", ["absent", None, False])
def test_batch_ready_replay_rejects_missing_or_nonobject_plan(tmp_path, prior):
    with SharedFilesystemStore(tmp_path, "metadata-replay") as store:
        queue = Coordinator(store, exclusive_owner="test")
        queue.close()

    # Quiescent malformed-business fixture: use the real frame writer, with
    # intact checksums and run identity. This is not a reachable valid commit.
    identity = json.loads((tmp_path / "run.json").read_text())
    journal = Journal(FilesystemBackend(tmp_path), identity, recover=True)
    try:
        if prior != "absent":
            journal.append([{"type": "BatchPlanned", "batch_id": "missing", "batch": prior}])
        journal.append(
            [
                {
                    "type": "BatchReady",
                    "batch_id": "missing",
                    "consumer_id": "training",
                    "ready_ref": None,
                    "state": {"fetch_cursor": 0, "processed_cursor": 0},
                }
            ]
        )
    finally:
        journal.close()
    before = (tmp_path / "control/journal.log").read_bytes()
    with SharedFilesystemStore(tmp_path, "metadata-replay") as store:
        with pytest.raises(UnsafeRecovery, match="metadata path"):
            Coordinator(store, exclusive_owner="old owner stopped", recover=True)
    assert (tmp_path / "control/journal.log").read_bytes() == before
