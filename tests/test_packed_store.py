import errno
import hashlib
from dataclasses import replace

import pytest

from straw import Publication, Record, SharedFilesystemStore
from straw.backend import FilesystemBackend
from straw.errors import (
    CorruptData,
    IndeterminateCommit,
    InvalidReference,
    StorageUnavailable,
)


def test_sparse_samples_share_a_file_and_old_extents_stay_immutable(tmp_path):
    store = SharedFilesystemStore(tmp_path, "run", segment_target_bytes=1024**2)
    refs = [store.publish([Record(f"sample-{i}", bytes([i]) * 1024)], submission_id=str(i)) for i in range(100)]
    assert len(list(tmp_path.rglob("*.pack"))) == 1
    assert len(list(tmp_path.rglob("*.*"))) == 1
    reader = SharedFilesystemStore(tmp_path, "run")
    for i, ref in enumerate(refs):
        assert next(reader.read(ref)).payload == bytes([i]) * 1024
        reader.inspect_segment(ref.manifest.segment, verify_payload=True)


def test_packed_dependencies_and_physical_layout_do_not_change_digest(tmp_path):
    store = SharedFilesystemStore(tmp_path, "run", segment_target_bytes=1024**2)
    blob = Record("blob", b"payload")
    root = Record("sample", b"metadata")
    first = store.publish_many([Publication((blob,)), Publication((root,), (0,))], submission_id="a")
    second = store.publish_many(
        [
            Publication((Record("unrelated", b"x"),)),
            Publication((blob,)),
            Publication((root,), (1,)),
        ],
        submission_id="b",
    )
    assert first[1].digest == second[2].digest
    assert first[0].manifest.segment == first[1].manifest.segment
    assert next(store.read(first[1])).payload == b"metadata"
    assert first[1].payload_bytes == len(b"payloadmetadata")
    assert store.manifest(first[1])["dependencies"][0]["manifest"]["segment"]["offset"] == 0


def test_torn_tail_is_unreachable_after_new_writer_starts(tmp_path):
    store = SharedFilesystemStore(tmp_path, "run", segment_target_bytes=1024**2)
    good = store.publish([Record("good", b"committed")], submission_id="good")

    def crash(phase):
        if phase == "after_record":
            raise RuntimeError("process terminated while appending")

    store.backend.fault = crash
    with pytest.raises(RuntimeError):
        store.publish([Record("bad", b"incomplete")], submission_id="bad")
    restarted = SharedFilesystemStore(tmp_path, "run", segment_target_bytes=1024**2)
    later = restarted.publish([Record("later", b"new incarnation")], submission_id="later")
    assert good.manifest.segment.path != later.manifest.segment.path
    assert next(restarted.read(good)).payload == b"committed"
    assert next(restarted.read(later)).payload == b"new incarnation"
    restarted.inspect_segment(good.manifest.segment, verify_payload=True)


def test_append_uncertain_sync_fences_writer_until_same_identity_is_resolved(tmp_path):
    backend = FilesystemBackend(tmp_path)
    store = SharedFilesystemStore(tmp_path, "run", backend=backend, segment_target_bytes=1024**2)
    first = store.publish([Record("a", b"a")], submission_id="a")

    def fail(phase):
        if phase == "before_file_sync":
            raise OSError(errno.EIO, "sync outcome unknown")

    backend.fault = fail
    with pytest.raises(StorageUnavailable):
        store.publish([Record("b", b"b")], submission_id="b")
    with pytest.raises(IndeterminateCommit):
        store.publish([Record("c", b"c")], submission_id="c")
    backend.fault = lambda _: None
    retried = store.publish([Record("b", b"b")], submission_id="b")
    assert retried.manifest.segment.offset == first.manifest.segment.size
    assert next(store.read(retried)).payload == b"b"
    assert len(list(tmp_path.rglob("*.pack"))) == 1


