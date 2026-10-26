"""Unreturned complete publications stay bound until their whole handoff ends."""

import errno

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import IdempotencyConflict, IndeterminateCommit, InvalidReference, StorageUnavailable

PACKED = 64 * 1024 * 1024


def make(root, target=0):
    return SharedFilesystemStore(root, "handoff", online_gc=True, segment_target_bytes=target)


def invoke(writer, api, raw, dependency, identity="first", changed=False):
    if api == "record_set":
        return [writer.record_set([] if changed else raw, submission_id=identity)]
    records = [Record("changed" if changed else identity, b"x" * 2048)]
    if api == "write_records":
        return writer.write_records(records, submission_id=identity)
    return [writer.publish(records, submission_id=identity, dependencies=[dependency])]


CASES = (
    [
        (api, target, phase, 1)
        for api in ("write_records", "publish", "record_set")
        for target in (0, PACKED)
        for phase in ("after_data_sync", "before_storage_catalog_sync", "after_storage_catalog_sync")
    ]
    + [
        (api, 0, phase, 1)
        for api in ("write_records", "publish", "record_set")
        for phase in ("before_publication_seal", "after_publication_seal")
    ]
    + [
        ("record_set", target, phase, occurrence)
        for target in (0, PACKED)
        for phase in ("before_storage_catalog_sync", "after_storage_catalog_sync")
        for occurrence in ((3, 4, 5) if target == 0 else (2, 3, 4))
    ]
    + [
        ("publish", target, phase, 3 if target == 0 else 2)
        for target in (0, PACKED)
        for phase in ("before_storage_catalog_sync", "after_storage_catalog_sync")
    ]
)


@pytest.mark.parametrize("api,target,phase,occurrence", CASES)
def test_handoff_errors_fence_other_requests_without_losing_original_bytes(tmp_path, api, target, phase, occurrence):
    with make(tmp_path) as source, make(tmp_path, target) as writer:
        raw = source.write_records([Record("raw", b"x" * 2048)], submission_id="raw")
        dependency = source.publish([Record("dependency", b"dep")], submission_id="dependency")
        warm = writer.write_records([Record("warm", b"warm")], submission_id="warm")
        armed, fired, seen = False, False, 0

        def fail(at):
            nonlocal armed, fired, seen
            if at == "after_data_sync":
                armed = True
            if armed and at == phase:
                seen += 1
                if seen == occurrence and not fired:
                    fired = True
                    raise OSError(errno.EIO, "handoff response lost")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="handoff response lost"):
            invoke(writer, api, raw, dependency)
        assert fired
        writer.backend.fault = lambda _: None
        files = {p: p.read_bytes() for p in tmp_path.rglob("*") if p.suffix in (".partial", ".pack", ".sealed")}
        catalog = (tmp_path / "storage.log").read_bytes()
        for other_api in ("write_records", "publish", "record_set"):
            with pytest.raises(IndeterminateCommit):
                invoke(writer, other_api, raw, dependency, "other")
            if other_api != api:
                with pytest.raises(IndeterminateCommit):
                    invoke(writer, other_api, raw, dependency)
        # A formerly acknowledged request must not bypass a newer pending one.
        with pytest.raises(IndeterminateCommit):
            writer.write_records([Record("warm", b"warm")], submission_id="warm")
        assert (tmp_path / "storage.log").read_bytes() == catalog
        with pytest.raises(IdempotencyConflict):
            invoke(writer, api, raw, dependency, changed=True)
        with pytest.raises(IndeterminateCommit):
            writer.seal()
        assert all(p.read_bytes() == value for p, value in files.items())
        assert source.collect_garbage()["reclaimed_files"] == 0
        phases = []
        writer.backend.fault = phases.append
        first = invoke(writer, api, raw, dependency)
        assert "after_payload_chunk" not in phases
        assert all(
            (p.with_suffix(".sealed") if p.suffix == ".partial" else p).read_bytes() == value
            for p, value in files.items()
        )
        result = writer.read_record(first[0]) if api == "write_records" else next(writer.read(first[0]))
        assert result.payload == b"x" * 2048
        other = invoke(writer, api, raw, dependency, "other")
        writer.release_publications([*first, *other, *warm])
        source.release_publications([*raw, dependency])
        writer.seal()
        gc = source.collect_garbage()
        assert gc["owners"] == gc["open_packs"] == 0
        assert not [p for p in tmp_path.rglob("*") if p.suffix in (".partial", ".pack", ".sealed")]


def test_replay_of_acknowledged_reclaimed_record_does_not_fence_a_new_request(tmp_path):
    with make(tmp_path) as writer:
        first = writer.write_records([Record("first", b"first")], submission_id="first")
        writer.release_publications(first)
        assert writer.collect_garbage()["reclaimed_files"] == 1
        with pytest.raises(InvalidReference):
            writer.write_records([Record("first", b"first")], submission_id="first")
        other = writer.write_records([Record("other", b"other")], submission_id="other")
        writer.release_publications(other)
        assert writer.collect_garbage()["owners"] == 0


@pytest.mark.parametrize("api", ["publish", "record_set"])
def test_closed_writer_rejects_before_creating_a_dependency_hold(tmp_path, api):
    with make(tmp_path) as source:
        raw = source.write_records([Record("raw", b"raw")], submission_id="raw")
        dep = source.publish([Record("dep", b"dep")], submission_id="dep")
        writer = make(tmp_path)
        writer.close()
        catalog = (tmp_path / "storage.log").read_bytes()
        with pytest.raises(RuntimeError):
            invoke(writer, api, raw, dep)
        assert (tmp_path / "storage.log").read_bytes() == catalog
        source.release_publications([*raw, dep])
        assert source.collect_garbage()["owners"] == 0
