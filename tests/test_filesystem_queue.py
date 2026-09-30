"""Queue invariants on local POSIX and an explicitly selected shared mount.

Set STRAW_TEST_ROOT to run the same tests under a real mount. This does
not certify a JuiceFS profile or replace independent-client admission tests.
"""

import dataclasses
import errno
import multiprocessing
import os
import shutil
import struct
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.backend import FilesystemBackend
from straw.coordinator import Coordinator
from straw.errors import (
    CorruptData,
    IdempotencyConflict,
    IndeterminateCommit,
    InvalidReference,
    LeaseExpired,
    StaleAttempt,
    StorageUnavailable,
    UnsafeRecovery,
    UnsupportedSchema,
)
from straw.protocol import Limits, encode

# Independent v1 wire-format oracle for corruption tests, not a storage implementation.
HEADER = struct.Struct("<8sI")
FRAME = struct.Struct("<IQ")
HEADER_SIZE = 56

NUM_GPUS = 0


@pytest.fixture
def root(tmp_path):
    base = os.environ.get("STRAW_TEST_ROOT")
    if base:
        with tempfile.TemporaryDirectory(prefix="case-", dir=base) as directory:
            yield Path(directory)
    else:
        yield tmp_path


@pytest.fixture
def queue(root, request):
    standalone = request.node.originalname == "test_crash_windows_do_not_accept_physical_data"
    store = SharedFilesystemStore(root, "run", segment_target_bytes=0 if standalone else 1024**3)
    coordinator = Coordinator(store, exclusive_owner="test owns the only coordinator")
    yield coordinator
    coordinator.close()
    store.close()


def result(queue, lease, *, payload=b"hello", submission_id="physical", records=1):
    return queue.store.publish(
        [
            Record(
                f"r-{i}",
                payload,
                metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
                tokens=3,
            )
            for i in range(records)
        ],
        submission_id=submission_id,
    )


def complete(queue, lease, *, submission_id="completion", **kwargs):
    ref = result(
        queue,
        lease,
        submission_id=f"physical:{lease.attempt_id}:{submission_id}",
        **kwargs,
    )
    return queue.complete_task(lease, submission_id=submission_id, result_ref=ref, result_digest=ref.digest)


def recover(queue, **kwargs):
    queue.close()
    return Coordinator(queue.store, exclusive_owner="previous owner closed", recover=True, **kwargs)


def test_store_order_empty_payload_reference_relocation(root):
    store = SharedFilesystemStore(root / "source", "run")
    ref = store.publish([Record("a", b""), Record("b", b"\x00\xff", tokens=2)], submission_id="a")
    assert ref.records == 2 and ref.tokens == 2 and ref.payload_bytes == 2
    assert [record.payload for record in store.read(ref)] == [b"", b"\x00\xff"]
    assert len(encode(dataclasses.asdict(ref))) < 1000
    shutil.copytree(root / "source", root / "other_mount")
    assert [record.record_id for record in SharedFilesystemStore(root / "other_mount", "run").read(ref)] == ["a", "b"]


def test_logical_digest_independent_of_physical_layout(root):
    store = SharedFilesystemStore(root, "run")
    records = [Record("a", b"first"), Record("b", b"second")]
    first = store.publish(records, submission_id="together")
    refs = [store.write_records([record], submission_id=record.record_id)[0] for record in records]
    second = store.record_set(refs, submission_id="separate")
    assert first.digest == second.digest
    assert first.manifest != second.manifest
    reversed_ref = store.record_set(refs[::-1], submission_id="reverse")
    assert first.digest != reversed_ref.digest


def test_store_many_tasks_one_segment_accept_only_one(queue):
    queue.submit_tasks("request", [TaskSpec("a"), TaskSpec("b")])
    a, b = [assignment.lease for assignment in queue.acquire("worker", 2).assignments]
    refs = queue.store.write_records(
        [
            Record(
                lease.task_id,
                lease.task_id.encode(),
                metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
            )
            for lease in (a, b)
        ],
        submission_id="shared-segment",
    )
    ref = queue.store.record_set(refs[:1], submission_id="a-only")
    queue.complete_task(a, submission_id="a", result_ref=ref, result_digest=ref.digest)
    assert queue.task_status("a")["state"] == "completed"
    assert queue.task_status("b")["state"] == "leased"
    assert len(queue.read_commits().commits) == 1
    with pytest.raises(InvalidReference, match="belong"):
        queue.complete_task(b, submission_id="wrong", result_ref=ref, result_digest=ref.digest)


@pytest.mark.parametrize(
    "change, error",
    [
        ({"path": "../outside.sealed"}, InvalidReference),
        ({"path": "/absolute.sealed"}, InvalidReference),
        ({"path": "raw/file.partial"}, InvalidReference),
        ({"run_id": "other"}, InvalidReference),
        ({"version": 100}, UnsupportedSchema),
        ({"size": 1}, CorruptData),
        ({"segment_id": "other"}, InvalidReference),
    ],
)
def test_invalid_references(root, change, error):
    store = SharedFilesystemStore(root, "run")
    (ref,) = store.write_records([Record("r", b"value")], submission_id="s")
    with pytest.raises(error):
        store.read_record(dataclasses.replace(ref, segment=dataclasses.replace(ref.segment, **change)))