def test_rotation_and_range_reads_at_nonzero_offsets(tmp_path):
    store = SharedFilesystemStore(tmp_path, "run", segment_target_bytes=16000)
    refs = [store.publish([Record(str(i), bytes([i]) * 4000)], submission_id=str(i)) for i in range(6)]
    assert 1 < len(list(tmp_path.rglob("*.pack"))) < 6
    member = store.validate(refs[1])[0]
    assert member.segment.offset > 0
    payload = bytes([1]) * 4000
    checksums = [hashlib.sha256(payload[i : i + 1024]).hexdigest() for i in range(0, 4000, 1024)]
    assert store.read_range(member, 1100, 2400, chunk_bytes=1024, checksums=checksums) == payload[1100:2400]
    with store.backend.path(member.segment.path).open("r+b") as f:
        entry = store.inspect_segment(member.segment)["records"][member.ordinal]
        f.seek(
            member.segment.offset
            + entry["offset"]
            + len(__import__("straw.protocol", fromlist=["encode"]).encode(entry["envelope"]))
            + 12
            + 2000
        )
        f.write(b"X")
    with pytest.raises(CorruptData):
        store.read_range(member, 1100, 2400, chunk_bytes=1024, checksums=checksums)
    assert next(store.read(refs[0])).payload == bytes(4000)


def test_local_dependency_must_precede_manifest(tmp_path):
    store = SharedFilesystemStore(tmp_path, "run")
    with pytest.raises(InvalidReference):
        store.publish_many([Publication((Record("x", b"x"),), (0,))], submission_id="bad")
    assert not list(tmp_path.rglob("*.sealed"))


def test_external_manifest_uses_native_float_canonicalization(tmp_path):
    store = SharedFilesystemStore(tmp_path, "float", segment_target_bytes=1024**2)
    refs = store.write_records([Record("small", b"x", metadata={"p": 1e-7, "large": 1e20})], submission_id="bytes")
    subset = store.record_set(refs, submission_id="subset")
    assert list(store.read(subset))[0].metadata == {"p": 1e-7, "large": 1e20}


def test_acquire_obeys_requested_page_without_legacy_metadata_budget(tmp_path):
    from straw import TaskSpec
    from straw.coordinator import Coordinator
    from straw.protocol import Limits

    q = Coordinator(
        SharedFilesystemStore(tmp_path, "pages"),
        exclusive_owner="test",
        limits=Limits(inflight_tasks=128, metadata_bytes=8192),
    )
    try:
        for i in range(32):
            q.submit_tasks(f"submit-{i}", [TaskSpec(str(i), metadata={"description": "x" * 1024})])
        first = q.acquire("worker", max_tasks=32)
        assert len(first.assignments) == 32
        assert q.acquire("worker", max_tasks=32).status == "empty"
    finally:
        q.close()


def test_incremental_accounting_matches_task_and_commit_state(tmp_path):
    from straw import TaskSpec
    from straw.coordinator import Coordinator

    clock = [1.0]
    q = Coordinator(SharedFilesystemStore(tmp_path, "accounting"), exclusive_owner="test", clock=lambda: clock[0])

    def check():
        active = [t["spec"] for t in q.tasks.values() if t["state"] == "leased"]
        usage = q._usage()
        assert usage["inflight"] == len(active)
        for key, field in (("records", "records"), ("bytes", "payload_bytes"), ("tokens", "tokens")):
            accepted = sum(c["result_ref"][field] for c in q.commits)
            assert usage["accepted_unprocessed"][key] == accepted
            assert usage[key] == accepted + sum(t[f"estimated_{key}"] for t in active)

    try:
        q.submit_tasks("submit", [TaskSpec(str(i), estimated_bytes=10, estimated_tokens=3) for i in range(20)])
        check()
        leases = [a.lease for a in q.acquire("worker", 20).assignments]
        check()
        for i, lease in enumerate(leases[:10]):
            if i % 2:
                q.fail_task(lease, request_id=f"fail-{i}", failure={"reason": "test"})
            else:
                result = q.store.publish(
                    [
                        Record(
                            str(i),
                            b"result",
                            metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
                            tokens=2,
                        )
                    ],
                    submission_id=f"payload-{i}",
                )
                q.complete_task(lease, submission_id=f"complete-{i}", result_ref=result, result_digest=result.digest)
            check()
        q.cancel_task(leases[10].task_id, request_id="cancel")
        check()
        clock[0] = 1000
        q.acquire("new-worker", 4)
        check()
    finally:
        q.close()


