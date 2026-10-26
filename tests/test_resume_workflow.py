"""Resume from durable sampling progress and a matching model/data checkpoint."""

import json
import os
import subprocess
import sys
from dataclasses import asdict
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import InvalidReference, StaleAttempt, StorageUnavailable, UnsafeRecovery
from straw.protocol import Lease, RecordSetRef, decode, encode


def open_store(root):
    return SharedFilesystemStore(root, "resume", online_gc=True, segment_target_bytes=0)


def open_queue(store, *, recover=False):
    return Coordinator(store, exclusive_owner="test predecessor stopped", recover=recover)


def blob(store, name, value, *, dependencies=()):
    return store.publish([Record(name, encode(value), "json.v1")], submission_id=name, dependencies=dependencies)


def _sampling_crash(root, mode, report):
    store = open_store(root)
    queue = open_queue(store)
    initial = blob(store, "initial", {"tokens": [], "rng": 1})
    queue.submit_tasks("sample", [TaskSpec("sample", input_ref=initial, max_attempts=4)])
    lease = queue.acquire("sampler").assignments[0].lease
    continuation = blob(store, "partial", {"tokens": [11, 12], "rng": 23}, dependencies=[initial])
    if mode == "saved":
        assert queue.save_task_progress(lease, request_id="progress", input_ref=continuation) == "saved"
    elif mode == "yielded":
        assert queue.yield_task(lease, request_id="progress", input_ref=continuation) == "pending"
    elif mode in ("before_accept", "lost_reply"):
        cut = "before_journal_write" if mode == "before_accept" else "after_journal_sync"
        interrupted = []

        def interrupt(at):
            if at == cut:
                interrupted.append(at)
                raise RuntimeError("sampling progress reply interrupted")

        store.backend.fault = interrupt
        with pytest.raises(RuntimeError, match="sampling progress reply interrupted"):
            queue.save_task_progress(lease, request_id="progress", input_ref=continuation)
        assert interrupted == [cut]
    report.write_text(
        json.dumps({"initial": asdict(initial), "continuation": asdict(continuation), "lease": asdict(lease)})
    )
    # Do not run queue/store close or destructors. The parent waits for this
    # process to terminate before creating the replacement coordinator.
    os._exit(23)