def test_reference_cannot_escape_via_symlink(root, tmp_path):
    store = SharedFilesystemStore(root, "run")
    root.mkdir(exist_ok=True)
    (root / "escape").symlink_to("/tmp", target_is_directory=True)
    with pytest.raises(InvalidReference):
        store.backend.path("escape/outside.sealed")


def test_payload_corruption_is_never_skipped(root):
    store = SharedFilesystemStore(root, "run")
    ref = store.publish([Record("r", b"payload")], submission_id="s")
    (record_ref,) = store.validate(ref)
    path = store.backend.path(record_ref.segment.path)
    with path.open("r+b") as stream:
        stream.seek(HEADER.size)
        length, _ = FRAME.unpack(stream.read(FRAME.size))
        stream.seek(length, os.SEEK_CUR)
        stream.write(b"X")
    with pytest.raises(CorruptData, match="payload checksum"):
        list(store.read(ref))


@pytest.mark.parametrize("cut", [1, 32, 100])
def test_segment_truncation(root, cut):
    store = SharedFilesystemStore(root, "run")
    ref = store.publish([Record("r", b"payload")], submission_id="s")
    path = store.backend.path(ref.manifest.segment.path)
    with path.open("r+b") as stream:
        stream.truncate(ref.manifest.segment.size - cut)
    with pytest.raises(CorruptData):
        list(store.read(ref))


def test_records_and_metadata_ignore_legacy_size_quotas(root):
    store = SharedFilesystemStore(root, "run", max_record_bytes=1024, max_buffer_bytes=2048)
    large = store.publish([Record("r", b"x" * 1025)], submission_id="large")
    assert next(store.read(large)).payload == b"x" * 1025
    refs = store.write_records([Record(str(i), b"x" * 1000) for i in range(3)], submission_id="batch")
    assert [store.read_record(ref).payload for ref in refs] == [b"x" * 1000] * 3
    with pytest.raises(UnsupportedSchema):
        store.publish([Record("r", b"", codec="pickle")], submission_id="pickle")
    ref = store.publish(
        [Record("r", b"", metadata={"large": "x" * 65536})],
        submission_id="metadata",
    )
    assert next(store.read(ref)).metadata == {"large": "x" * 65536}


@pytest.mark.parametrize(
    "phase,sealed",
    [
        ("after_record", False),
        ("before_footer", False),
        ("after_data_sync", False),
        ("after_publish", True),
    ],
)
def test_crash_windows_do_not_accept_physical_data(queue, phase, sealed):
    class Crash(BaseException):
        pass

    def fault(current):
        if current == phase:
            raise Crash()

    queue.submit_tasks("s", [TaskSpec("t")])
    lease = queue.acquire("w").assignments[0].lease
    queue.store.backend.fault = fault
    with pytest.raises(Crash):
        result(queue, lease)
    queue.store.backend.fault = lambda _: None
    assert bool(list(queue.store.backend.root.rglob("*.sealed"))) == sealed
    restored = recover(queue)
    try:
        assert not restored.read_commits().commits
        assert restored.task_status("t")["state"] == "pending"
    finally:
        restored.close()


def test_ambiguous_rename_rechecks_same_publication(root):
    fired = False

    def fault(phase):
        nonlocal fired
        if phase == "after_publish" and not fired:
            fired = True
            raise OSError(errno.EIO, "reply lost after rename")

    backend = FilesystemBackend(root, fault=fault)
    store = SharedFilesystemStore(root, "run", backend=backend, segment_target_bytes=0)
    ref = store.publish([Record("r", b"payload")], submission_id="s")
    assert next(store.read(ref)).payload == b"payload"
    assert len(list(root.rglob("*.sealed"))) == 1  # data and manifest packed together; no retry duplicate


def test_visibility_retry_is_bounded(root):
    store = SharedFilesystemStore(root, "run")
    ref = store.publish([Record("r", b"payload")], submission_id="s")
    path = store.backend.path(ref.manifest.segment.path)
    hidden = path.with_suffix(".hidden")
    path.rename(hidden)
    # The native reader must give up after the finite visibility window, then
    # remain usable when the independently published file becomes visible.
    with pytest.raises(StorageUnavailable):
        next(store.read(ref))
    hidden.rename(path)
    assert next(store.read(ref)).payload == b"payload"


def test_unresolved_publication_retries_the_original_identity(root):
    store = SharedFilesystemStore(root, "run", segment_target_bytes=0)
    failed = True

    def fault(phase):
        if phase == "before_publish" and failed:
            raise OSError(errno.EIO, "publication unavailable")

    store.backend.fault = fault
    records = [Record("r", b"payload")]
    with pytest.raises(IndeterminateCommit):
        store.write_records(records, submission_id="original")
    partials = list(root.rglob("*.partial"))
    assert len(partials) == 1
    with pytest.raises(IndeterminateCommit):
        store.write_records(records, submission_id="different")
    failed = False
    (ref,) = store.write_records(records, submission_id="original")
    assert Path(ref.segment.path).stem == partials[0].stem
    assert len(list(root.rglob("*.sealed"))) == 1
    assert store.read_record(ref).payload == b"payload"


