"""Checkpoint rewind must re-own history before publishing a live consumer cursor."""

import json
import struct
import subprocess
import sys
from dataclasses import asdict

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import CorruptData, InvalidReference, StorageUnavailable, UnsafeRecovery


def setup_history(root, count=1, *, checkpoint=True, seal=True, payload=None, indirect=False):
    store = SharedFilesystemStore(root, "rewind", online_gc=True, codecs=("bytes.v1", "json.v1"))
    queue = Coordinator(store, exclusive_owner="test owner")
    queue.submit_tasks("inputs", [TaskSpec(str(i)) for i in range(count)])
    receipts = []
    for i in range(count):
        lease = queue.acquire("worker").assignments[0].lease
        data = f"payload-{i}".encode() if payload is None else payload
        dependencies = []
        if indirect:
            dependencies.append(store.publish([Record(f"dep-{i}", data)], submission_id=f"dep-{i}"))
        ref = store.publish(
            [
                Record(
                    str(i),
                    b"parent" if indirect else data,
                    metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
                )
            ],
            submission_id=f"result-{i}",
            dependencies=dependencies,
        )
        if dependencies:
            store.release_publications(dependencies)
        receipts.append(
            queue.complete_task(lease, submission_id=f"accepted-{i}", result_ref=ref, result_digest=ref.digest)
        )
    if seal:
        store.seal()
    # Different writer/pack lets unprotected open result packs remain physical.
    states = SharedFilesystemStore(root, "rewind", online_gc=True, codecs=("bytes.v1", "json.v1"))
    state = states.publish([Record("state", b"state")], submission_id="state")
    states.retain("test-state", [state])
    states.release_publications([state])
    if checkpoint:
        saved = states.publish(
            [Record("checkpoint", b"saved")], submission_id="checkpoint", dependencies=[r.result_ref for r in receipts]
        )
        states.retain("checkpoint", [saved])
        states.release_publications([saved])
    token = queue.open_consumer("training", exclusive_owner="test trainer")
    return store, states, queue, token, state, receipts


def progress(store, name, *, positions=(), finished=()):
    return store.publish(
        [
            Record(
                name,
                json.dumps(
                    dict(version=1, processed_positions=list(positions), finished_batches=list(finished))
                ).encode(),
                codec="json.v1",
            )
        ],
        submission_id=name,
    )


def save(queue, token, state, name, fetched, processed, progress_ref=None):
    return queue.save_consumer_state(
        "training",
        token=token,
        request_id=name,
        state_ref=state,
        fetch_cursor=fetched,
        processed_cursor=processed,
        progress_ref=progress_ref,
    )


@pytest.mark.parametrize("kind", ["prefix", "sparse"])
def test_rewind_reowns_checkpoint_results_and_immediately_restores_replay(tmp_path, kind):
    store, states, queue, token, state, receipts = setup_history(tmp_path, 3)
    old = progress(states, "old", positions=[0, 2]) if kind == "sparse" else None
    save(queue, token, state, "advanced", 3, 0 if old else 3, old)
    queue.collect_garbage()
    with pytest.raises(InvalidReference, match="released"):
        queue.read_commits(0, 1)
    restored = progress(states, "restored", positions=[2]) if old else None
    save(queue, token, state, "restore", 3, 0, restored)
    assert queue.read_commits(0, 1).commits == (receipts[0],)
    if old:
        with pytest.raises(InvalidReference, match="released"):
            queue.read_commits(2, 1)
    # Explicitly saving the same restored state must also be valid.
    save(queue, token, state, "same-state-restore", 3, 0, restored)
    states.release("checkpoint")
    queue.collect_garbage()
    assert next(store.read(receipts[0].result_ref)).payload == b"payload-0"
    assert queue.lookup_submission("accepted-0") == receipts[0]
    queue.close()
    recovered = Coordinator(store, exclusive_owner="replacement", recover=True)
    assert recovered.read_commits(0, 1).commits == (receipts[0],)
    recovered.collect_garbage()
    recovered.close()