@pytest.mark.parametrize("mode", ["unsaved", "saved", "yielded", "before_accept", "lost_reply"])
def test_sampling_restart_uses_only_committed_continuation(tmp_path, mode):
    report = tmp_path / "crash-fixture.json"
    child = subprocess.run(
        [
            sys.executable,
            "-I",
            "-c",
            "import sys; from pathlib import Path; sys.path.insert(0, sys.argv[1]); "
            "from test_resume_workflow import _sampling_crash; "
            "_sampling_crash(Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4]))",
            str(Path(__file__).parent),
            str(tmp_path),
            mode,
            str(report),
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert child.returncode == 23, child.stdout + child.stderr
    fixture = json.loads(report.read_text())
    initial = RecordSetRef.from_dict(fixture["initial"])
    continuation = RecordSetRef.from_dict(fixture["continuation"])
    lease = Lease(**fixture["lease"])
    with open_store(tmp_path) as store, open_queue(store, recover=True) as recovered:
        recovered.collect_garbage()
        assignment = recovered.acquire("replacement").assignments[0]
        has_continuation = mode not in ("unsaved", "before_accept")
        expected = continuation if has_continuation else initial
        assert assignment.task.input_ref == expected
        restored = decode(next(store.read(assignment.task.input_ref)).payload)
        assert restored == ({"tokens": [11, 12], "rng": 23} if has_continuation else {"tokens": [], "rng": 1})
        assert assignment.lease.attempt_id != lease.attempt_id
        assert recovered.task_status("sample")["failures"] == (0 if mode == "yielded" else 1)
        with pytest.raises(StaleAttempt):
            recovered.save_task_progress(lease, request_id="late", input_ref=continuation)
        output = store.publish(
            [Record("result", b"done", metadata={"task_id": "sample", "attempt_id": assignment.lease.attempt_id})],
            submission_id="result",
        )
        receipt = recovered.complete_task(
            assignment.lease, submission_id="accepted", result_ref=output, result_digest=output.digest
        )
        assert (
            recovered.complete_task(
                assignment.lease, submission_id="accepted", result_ref=output, result_digest=output.digest
            )
            == receipt
        )
        assert recovered.read_commits().commits == (receipt,)


@pytest.mark.parametrize("protect_replay", [False, True])
def test_checkpoint_resume_after_later_training_and_gc(tmp_path, protect_replay):
    with open_store(tmp_path) as store, open_queue(store) as queue:
        queue.submit_tasks("training-inputs", [TaskSpec(f"sample-{i}") for i in range(3)])
        receipts = []

        def produce(index):
            lease = queue.acquire("sampler").assignments[0].lease
            result = store.publish(
                [
                    Record(
                        f"result-{index}",
                        encode({"gradient": index + 1}),
                        "json.v1",
                        metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
                    )
                ],
                submission_id=f"result-{index}",
            )
            receipts.append(
                queue.complete_task(
                    lease, submission_id=f"accepted-{index}", result_ref=result, result_digest=result.digest
                )
            )

        produce(0)
        checkpoint_state = blob(store, "state-1", {"cursor": 1, "pending": []})
        model = blob(store, "model-1", {"weight": -0.1, "optimizer_step": 1})
        checkpoint = blob(
            store,
            "checkpoint-1",
            {
                "version": 1,
                "checkpoint_id": "step-1",
                "consumer_state": asdict(checkpoint_state),
                "batch_ids": [],
                "training_dependencies": [asdict(model)],
            },
            dependencies=[checkpoint_state, model],
        )
        queue.register_checkpoint("step-1", checkpoint)
        # These results did not exist when the immutable checkpoint was made.
        produce(1)
        produce(2)
        if protect_replay:
            # An application replay owner keeps post-checkpoint work until a
            # newer model/optimizer checkpoint commits; processed != checkpointed.
            store.retain("training-replay:step-1", [receipts[1].result_ref])
        transient = blob(store, "state-2", {"cursor": 2, "pending": []})
        token = queue.open_consumer("training", exclusive_owner="test trainer")
        queue.save_consumer_state(
            "training", token=token, request_id="step-2", state_ref=transient, fetch_cursor=2, processed_cursor=2
        )
        queue.collect_garbage()
        replay_path = tmp_path / receipts[1].result_ref.manifest.segment.path
        assert replay_path.exists() == protect_replay
        accepted_ids = [r.commit_id for r in receipts]
    with open_store(tmp_path) as store, open_queue(store, recover=True) as recovered:
        latest = recovered.load_consumer_state("training")
        assert latest["processed_cursor"] == 2
        manifest = decode(next(store.read(checkpoint)).payload)
        checkpoint_state = RecordSetRef.from_dict(manifest["consumer_state"])
        model = RecordSetRef.from_dict(manifest["training_dependencies"][0])
        parameters = decode(next(store.read(model)).payload)
        cursor = decode(next(store.read(checkpoint_state)).payload)["cursor"]
        assert parameters["optimizer_step"] == cursor == 1
        assert cursor != latest["processed_cursor"]
        token = recovered.open_consumer("training", exclusive_owner="recovered trainer")

        def restore():
            return recovered.save_consumer_state(
                "training",
                token=token,
                request_id="restore-step-1",
                state_ref=checkpoint_state,
                fetch_cursor=cursor,
                processed_cursor=cursor,
            )

        if not protect_replay:
            before = recovered.journal.path.read_bytes()
            with pytest.raises((InvalidReference, StorageUnavailable, UnsafeRecovery)):
                restore()
            assert recovered.journal.path.read_bytes() == before
            assert recovered.load_consumer_state("training")["processed_cursor"] == 2
            return
        restore()
        # The restored queue has re-owned replay data before the temporary
        # application owner is released; collection must still preserve it.
        store.release("training-replay:step-1")
        recovered.collect_garbage()
        replay = recovered.read_commits(cursor).commits
        assert [r.commit_id for r in replay] == accepted_ids[1:]
        for receipt in replay:
            parameters["weight"] -= 0.1 * decode(next(store.read(receipt.result_ref)).payload)["gradient"]
            parameters["optimizer_step"] += 1
        assert parameters["optimizer_step"] == 3
        assert parameters["weight"] == pytest.approx(-0.6)