def test_submit_idempotency_and_terminal_states(queue):
    tasks = [TaskSpec("a"), TaskSpec("b")]
    assert queue.submit_tasks("r", tasks) == ["a", "b"]
    assert queue.submit_tasks("r", tasks) == ["a", "b"]
    with pytest.raises(IdempotencyConflict):
        queue.submit_tasks("r", [TaskSpec("c")])
    with pytest.raises(IdempotencyConflict):
        queue.submit_tasks("different", [TaskSpec("a")])
    queue.cancel_task("a", request_id="cancel")
    lease = queue.acquire("w").assignments[0].lease
    assert queue.fail_task(lease, request_id="fail", failure={"reason": "invalid"}, retryable=False) == "failed"
    assert queue.acquire("w").status == "empty"
    queue.seal_input("seal")
    assert queue.acquire("w").status == "end_of_input"
    assert queue.read_commits().end_of_input


def test_concurrent_acquire_has_unique_current_leases(queue):
    queue.submit_tasks("r", [TaskSpec(str(i)) for i in range(100)])
    with ThreadPoolExecutor(max_workers=8) as executor:
        results = list(executor.map(lambda i: queue.acquire(str(i), 10), range(10)))
    leases = [item.lease for result in results for item in result.assignments]
    assert len(leases) == len({lease.task_id for lease in leases}) == 100


def test_complete_idempotency_survives_epoch_and_physical_relayout(queue):
    queue.submit_tasks("r", [TaskSpec("a")])
    lease = queue.acquire("w").assignments[0].lease
    ref = result(queue, lease, records=3)
    receipt = queue.complete_task(lease, submission_id="c", result_ref=ref, result_digest=ref.digest)
    other_layout = queue.store.publish(list(queue.store.read(ref)), submission_id="new-files")
    restored = recover(queue)
    try:
        assert (
            restored.complete_task(
                lease,
                submission_id="c",
                result_ref=other_layout,
                result_digest=other_layout.digest,
            )
            == receipt
        )
        assert len(restored.read_commits().commits) == 1
        with pytest.raises(IdempotencyConflict):
            restored.complete_task(lease, submission_id="c", result_ref=ref, result_digest="changed")
    finally:
        restored.close()


def test_expiry_reassignment_rejects_late_worker(root):
    clock = [1.0]
    queue = Coordinator(
        SharedFilesystemStore(root, "run"),
        exclusive_owner="test",
        clock=lambda: clock[0],
        lease_seconds=2,
    )
    try:
        queue.submit_tasks("r", [TaskSpec("a")])
        old = queue.acquire("A").assignments[0].lease
        clock[0] = 4
        with pytest.raises(LeaseExpired):
            complete(queue, old)
        current = queue.acquire("B").assignments[0].lease
        assert current.generation == old.generation + 1 and current.attempt_id != old.attempt_id
        assert queue.heartbeat([old, current]) == ["StaleAttempt", "extended"]
        receipt = complete(queue, current, submission_id="B", payload=b"B")
        with pytest.raises(StaleAttempt):
            complete(queue, old, submission_id="A", payload=b"A")
        assert queue.read_commits().commits == (receipt,)
    finally:
        queue.close()


def test_recovery_revokes_old_leases_and_enforces_attempt_limit(queue):
    queue.submit_tasks("r", [TaskSpec("a", max_attempts=1), TaskSpec("b")])
    assignments = queue.acquire("w", 2).assignments
    restored = recover(queue)
    try:
        assert restored.task_status("a")["state"] == "failed"
        assert restored.task_status("b")["state"] == "pending"
        for assignment in assignments:
            with pytest.raises(StaleAttempt):
                complete(restored, assignment.lease, submission_id=assignment.task.task_id)
    finally:
        restored.close()


def test_reply_loss_after_complete_returns_original_receipt(queue):
    queue.submit_tasks("r", [TaskSpec("a")])
    lease = queue.acquire("w").assignments[0].lease
    ref = result(queue, lease)

    def fault(phase):
        if phase == "before_complete_reply":
            raise ConnectionError("client disconnected")

    queue.store.backend.fault = fault
    with pytest.raises(ConnectionError):
        queue.complete_task(lease, submission_id="c", result_ref=ref, result_digest=ref.digest)
    queue.store.backend.fault = lambda _: None
    restored = recover(queue)
    try:
        receipt = restored.complete_task(lease, submission_id="c", result_ref=ref, result_digest=ref.digest)
        assert restored.read_commits().commits == (receipt,)
    finally:
        restored.close()


@pytest.mark.parametrize("phase,committed", [("after_journal_part", False), ("after_journal_sync", True)])
def test_torn_and_unacknowledged_journal_transactions(queue, phase, committed):
    queue.submit_tasks("first", [TaskSpec("a")])

    def fault(current):
        if current == phase:
            raise OSError(errno.EIO, "crash")

    queue.store.backend.fault = fault
    with pytest.raises(IndeterminateCommit):
        queue.submit_tasks("second", [TaskSpec("b")])
    with pytest.raises(UnsafeRecovery):
        queue.acquire("w")
    queue.store.backend.fault = lambda _: None
    restored = recover(queue)
    try:
        assert ("b" in restored.tasks) == committed
        assert restored.submit_tasks("second", [TaskSpec("b")]) == ["b"]
        assert set(restored.tasks) == {"a", "b"}
    finally:
        restored.close()


