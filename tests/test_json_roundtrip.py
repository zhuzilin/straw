"""JSON business values must survive queue WAL replay without numeric drift."""

import math

from straw import SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator


def test_recovery_preserves_a_nonintegral_run_configuration(tmp_path):
    # This finite positive duration previously changed again on each JSON read.
    duration = 9.901469416441159e-145
    with SharedFilesystemStore(tmp_path, "json-roundtrip") as store:
        with Coordinator(store, exclusive_owner="stopped predecessor", lease_seconds=duration):
            pass
        with Coordinator(store, exclusive_owner="stopped predecessor", lease_seconds=duration, recover=True):
            pass


def test_producer_continuation_survives_repeated_recovery_and_request_replay(tmp_path):
    continuation = {
        "temperature": 0.00018313042101781934,
        "nested": [9.901469416441159e-145, 1.294108664437241e276, -0.0],
    }
    tasks = [TaskSpec("sample")]
    with SharedFilesystemStore(tmp_path, "json-roundtrip") as store:
        with Coordinator(store, exclusive_owner="stopped predecessor") as queue:
            accepted = queue.submit_tasks("submit", tasks, producer_id="sampler", producer_state=continuation)
            before = queue.producer_state("sampler")
        for _ in range(3):
            with Coordinator(store, exclusive_owner="stopped predecessor", recover=True) as queue:
                restored = queue.producer_state("sampler")
                assert restored == before
                assert restored == continuation
                assert math.copysign(1, restored["nested"][-1]) == -1
                sequence = queue.journal.sequence
                assert (
                    queue.submit_tasks("submit", tasks, producer_id="sampler", producer_state=continuation) == accepted
                )
                assert queue.journal.sequence == sequence