@pytest.mark.skipif(sys.platform != "linux", reason="Uses Linux address-space accounting")
@pytest.mark.parametrize("indirect", [False, True])
def test_rewind_verifies_large_reachable_payload_with_bounded_scratch(tmp_path, indirect):
    store, states, queue, token, state, receipts = setup_history(
        tmp_path, payload=b"x" * (64 * 1024**2), indirect=indirect
    )
    save(queue, token, state, "advanced", 1, 1)
    queue.collect_garbage()
    queue.close()
    # A fresh process must not allocate the whole 64 MiB payload just to check
    # it. Disable core dumps so the historical allocator abort is contained.
    script = """
import json, resource, sys
from pathlib import Path
from straw import SharedFilesystemStore
from straw.coordinator import Coordinator
from straw.protocol import RecordSetRef
resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
store = SharedFilesystemStore(sys.argv[1], 'rewind', online_gc=True)
queue = Coordinator(store, exclusive_owner='memory test', recover=True)
token = queue.open_consumer('training', exclusive_owner='memory trainer')
state = RecordSetRef.from_dict(json.loads(sys.argv[2]))
status = Path('/proc/self/status').read_text().splitlines()
size = int(next(line.split()[1] for line in status if line.startswith('VmSize:'))) * 1024
resource.setrlimit(resource.RLIMIT_AS, (size + 32 * 1024**2, size + 32 * 1024**2))
print('rewind_started', file=sys.stderr, flush=True)
queue.save_consumer_state('training', token=token, request_id='restore', state_ref=state,
                          fetch_cursor=1, processed_cursor=0)
assert len(queue.read_commits(0, 1).commits) == 1
store.release('checkpoint')
queue.collect_garbage()
assert len(queue.read_commits(0, 1).commits) == 1
queue.close()
"""
    result = subprocess.run(
        [sys.executable, "-I", "-c", script, str(tmp_path), json.dumps(asdict(state))],
        capture_output=True,
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, result.stderr
    recovered = Coordinator(store, exclusive_owner="after memory test", recover=True)
    assert recovered.read_commits(0, 1).commits == (receipts[0],)
    recovered.close()
    states.close()
    store.close()


def test_finished_batch_rewind_reowns_plan_and_ready_roots(tmp_path):
    store, states, queue, token, state, _ = setup_history(tmp_path)
    plan = states.publish([Record("plan", b"plan")], submission_id="plan")
    ready = states.publish([Record("ready", b"ready")], submission_id="ready")
    states.retain("saved-batch", [plan, ready])
    queue.plan_batch("training", token=token, batch_id="batch", input_positions=[0], plan_ref=plan)
    queue.batch_ready(
        "training", token=token, batch_id="batch", ready_ref=ready, state_ref=state, fetch_cursor=1, processed_cursor=1
    )
    done = progress(states, "done", finished=["batch"])
    save(queue, token, state, "finished", 1, 1, done)
    queue.collect_garbage()
    save(queue, token, state, "restore", 1, 1)
    states.release("saved-batch")
    queue.collect_garbage()
    assert next(store.read(plan)).payload == b"plan"
    assert next(store.read(ready)).payload == b"ready"
    queue.close()


@pytest.mark.parametrize("indirect", [False, True])
def test_rewind_checks_tail_of_every_payload_before_any_owner_or_wal_update(tmp_path, indirect):
    payload = b"x" * (2 * 1024**2 + 7)
    store, states, queue, token, state, receipts = setup_history(tmp_path, 2, payload=payload, indirect=indirect)
    save(queue, token, state, "advanced", 2, 2)
    queue.collect_garbage()
    reference = receipts[1].result_ref
    if indirect:
        from straw.protocol import RecordSetRef

        reference = RecordSetRef.from_dict(store.manifest(reference)["dependencies"][0])
    record = store.manifest(reference)["records"][0]
    entry = store.inspect_segment(reference.manifest.segment)["records"][record["ordinal"]]
    path = tmp_path / reference.manifest.segment.path
    with path.open("r+b") as stream:
        offset = reference.manifest.segment.offset + entry["offset"]
        stream.seek(offset)
        meta_bytes, length = struct.unpack("<IQ", stream.read(12))
        assert length == len(payload)
        tail = offset + 12 + meta_bytes + length - 1
        stream.seek(tail)
        stream.write(b"y")
    before = queue.journal.path.read_bytes(), (tmp_path / "storage.log").read_bytes()
    with pytest.raises(CorruptData, match="payload checksum"):
        save(queue, token, state, "restore", 2, 0)
    assert (queue.journal.path.read_bytes(), (tmp_path / "storage.log").read_bytes()) == before
    assert queue.load_consumer_state("training")["processed_cursor"] == 2
    with path.open("r+b") as stream:
        stream.seek(tail)
        stream.write(b"x")
    save(queue, token, state, "retry-restore", 2, 0)
    states.release("checkpoint")
    queue.collect_garbage()
    assert queue.read_commits(0, 2).commits == tuple(receipts)
    queue.close()
    states.close()
    store.close()


@pytest.mark.parametrize("phase", ["before_reactivation_verify", "after_reactivation_verify"])
@pytest.mark.parametrize("action", ["release", "collect", "replace", "unrelated"])
def test_rewind_rechecks_ownership_after_unlocked_verification(tmp_path, phase, action):
    store, states, queue, token, state, receipts = setup_history(tmp_path)
    save(queue, token, state, "advanced", 1, 1)
    queue.collect_garbage()
    wal = queue.journal.path.read_bytes()
    observations = []
    peer = """
import json, sys
from straw import Record, SharedFilesystemStore
from straw.protocol import RecordSetRef
s = SharedFilesystemStore(sys.argv[1], 'rewind', online_gc=True)
action = sys.argv[2]
if action == 'replace':
    s.retain('replacement-checkpoint', [RecordSetRef.from_dict(json.loads(sys.argv[3]))])
if action != 'unrelated':
    s.release('checkpoint')
if action == 'collect':
    assert s.collect_garbage()['reclaimed_files'] == 1
if action == 'unrelated':
    ref = s.publish([Record('peer', b'peer')], submission_id='peer')
    s.seal()
    s.release_publications([ref])
    assert s.collect_garbage()['reclaimed_files'] == 1
s.close()
"""

    def interleave(at):
        if at != phase:
            return
        # A separate process must finish catalog writes/GC while this rewind
        # is stopped at the scan boundary. A held lock would time out here.
        result = subprocess.run(
            [sys.executable, "-I", "-c", peer, str(tmp_path), action, json.dumps(asdict(receipts[0].result_ref))],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0, result.stderr
        observations.append((tmp_path / "storage.log").read_bytes())

    store.backend.fault = interleave
    if action in ("release", "collect"):
        with pytest.raises((InvalidReference, UnsafeRecovery, StorageUnavailable)):
            save(queue, token, state, "restore", 1, 0)
        assert len(observations) == 1
        assert (tmp_path / "storage.log").read_bytes() == observations[0]
        assert queue.journal.path.read_bytes() == wal
    else:
        save(queue, token, state, "restore", 1, 0)
        assert len(observations) == 1
        states.release("replacement-checkpoint")
        queue.collect_garbage()
        assert queue.read_commits(0, 1).commits == (receipts[0],)
    queue.close()
    fresh = SharedFilesystemStore(tmp_path, "rewind", online_gc=True)
    recovered = Coordinator(fresh, exclusive_owner="after scan race", recover=True)
    assert recovered.load_consumer_state("training")["processed_cursor"] == (
        1 if action in ("release", "collect") else 0
    )
    recovered.close()
    fresh.close()
    states.close()
    store.close()


@pytest.mark.parametrize("damage", ["reclaimed", "unprotected", "missing", "corrupt"])
def test_rewind_rejects_unavailable_history_before_any_wal_or_owner_change(tmp_path, damage):
    store, states, queue, token, state, receipts = setup_history(
        tmp_path, checkpoint=damage in ("missing", "corrupt"), seal=damage != "unprotected"
    )
    save(queue, token, state, "advanced", 1, 1)
    queue.collect_garbage()
    ref = receipts[0].result_ref
    path = tmp_path / ref.manifest.segment.path
    if damage == "missing":
        path.unlink()
    elif damage == "corrupt":
        record = store.manifest(ref)["records"][0]
        entry = store.inspect_segment(ref.manifest.segment)["records"][record["ordinal"]]
        offset = ref.manifest.segment.offset + entry["offset"]
        with path.open("r+b") as stream:
            stream.seek(offset)
            meta_bytes, _ = struct.unpack("<IQ", stream.read(12))
            stream.seek(offset + 12 + meta_bytes)
            byte = stream.read(1)
            stream.seek(-1, 1)
            stream.write(bytes([byte[0] ^ 1]))
    before = queue.journal.path.read_bytes(), (tmp_path / "storage.log").read_bytes()
    with pytest.raises((InvalidReference, UnsafeRecovery, StorageUnavailable, CorruptData)):
        save(queue, token, state, "restore", 1, 0)
    assert queue.journal.path.read_bytes() == before[0]
    assert (tmp_path / "storage.log").read_bytes() == before[1]
    assert queue.load_consumer_state("training")["processed_cursor"] == 1
    queue.close()


@pytest.mark.parametrize(
    "phase,committed",
    [
        ("before_reactivation_verify", False),
        ("after_reactivation_verify", False),
        ("before_storage_catalog_write", False),
        ("after_storage_catalog_sync", False),
        ("before_journal_write", False),
        ("after_journal_part", False),
        ("after_journal_sync", True),
    ],
)
def test_rewind_crash_windows_keep_owner_before_restored_cursor(tmp_path, phase, committed):
    store, states, queue, token, state, receipts = setup_history(tmp_path)
    save(queue, token, state, "advanced", 1, 1)
    queue.collect_garbage()

    def crash(at):
        if at == phase:
            raise RuntimeError("injected loss")

    store.backend.fault = crash
    with pytest.raises(RuntimeError, match="injected loss"):
        save(queue, token, state, "restore", 1, 0)
    queue.close()
    fresh = SharedFilesystemStore(tmp_path, "rewind", online_gc=True, codecs=("bytes.v1", "json.v1"))
    recovered = Coordinator(fresh, exclusive_owner="replacement", recover=True)
    assert recovered.load_consumer_state("training")["processed_cursor"] == (0 if committed else 1)
    recovered.collect_garbage()
    token = recovered.open_consumer("training", exclusive_owner="replacement trainer")
    save(recovered, token, state, "retry-restore", 1, 0)
    assert recovered.read_commits(0, 1).commits == (receipts[0],)
    states.release("checkpoint")
    recovered.collect_garbage()
    assert next(fresh.read(receipts[0].result_ref)).payload == b"payload-0"
    recovered.close()