@pytest.mark.parametrize("offset", [8, HEADER_SIZE + 3, -12])
def test_complete_journal_corruption_stops_recovery(queue, offset):
    queue.submit_tasks("r", [TaskSpec("a")])
    path = queue.journal.path
    queue.close()
    with path.open("r+b") as stream:
        stream.seek(offset, os.SEEK_END if offset < 0 else os.SEEK_SET)
        original = stream.read(1)
        stream.seek(-1, os.SEEK_CUR)
        stream.write(bytes([original[0] ^ 0xFF]))
    with pytest.raises(CorruptData):
        Coordinator(queue.store, exclusive_owner="previous owner closed", recover=True)


def test_sync_failure_is_not_acknowledged(queue):
    def fault(phase):
        if phase == "before_file_sync":
            raise OSError(errno.ENOSPC, "quota full")

    queue.store.backend.fault = fault
    with pytest.raises(IndeterminateCommit):
        queue.submit_tasks("r", [TaskSpec("a")])
    assert "a" not in queue.tasks
    with pytest.raises(UnsafeRecovery):
        queue.acquire("w")


def test_legacy_budgets_do_not_block_acquire_complete_or_empty_output(root):
    limits = Limits(inflight_tasks=1, accepted_records=1, accepted_bytes=10)
    queue = Coordinator(SharedFilesystemStore(root, "run"), exclusive_owner="test", limits=limits)
    try:
        queue.submit_tasks("r", [TaskSpec("a"), TaskSpec("b", allow_empty=True)])
        lease = queue.acquire("w").assignments[0].lease
        second = queue.acquire("w").assignments[0].lease
        complete(queue, lease, payload=b"x" * 20)
        assert queue.acquire("w").status == "empty"
        token = queue.open_consumer("training", exclusive_owner="test")
        state = queue.store.publish([Record("state", b"{}", "json.v1")], submission_id="state")
        queue.save_consumer_state(
            "training",
            token=token,
            request_id="state",
            state_ref=state,
            fetch_cursor=1,
            processed_cursor=1,
        )
        receipt = complete(queue, second, submission_id="empty", records=0)
        assert receipt.result_ref.records == 0
        assert not list(queue.store.read(receipt.result_ref))
        assert len(queue.read_commits().commits) == 2  # processing did not delete raw history
    finally:
        queue.close()


def test_consumer_fetch_processing_batch_and_checkpoint_are_distinct(queue):
    queue.submit_tasks("r", [TaskSpec("a")])
    lease = queue.acquire("w").assignments[0].lease
    complete(queue, lease)
    first = queue.read_commits()
    assert queue.read_commits() == first  # independent replay
    token = queue.open_consumer("training", exclusive_owner="test")
    with pytest.raises(UnsafeRecovery):
        queue.open_consumer("training", exclusive_owner="second")
    state = queue.store.publish([Record("state", b'{"pending":[0]}', "json.v1")], submission_id="state")
    queue.save_consumer_state(
        "training",
        token=token,
        request_id="fetched",
        state_ref=state,
        fetch_cursor=1,
        processed_cursor=0,
    )
    assert queue.load_consumer_state("training")["processed_cursor"] == 0
    plan = queue.store.publish(
        [Record("plan", b'{"transform":"v1","order":[0]}', "json.v1")],
        submission_id="plan",
    )
    queue.plan_batch("training", token=token, batch_id="batch", input_positions=[0], plan_ref=plan)
    restored = recover(queue)
    try:
        token = restored.open_consumer("training", exclusive_owner="test after recovery")
        assert not restored.get_batch("batch")["ready"]
        restored.plan_batch(
            "training",
            token=token,
            batch_id="batch",
            input_positions=[0],
            plan_ref=plan,
        )
        with pytest.raises(IdempotencyConflict):
            restored.plan_batch(
                "training",
                token=token,
                batch_id="batch",
                input_positions=[],
                plan_ref=plan,
            )
        output = restored.store.publish([Record("batch", b"converted")], submission_id="converted")
        restored.batch_ready(
            "training",
            token=token,
            batch_id="batch",
            ready_ref=output,
            state_ref=state,
            fetch_cursor=1,
            processed_cursor=1,
        )
        assert not restored.checkpoints
        # Mock optimizer advanced, but the previous model checkpoint is still
        # authoritative until the final manifest below is durably published.
        model = restored.store.publish([Record("model", b'{"step":1}', "json.v1")], submission_id="model")
        value = {
            "version": 1,
            "checkpoint_id": "step-1",
            "consumer_state": dataclasses.asdict(state),
            "batch_ids": ["batch"],
            "training_dependencies": [dataclasses.asdict(model)],
        }
        checkpoint = restored.store.publish(
            [Record("checkpoint", encode(value), "json.v1")],
            submission_id="checkpoint",
            dependencies=[state, model],
        )
        # Crash after final manifest, before the rebuildable queue registration.
        restored = recover(restored)
        assert not restored.checkpoints
        restored.register_checkpoint("step-1", checkpoint)
        restored.register_checkpoint("step-1", checkpoint)
        assert len(restored.checkpoints) == 1
        assert restored.metrics()["usage_with_reservations"]["ready_bytes"] == 0
        assert len(restored.read_commits().commits) == 1
    finally:
        restored.close()


