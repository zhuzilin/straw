"""A complete file stays bound to its original identity across sync failures."""

import errno

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, IdempotencyConflict, IndeterminateCommit, StorageUnavailable


@pytest.mark.parametrize("api", ["write_records", "record_set", "publish"])
@pytest.mark.parametrize("target", [0, 1024 * 1024])
@pytest.mark.parametrize("failed_phase", ["before_file_sync", "after_data_sync"])
def test_complete_pending_file_is_reused_and_fences_other_requests(tmp_path, api, target, failed_phase):
    with SharedFilesystemStore(tmp_path, "pending", online_gc=True, segment_target_bytes=target) as store:
        payload = b"x" * 2048
        raw = store.write_records([Record("raw", payload)], submission_id="raw") if api == "record_set" else None

        def publish(identity="retry", changed=False):
            if api == "record_set":
                return [store.record_set([] if changed or identity == "other" else raw, submission_id=identity)]
            record = Record("r", b"changed" if changed else payload)
            if api == "write_records":
                return store.write_records([record], submission_id=identity)
            return [store.publish([record], submission_id=identity)]

        def fail(phase):
            if phase == failed_phase:
                raise OSError(errno.EIO, "uncertain data sync")

        store.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="uncertain data sync"):
            publish()
        pending = list(tmp_path.rglob("*.partial" if target == 0 else "*.pack"))
        assert len(pending) == 1
        length = pending[0].stat().st_size
        with pytest.raises(IndeterminateCommit):
            publish("other")
        with pytest.raises(IdempotencyConflict):
            publish(changed=True)
        with pytest.raises(IndeterminateCommit):
            store.seal()
        assert store.collect_garbage()["reclaimed_files"] == 0
        phases = []
        store.backend.fault = phases.append
        refs = publish()
        segment = refs[0].segment if api == "write_records" else refs[0].manifest.segment
        actual = tmp_path / segment.path
        assert actual.stem == pending[0].stem and actual.stat().st_size == length
        assert not list(tmp_path.rglob("*.partial"))
        assert "after_payload_chunk" not in phases
        assert phases.count("before_file_sync") == (1 if target or failed_phase == "before_file_sync" else 0)
        record = store.read_record(refs[0]) if api == "write_records" else next(store.read(refs[0]))
        assert record.payload == payload
        store.seal()
        store.release_publications(refs)
        assert store.collect_garbage()["reclaimed_files"] == (2 if api == "record_set" and target == 0 else 1)


@pytest.mark.parametrize("target", [0, 1024 * 1024])
@pytest.mark.parametrize("failed_phase", ["before_file_sync", "after_data_sync"])
def test_retry_authenticates_original_bytes_even_after_a_confirmed_sync(tmp_path, target, failed_phase):
    with SharedFilesystemStore(tmp_path, "pending", online_gc=True, segment_target_bytes=target) as store:

        def fail(phase):
            if phase == failed_phase:
                raise OSError(errno.EIO, "uncertain data sync")

        store.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            store.write_records([Record("r", b"x" * 2048)], submission_id="retry")
        (pending,) = tmp_path.rglob("*.partial" if target == 0 else "*.pack")
        with pending.open("r+b") as f:
            f.seek(pending.stat().st_size // 2)
            f.write(b"!")
        phases = []
        store.backend.fault = phases.append
        with pytest.raises(CorruptData):
            store.write_records([Record("r", b"x" * 2048)], submission_id="retry")
        assert "after_payload_chunk" not in phases
        assert store.collect_garbage()["reclaimed_files"] == 0
