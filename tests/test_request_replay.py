"""Durable request replies remain replayable after their data has been retired."""

import json
from dataclasses import asdict

import pytest

from straw import Record, SharedFilesystemStore
from straw.coordinator import Coordinator
from straw.errors import IdempotencyConflict


@pytest.mark.parametrize("method", ["save_consumer_state", "plan_batch", "batch_ready", "register_checkpoint"])
@pytest.mark.parametrize("reopen", [False, True])
def test_request_replay_after_gc_uses_receipt_without_reviving_data(tmp_path, method, reopen):
    store = SharedFilesystemStore(
        tmp_path, "request-replay", online_gc=True, segment_target_bytes=0, codecs=("bytes.v1", "json.v1")
    )
    queue = Coordinator(store, exclusive_owner="test owner")
    token = queue.open_consumer("training", exclusive_owner="trainer")

    def publish(name, value=None, dependencies=()):
        return store.publish(
            [
                Record(
                    name,
                    name.encode() if value is None else json.dumps(value).encode(),
                    codec="bytes.v1" if value is None else "json.v1",
                )
            ],
            submission_id=name,
            dependencies=dependencies,
        )

    old_state = publish("old-state")
    new_state = publish("new-state")
    if method == "save_consumer_state":
        args = dict(consumer_id="training", request_id="old", state_ref=old_state, fetch_cursor=0, processed_cursor=0)
        expected = queue.save_consumer_state(token=token, **args)
        retired = old_state
    elif method in {"plan_batch", "batch_ready"}:
        plan, ready = publish("plan"), publish("ready")
        plan_args = dict(consumer_id="training", batch_id="b", input_positions=[], plan_ref=plan)
        ready_args = dict(
            consumer_id="training",
            batch_id="b",
            ready_ref=ready,
            state_ref=old_state,
            fetch_cursor=0,
            processed_cursor=0,
        )
        queue.plan_batch(token=token, **plan_args)
        ready_reply = queue.batch_ready(token=token, **ready_args)
        args = plan_args if method == "plan_batch" else ready_args
        expected = queue.get_batch("b") if method == "plan_batch" else ready_reply
        retired = plan if method == "plan_batch" else ready
    else:
        checkpoint = publish(
            "checkpoint",
            dict(
                version=1, checkpoint_id="c", consumer_state=asdict(old_state), batch_ids=[], training_dependencies=[]
            ),
            [old_state],
        )
        args = dict(checkpoint_id="c", checkpoint_ref=checkpoint)
        expected = queue.register_checkpoint(**args)
        queue.release_checkpoint("c")
        retired = checkpoint

    progress = publish(
        "finished",
        dict(
            version=1,
            processed_positions=[],
            finished_batches=["b"] if method in {"plan_batch", "batch_ready"} else [],
        ),
    )
    queue.save_consumer_state(
        "training",
        token=token,
        request_id="latest",
        state_ref=new_state,
        fetch_cursor=0,
        processed_cursor=0,
        progress_ref=progress,
    )
    store.seal()
    queue.collect_garbage()
    assert not (tmp_path / retired.manifest.segment.path).exists()
    if reopen:
        queue.close()
        queue = Coordinator(store, exclusive_owner="previous owner closed", recover=True)
        token = queue.open_consumer("training", exclusive_owner="recovered trainer")

    before = queue._call("state", {})
    journal = queue.journal.path.read_bytes()
    catalog = (tmp_path / "storage.log").read_bytes()
    call_args = args if method == "register_checkpoint" else dict(token=token, **args)
    assert getattr(queue, method)(**call_args) == expected
    assert queue._call("state", {}) == before
    assert queue.journal.path.read_bytes() == journal
    assert (tmp_path / "storage.log").read_bytes() == catalog
    assert not (tmp_path / retired.manifest.segment.path).exists()

    changed = dict(call_args)
    changed[
        {
            "save_consumer_state": "state_ref",
            "plan_batch": "plan_ref",
            "batch_ready": "ready_ref",
            "register_checkpoint": "checkpoint_ref",
        }[method]
    ] = new_state
    with pytest.raises(IdempotencyConflict):
        getattr(queue, method)(**changed)
    assert queue._call("state", {}) == before
    assert queue.journal.path.read_bytes() == journal
    assert (tmp_path / "storage.log").read_bytes() == catalog
    queue.close()
    store.close()