def test_missing_accepted_data_fails_and_pauses_new_work(queue):
    queue.submit_tasks("r", [TaskSpec("a"), TaskSpec("b")])
    lease = queue.acquire("w").assignments[0].lease
    ref = result(queue, lease)
    queue.store.backend.path(ref.manifest.segment.path).unlink()
    with pytest.raises(StorageUnavailable):
        queue.complete_task(lease, submission_id="c", result_ref=ref, result_digest=ref.digest)
    with pytest.raises(StorageUnavailable):
        queue.acquire("w")
    assert queue.fail_task(lease, request_id="failure", failure={"category": "StorageUnavailable"}) == "pending"


def test_optimizer_progress_after_checkpoint_is_replayed_from_matching_consumer_state(
    queue,
):
    from straw.protocol import RecordSetRef, decode

    queue.submit_tasks("training-input", [TaskSpec(f"task-{i}") for i in range(3)])
    for i in range(3):
        lease = queue.acquire("worker").assignments[0].lease
        complete(
            queue,
            lease,
            submission_id=f"complete-{i}",
            payload=encode({"gradient": i + 1}),
        )
    accepted_ids = [receipt.commit_id for receipt in queue.read_commits().commits]
    token = queue.open_consumer("training", exclusive_owner="mock trainer")

    def blob(owner, name, value):
        return owner.store.publish([Record(name, encode(value), "json.v1")], submission_id=name)

    state = blob(queue, "state-step-1", {"cursor": 1, "pending": []})
    model = blob(queue, "model-step-1", {"weight": -0.1, "optimizer_step": 1})
    manifest = {
        "version": 1,
        "checkpoint_id": "step-1",
        "consumer_state": dataclasses.asdict(state),
        "batch_ids": [],
        "training_dependencies": [dataclasses.asdict(model)],
    }
    checkpoint = queue.store.publish(
        [Record("checkpoint", encode(manifest), "json.v1")],
        submission_id="checkpoint",
        dependencies=[state, model],
    )
    queue.register_checkpoint("step-1", checkpoint)
    # A subsequent optimizer step and processing snapshot are not a model checkpoint.
    uncheckpointed_weight = -0.1 - 0.1 * 2
    transient = blob(queue, "transient-step-2", {"cursor": 2, "pending": []})
    queue.save_consumer_state(
        "training",
        token=token,
        request_id="step-2",
        state_ref=transient,
        fetch_cursor=2,
        processed_cursor=2,
    )
    restored = recover(queue)
    try:
        assert restored.load_consumer_state("training")["processed_cursor"] == 2
        root = decode(next(restored.store.read(checkpoint)).payload)
        checkpoint_state = RecordSetRef.from_dict(root["consumer_state"])
        cursor = decode(next(restored.store.read(checkpoint_state)).payload)["cursor"]
        model_ref = RecordSetRef.from_dict(root["training_dependencies"][0])
        parameters = decode(next(restored.store.read(model_ref)).payload)
        assert parameters == {"weight": -0.1, "optimizer_step": 1}
        token = restored.open_consumer("training", exclusive_owner="recovered mock trainer")
        restored.save_consumer_state(
            "training",
            token=token,
            request_id="restore-step-1",
            state_ref=checkpoint_state,
            fetch_cursor=cursor,
            processed_cursor=cursor,
        )
        replay = restored.read_commits(cursor, 1).commits[0]
        gradient = decode(next(restored.store.read(replay.result_ref)).payload)["gradient"]
        parameters["weight"] -= 0.1 * gradient
        parameters["optimizer_step"] += 1
        assert parameters["weight"] == uncheckpointed_weight and parameters["optimizer_step"] == 2
        assert [receipt.commit_id for receipt in restored.read_commits().commits] == accepted_ids
    finally:
        restored.close()


def test_recovery_requires_external_single_owner(root):
    store = SharedFilesystemStore(root, "run")
    with pytest.raises(UnsafeRecovery):
        Coordinator(store, exclusive_owner=False)


def test_producer_cursor_is_atomic_with_task_submission(queue):
    state = {"version": 1, "position": 2, "seed": 17}
    queue.submit_tasks(
        "producer-0",
        [TaskSpec("a"), TaskSpec("b")],
        producer_id="dataset",
        producer_state=state,
    )
    assert queue.producer_state("dataset") == state
    restored = recover(queue)
    try:
        assert restored.producer_state("dataset") == state
        assert restored.submit_tasks(
            "producer-0",
            [TaskSpec("a"), TaskSpec("b")],
            producer_id="dataset",
            producer_state=state,
        ) == ["a", "b"]
        assert len(restored.tasks) == 2
        with pytest.raises(IdempotencyConflict):
            restored.submit_tasks(
                "producer-0",
                [TaskSpec("a"), TaskSpec("b")],
                producer_id="dataset",
                producer_state={**state, "position": 3},
            )
    finally:
        restored.close()