def test_batch_release_is_durable_idempotent_and_does_not_spend_retries(tmp_path):
    from straw import TaskSpec
    from straw.coordinator import Coordinator

    q = Coordinator(SharedFilesystemStore(tmp_path, "release"), exclusive_owner="test")
    q.submit_tasks("submit", [TaskSpec(str(i)) for i in range(40)])
    leases = [a.lease for a in q.acquire("worker", 40).assignments]
    before = q.journal.sequence
    assert q.release_tasks(leases, request_id="release") == ["released"] * 40
    assert q.journal.sequence == before + 1
    q.close()
    q = Coordinator(q.store, exclusive_owner="old owner stopped", recover=True)
    try:
        assert q.release_tasks(leases, request_id="release") == ["released"] * 40
        assert len(q.acquire("new-worker", 40).assignments) == 40
        assert all(task["failures"] == 0 for task in q.tasks.values())
    finally:
        q.close()


def test_progress_batch_validates_all_leases_before_publishing_state(tmp_path):
    from dataclasses import asdict

    from straw import TaskSpec
    from straw.coordinator import Coordinator
    from straw.errors import StaleAttempt

    q = Coordinator(SharedFilesystemStore(tmp_path, "progress"), exclusive_owner="test")
    try:
        q.submit_tasks("tasks", [TaskSpec("a"), TaskSpec("b")])
        leases = [a.lease for a in q.acquire("worker", 2).assignments]
        ref = q.store.publish([Record("prefix", b"partial")], submission_id="prefix")
        updates = [{"lease": asdict(lease), "input_ref": asdict(ref)} for lease in leases]
        q.cancel_task("b", request_id="cancel")
        with pytest.raises(StaleAttempt):
            q.save_task_progress_many(updates, request_id="failed")
        assert q.task_status("a")["spec"]["input_ref"] is None
        before = q.journal.sequence
        assert q.save_task_progress_many(updates[:1], request_id="progress") == "saved"
        assert q.journal.sequence == before + 1
        assert q.save_task_progress_many(updates[:1], request_id="progress") == "saved"
        assert q.journal.sequence == before + 1
        assert q.task_status("a")["spec"]["input_ref"] == asdict(ref)
    finally:
        q.close()


@pytest.mark.parametrize("ordinal", [-1, "0", None])
def test_malformed_subset_ordinal_cannot_alias_first_record(tmp_path, ordinal):
    store = SharedFilesystemStore(tmp_path, "bounds")
    (ref,) = store.write_records([Record("r", b"payload")], submission_id="record")
    with pytest.raises(InvalidReference):
        store.record_set([replace(ref, ordinal=ordinal)], submission_id="invalid")
    with pytest.raises(InvalidReference):
        store.inspect_segment(replace(ref.segment, offset=-1))


def test_native_discovery_validates_sealed_candidates_and_ignores_append_packs(tmp_path):
    with SharedFilesystemStore(tmp_path, "discovery", segment_target_bytes=0) as standalone:
        (record,) = standalone.write_records([Record("sealed", b"sealed")], submission_id="sealed")
    with SharedFilesystemStore(tmp_path, "discovery") as packed:
        packed.publish([Record("packed", b"packed")], submission_id="packed")
        candidates = [ref for partition in (tmp_path / "raw").iterdir() for ref in packed.discover(partition.name)]
        assert candidates == [record.segment]
        with pytest.raises(InvalidReference):
            list(packed.discover("../escape"))
        path = packed.backend.path(record.segment.path)
        with path.open("r+b") as stream:
            stream.seek(-1, 2)
            stream.write(b"X")
        with pytest.raises(CorruptData):
            list(packed.discover(record.segment.path.split("/")[1]))