def test_partial_yield_survives_recovery_and_does_not_spend_failure_retries(queue):
    queue.submit_tasks("source", [TaskSpec("partial", max_attempts=1)])
    prior = None
    for index in range(4):
        assignment = queue.acquire("worker").assignments[0]
        if prior is not None:
            assert assignment.task.input_ref == prior
        lease = assignment.lease
        assert lease.generation == index + 1
        prior = result(
            queue,
            lease,
            payload=f"partial-{index}".encode(),
            submission_id=f"partial-{index}",
        )
        assert queue.yield_task(lease, request_id=f"yield-{index}", input_ref=prior) == "pending"
        assert queue.yield_task(lease, request_id=f"yield-{index}", input_ref=prior) == "pending"
    restored = recover(queue)
    try:
        assignment = restored.acquire("resumed-worker").assignments[0]
        assert assignment.task.input_ref == prior
        assert next(restored.store.read(prior)).payload == b"partial-3"
        complete(restored, assignment.lease)
        assert len(restored.read_commits().commits) == 1
    finally:
        restored.close()


def test_juicefs_profile_requires_explicit_declarations(root):
    with pytest.raises(ValueError, match="declaration"):
        FilesystemBackend(root, profile="juicefs")
    declaration = {
        "direct_mount": True,
        "writeback": False,
        "open_cache": 0,
        "readdir_cache": False,
        "client_version": "test-only",
        "durability_description": "not an admission certificate",
    }
    backend = FilesystemBackend(root, profile="juicefs", declaration=declaration)
    assert not backend.diagnostics()["mount_options_automatically_verified"]
    assert backend.diagnostics()["multi_machine_admission"] == "not_verified"
    with pytest.raises(ValueError):
        FilesystemBackend(root, profile="juicefs", declaration={**declaration, "writeback": True})


def test_inspect_retains_shared_segments_and_only_deletes_confirmed_orphans(queue):
    from straw.inspect import cleanup_orphans, inspect_run

    queue.submit_tasks("r", [TaskSpec("a")])
    lease = queue.acquire("w").assignments[0].lease
    receipt = complete(queue, lease)
    # A whole pack remains retained if any extent is reachable. Only an
    # independent writer's entirely unreferenced pack is an orphan.
    cohabitant = queue.store.publish([Record("unused", b"same-pack")], submission_id="unused")
    other = SharedFilesystemStore(queue.store.backend.root, "run")
    orphan = other.publish([Record("orphan", b"unused")], submission_id="orphan")
    other.close()
    queue.close()
    root = queue.store.backend.root
    report = inspect_run(root)
    assert report["deletion_allowed"]
    assert cohabitant.manifest.segment.path in report["retained_paths"]
    assert orphan.manifest.segment.path in report["unaccepted_sealed"]
    assert receipt.result_ref.manifest.segment.path in report["retained_paths"]
    with pytest.raises(UnsafeRecovery):
        cleanup_orphans(root, report, all_participants_stopped=False)
    removed = cleanup_orphans(root, report, all_participants_stopped=True)
    assert set(removed) == set(report["unaccepted_sealed"])
    assert next(queue.store.read(receipt.result_ref)).payload == b"hello"


def test_inspect_does_not_repair_journal_or_delete_on_missing_retained_data(queue):
    from straw.inspect import cleanup_orphans, inspect_run

    queue.submit_tasks("r", [TaskSpec("a")])
    lease = queue.acquire("w").assignments[0].lease
    receipt = complete(queue, lease)
    queue.close()
    root = queue.store.backend.root
    queue.store.backend.path(receipt.result_ref.manifest.segment.path).unlink()
    report = inspect_run(root)
    assert not report["deletion_allowed"] and report["retained_dependency_errors"]
    with pytest.raises(UnsafeRecovery):
        cleanup_orphans(root, report, all_participants_stopped=True)
    with queue.journal.path.open("ab") as stream:
        stream.write(b"SLM")
    original = queue.journal.path.read_bytes()
    assert inspect_run(root)["incomplete_journal_tail_bytes"] == 3
    assert queue.journal.path.read_bytes() == original


def _rpc_server(root, connection):
    from straw.rpc import serve

    coordinator = Coordinator(SharedFilesystemStore(root, "run"), exclusive_owner="spawned test server")
    serve(
        coordinator,
        ("127.0.0.1", 0),
        token="test-token",
        ready=lambda address: connection.send(address[1]),
    )


def _rpc_worker(root, port, index):
    from straw.protocol import Lease
    from straw.rpc import QueueClient

    client = QueueClient(f"http://127.0.0.1:{port}", token="test-token")
    store = SharedFilesystemStore(root, "run")
    client.call("submit_tasks", request_id=f"submit-{index}", tasks=[TaskSpec(f"task-{index}")])
    assignment = client.call("acquire", worker_id=str(index), max_tasks=1)["assignments"][0]
    lease = Lease(**assignment["lease"])
    ref = store.publish(
        [
            Record(
                lease.task_id,
                lease.task_id.encode(),
                metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
            )
        ],
        submission_id=f"physical-{index}",
    )
    client.call(
        "complete_task",
        lease=lease,
        submission_id=f"complete-{index}",
        result_ref=ref,
        result_digest=ref.digest,
    )


def test_multiple_producer_worker_processes_and_server_crash(root):
    from straw.rpc import QueueClient

    context = multiprocessing.get_context("spawn")
    parent, child = context.Pipe(duplex=False)
    server = context.Process(target=_rpc_server, args=(str(root), child))
    workers = []
    server.start()
    try:
        assert parent.poll(30), "server did not become ready"
        port = parent.recv()
        for index in range(4):
            process = context.Process(target=_rpc_worker, args=(str(root), port, index))
            workers.append(process)
            process.start()
        for process in workers:
            process.join(30)
            assert process.exitcode == 0
        client = QueueClient(f"http://127.0.0.1:{port}", token="test-token")
        assert len(client.call("read_commits", cursor=0, limit=100)["commits"]) == 4
        server.kill()
        server.join(10)
        assert server.exitcode is not None
        restored = Coordinator(
            SharedFilesystemStore(root, "run"),
            exclusive_owner="server process confirmed terminal",
            recover=True,
        )
        try:
            page = restored.read_commits()
            assert len(page.commits) == 4
            assert {next(restored.store.read(commit.result_ref)).payload for commit in page.commits} == {
                f"task-{i}".encode() for i in range(4)
            }
        finally:
            restored.close()
    finally:
        for process in [*workers, server]:
            if process.is_alive():
                process.kill()
            process.join(10)
        parent.close()
        child.close()


def test_sparse_processing_releases_budget_and_checkpoint_view_can_rewind(root):
    queue = Coordinator(
        SharedFilesystemStore(root, "run"),
        exclusive_owner="test",
        limits=Limits(accepted_records=2),
    )
    queue.submit_tasks("inputs", [TaskSpec(str(i)) for i in range(3)])
    leases = [a.lease for a in queue.acquire("worker", 2).assignments]
    for i, lease in enumerate(leases):
        complete(queue, lease, submission_id=f"complete-{i}", payload=f"{i}".encode())
    assert queue.task_status("2")["state"] == "pending"
    token = queue.open_consumer("training", exclusive_owner="one builder")
    state = queue.store.publish([Record("state", b"opaque consumer snapshot")], submission_id="state")
    progress = queue.store.publish(
        [
            Record(
                "progress",
                encode({"version": 1, "processed_positions": [1], "finished_batches": []}),
                codec="json.v1",
            )
        ],
        submission_id="progress",
    )
    queue.save_consumer_state(
        "training",
        token=token,
        request_id="sparse",
        state_ref=state,
        fetch_cursor=2,
        processed_cursor=0,
        progress_ref=progress,
    )
    assert queue._usage()["records"] == 1
    restored = recover(queue, limits=queue.limits)
    try:
        assert restored._usage()["records"] == 1
        assert restored.acquire("worker").status == "acquired"
        token = restored.open_consumer("training", exclusive_owner="restored builder")
        restored.save_consumer_state(
            "training",
            token=token,
            request_id="earlier-checkpoint",
            state_ref=state,
            fetch_cursor=0,
            processed_cursor=0,
        )
        assert restored._usage()["records"] == 3  # two accepted plus the reserved attempt
        assert len(restored.read_commits().commits) == 2
    finally:
        restored.close()


def test_control_and_production_ignore_legacy_queue_capacities(root):
    queue = Coordinator(
        SharedFilesystemStore(root, "run"),
        exclusive_owner="test",
        limits=Limits(accepted_records=1, pending_tasks=1, inflight_tasks=1),
    )
    queue.submit_tasks("input", [TaskSpec("prompt")])
    lease = queue.acquire("worker").assignments[0].lease
    complete(queue, lease)
    queue.submit_tasks("next", [TaskSpec("next")])
    assert queue.acquire("worker").status == "acquired"
    queue.submit_tasks("collection", [TaskSpec("collection", control=True, estimated_records=0)])
    queue.submit_tasks("extra-control", [TaskSpec("extra", control=True, estimated_records=0)])
    assert queue.acquire("worker").status == "empty"
    control = queue.acquire("builder", control=True).assignments[0].lease
    complete(queue, control, submission_id="collection", payload=b"existing output refs")
    assert len(queue.read_commits().commits) == 2
    queue.close()


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__]))


def test_progress_batch_rejects_corrupt_later_manifest_without_partial_commit(queue):
    from straw import Publication

    original = queue.store.publish([Record("original", b"original")], submission_id="original")
    queue.submit_tasks("tasks", [TaskSpec("a", input_ref=original), TaskSpec("b", input_ref=original)])
    leases = [a.lease for a in queue.acquire("worker", 2).assignments]
    refs = queue.store.publish_many(
        [Publication((Record("a", b"first"),)), Publication((Record("b", b"second"),))],
        submission_id="progress",
    )
    assert refs[0].manifest.segment == refs[1].manifest.segment
    before_state = queue.tasks
    before_wal = queue.journal.path.read_bytes()
    ref = refs[1].manifest
    entry = queue.store.inspect_segment(ref.segment)["records"][ref.ordinal]
    start = ref.segment.offset + entry["offset"]
    with queue.store.backend.path(ref.segment.path).open("r+b") as stream:
        stream.seek(start)
        envelope_bytes, _ = FRAME.unpack(stream.read(FRAME.size))
        stream.seek(start + FRAME.size + envelope_bytes)
        byte = stream.read(1)
        stream.seek(-1, os.SEEK_CUR)
        stream.write(bytes([byte[0] ^ 1]))
    with pytest.raises(CorruptData, match="payload checksum"):
        queue.save_task_progress_many(
            [
                {
                    "lease": dataclasses.asdict(lease),
                    "input_ref": dataclasses.asdict(ref),
                }
                for lease, ref in zip(leases, refs, strict=True)
            ],
            request_id="batch",
        )
    assert queue.tasks == before_state
    assert queue.journal.path.read_bytes() == before_wal


def test_priority_yield_and_fifo_survive_wal_recovery(queue):
    queue.submit_tasks("initial", [TaskSpec("a"), TaskSpec("b"), TaskSpec("fresh")])
    leases = [assignment.lease for assignment in queue.acquire("writer", 2).assignments]
    refs = [result(queue, lease, submission_id=f"partial:{i}") for i, lease in enumerate(leases)]
    updates = [
        {
            "lease": dataclasses.asdict(lease),
            "input_ref": dataclasses.asdict(ref),
            "priority": 1,
            "scheduling_key": 2,
            "metadata": {"stage": "partial"},
        }
        for lease, ref in zip(reversed(leases), reversed(refs), strict=True)
    ]
    before = queue.journal.sequence
    assert queue.yield_tasks(updates, request_id="return") == "pending"
    assert queue.journal.sequence == before + 1
    assert queue.yield_tasks(updates, request_id="return") == "pending"
    assert queue.journal.sequence == before + 1
    queue.submit_tasks("ready", [TaskSpec("ready", priority=2, scheduling_key=99)])
    queue.submit_tasks("older", [TaskSpec("older", priority=1, scheduling_key=1)])
    assert [task.task_id for task in queue.pending_tasks()] == [
        "ready",
        "older",
        "b",
        "a",
        "fresh",
    ]
    assert queue.outstanding_reads() == ()
    restored = recover(queue)
    try:
        assignments = restored.acquire("next", 5).assignments
        assert [item.task.task_id for item in assignments] == [
            "ready",
            "older",
            "b",
            "a",
            "fresh",
        ]
        assert assignments[2].task.input_ref == refs[1]
        assert restored.heartbeat(leases) == ["StaleAttempt", "StaleAttempt"]
        assert all(restored.task_status(lease.task_id)["failures"] == 0 for lease in leases)
    finally:
        restored.close()


def test_batched_yield_rejects_stale_member_without_partial_changes(queue):
    queue.submit_tasks("tasks", [TaskSpec("a"), TaskSpec("b")])
    leases = [assignment.lease for assignment in queue.acquire("writer", 2).assignments]
    refs = [result(queue, lease, submission_id=f"partial:{i}") for i, lease in enumerate(leases)]
    queue.release_tasks([leases[1]], request_id="release-b")
    updates = [
        {
            "lease": dataclasses.asdict(lease),
            "input_ref": dataclasses.asdict(ref),
            "priority": 1,
        }
        for lease, ref in zip(leases, refs, strict=True)
    ]
    before, wal = queue.tasks, queue.journal.path.read_bytes()
    with pytest.raises(StaleAttempt):
        queue.yield_tasks(updates, request_id="return")
    assert queue.tasks == before
    assert queue.journal.path.read_bytes() == wal


@pytest.mark.parametrize("field,value", [("priority", True), ("priority", 2**63), ("scheduling_key", 1.5)])
def test_scheduling_keys_are_validated_before_submit(queue, field, value):
    with pytest.raises(ValueError, match="signed 64-bit integer"):
        queue.submit_tasks("bad", [TaskSpec("bad", **{field: value})])
    assert queue.pending_tasks() == []


@pytest.mark.parametrize("phase,committed", [("after_journal_part", False), ("after_journal_sync", True)])
def test_batched_yield_crash_recovers_all_inputs_and_order_or_none(queue, phase, committed):
    queue.submit_tasks("initial", [TaskSpec("a"), TaskSpec("b")])
    leases = [assignment.lease for assignment in queue.acquire("writer", 2).assignments]
    refs = [result(queue, lease, submission_id=f"prefix:{i}") for i, lease in enumerate(leases)]
    updates = [
        {
            "lease": dataclasses.asdict(lease),
            "input_ref": dataclasses.asdict(ref),
            "priority": 1,
            "scheduling_key": -i,
            "metadata": {"stage": "partial"},
        }
        for i, (lease, ref) in enumerate(zip(leases, refs, strict=True))
    ]

    def crash(current):
        if current == phase:
            raise OSError(errno.EIO, "injected yield crash")

    queue.store.backend.fault = crash
    with pytest.raises(IndeterminateCommit):
        queue.yield_tasks(updates, request_id="return")
    queue.store.backend.fault = lambda _: None
    restored = recover(queue)
    try:
        tasks = restored.pending_tasks()
        assert [task.task_id for task in tasks] == (["b", "a"] if committed else ["a", "b"])
        assert [task.input_ref for task in tasks] == (refs[::-1] if committed else [None, None])
        if committed:
            assert restored.yield_tasks(updates, request_id="return") == "pending"
        else:
            with pytest.raises(StaleAttempt):
                restored.yield_tasks(updates, request_id="return")
    finally:
        restored.close()
