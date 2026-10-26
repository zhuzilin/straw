"""Unexposed failed preparations cannot leave permanent dependency owners."""

import errno

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import IndeterminateCommit, InvalidReference, StorageUnavailable, UnsupportedSchema

PACKED = 64 * 1024 * 1024


def store(root, target=0):
    return SharedFilesystemStore(root, "provisional", online_gc=True, segment_target_bytes=target)


def inputs(source):
    raw = source.write_records([Record("raw", b"x" * 2048)], submission_id="raw")
    dep = source.publish([Record("dep", b"y" * 2048)], submission_id="dep")
    return raw, dep


def invoke(writer, api, raw, dep, invalid=False, identity="result"):
    if api == "record_set":
        return [writer.record_set([*raw, *raw] if invalid else raw, submission_id=identity)]
    return [
        writer.publish(
            [Record(identity, b"z" * 2048, codec="unsupported" if invalid else "bytes.v1")],
            submission_id=identity,
            dependencies=[dep],
        )
    ]


def retire(source, writer, raw, dep, results=()):
    source.release_publications([*raw, dep, *results])
    writer.close()
    source.close()
    with store(source.backend.root) as peer:
        gc = peer.collect_garbage()
        assert gc["owners"] == gc["open_packs"] == 0
    assert not [p for p in source.backend.root.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
def test_close_releases_failed_validation_preparation_hold(tmp_path, api, target):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        raw, dep = inputs(source)
        with pytest.raises((UnsupportedSchema, InvalidReference)):
            invoke(writer, api, raw, dep, invalid=True)
        retire(source, writer, raw, dep)


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize(
    "phase", ["after_storage_catalog_sync", "after_segment_header", "after_payload_chunk", "before_footer"]
)
def test_early_io_failure_releases_hold_and_allows_another_request(tmp_path, api, target, phase):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        raw, dep = inputs(source)
        fired = False

        def fail(at):
            nonlocal fired
            if at == phase and not fired:
                fired = True
                raise OSError(errno.EIO, "preparation interrupted")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="preparation interrupted"):
            invoke(writer, api, raw, dep)
        assert fired
        writer.backend.fault = lambda _: None
        result = invoke(writer, api, raw, dep, identity="different")
        assert next(writer.read(result[0])).payload == (b"x" * 2048 if api == "record_set" else b"z" * 2048)
        retire(source, writer, raw, dep, result)


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize("cut", ["before_provisional_release", "after_provisional_release"])
@pytest.mark.parametrize("next_action", ["write_records", "publish", "record_set", "seal", "close"])
def test_failed_cleanup_is_remembered_until_next_mutating_operation(tmp_path, api, target, cut, next_action):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        raw, dep = inputs(source)

        def fail(at):
            if at == cut:
                raise OSError(errno.EIO, "provisional cleanup interrupted")

        with pytest.raises((UnsupportedSchema, InvalidReference)):
            invoke(writer, api, raw, dep, invalid=True)
        writer.backend.fault = fail
        with pytest.raises(IndeterminateCommit, match="Preparation owner cleanup"):
            writer.seal()
        writer.backend.fault = lambda _: None
        results = []
        if next_action in ("seal", "close"):
            getattr(writer, next_action)()
        elif next_action == "write_records":
            results = writer.write_records([Record("other", b"other")], submission_id="other")
        else:
            results = invoke(writer, next_action, raw, dep, identity="other")
        retire(source, writer, raw, dep, results)


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
def test_cleanup_error_cannot_be_overwritten_by_another_hold(tmp_path, api, target):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        raw, dep = inputs(source)

        def fail(at):
            if at == "before_provisional_release":
                raise OSError(errno.EIO, "cleanup blocked")

        with pytest.raises((UnsupportedSchema, InvalidReference)):
            invoke(writer, api, raw, dep, invalid=True)
        writer.backend.fault = fail
        with pytest.raises(IndeterminateCommit):
            writer.seal()
        before = (tmp_path / "storage.log").read_bytes()
        for other in ("publish", "record_set"):
            with pytest.raises(IndeterminateCommit):
                invoke(writer, other, raw, dep, identity="other")
        assert (tmp_path / "storage.log").read_bytes() == before
        writer.backend.fault = lambda _: None
        retire(source, writer, raw, dep)


def test_rotation_preserves_preparation_dependency_owner(tmp_path):
    with store(tmp_path) as source, store(tmp_path, 1) as writer:
        raw, dep = inputs(source)
        warm = writer.write_records([Record("warm", b"warm")], submission_id="warm")
        released = False

        def inspect(at):
            nonlocal released
            if at == "after_segment_header" and not released:
                released = True
                source.release_publications([*raw, dep])
                assert source.collect_garbage()["reclaimed_files"] == 1  # unrelated raw only

        writer.backend.fault = inspect
        result = invoke(writer, "publish", raw, dep)
        assert released and next(writer.read(result[0])).payload == b"z" * 2048
        writer.backend.fault = lambda _: None
        retire(source, writer, raw, dep, [*result, *warm])


@pytest.mark.parametrize("api", ["publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize("cut", ["after_storage_catalog_sync", "after_payload_chunk", "before_footer"])
def test_failed_preparation_remains_retryable_after_source_release(tmp_path, api, target, cut):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        raw, dep = inputs(source)
        fired = False

        def fail(at):
            nonlocal fired
            if at == cut and not fired:
                fired = True
                raise OSError(errno.EIO, "retryable preparation")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            invoke(writer, api, raw, dep)
        assert fired
        writer.backend.fault = lambda _: None
        source.release_publications([*raw, dep])
        source.seal()
        source.collect_garbage()
        result = invoke(writer, api, raw, dep)
        assert next(writer.read(result[0])).payload == (b"x" * 2048 if api == "record_set" else b"z" * 2048)
        retire(source, writer, raw, dep, result)


@pytest.mark.parametrize("target", [0, PACKED])
def test_multiple_failed_requests_keep_distinct_owners_until_close(tmp_path, target):
    with store(tmp_path, target) as source, store(tmp_path, target) as writer:
        deps = [source.publish([Record(str(i), b"keep")], submission_id=str(i)) for i in range(3)]
        for i, dep in enumerate(deps):
            with pytest.raises(UnsupportedSchema):
                writer.publish([Record("bad", b"x", codec="unsupported")], submission_id=str(i), dependencies=[dep])
        source.release_publications(deps)
        source.seal()
        assert source.collect_garbage()["reclaimed_files"] == 0
        assert all(next(source.read(dep)).payload == b"keep" for dep in deps)
        result = writer.publish([Record("good", b"good")], submission_id="0", dependencies=deps[:1])
        writer.close()
        source.collect_garbage()
        assert next(source.read(deps[0])).payload == b"keep"
        source.release_publications([result])
        assert source.collect_garbage()["owners"] == 0


@pytest.mark.parametrize("target", [0, PACKED])
def test_close_cleans_old_preparations_but_keeps_complete_uncertain_handoff(tmp_path, target):
    with store(tmp_path) as source:
        old = source.publish([Record("old", b"old")], submission_id="old")
        needed = source.publish([Record("needed", b"needed")], submission_id="needed")
        writer = store(tmp_path, target)
        with pytest.raises(UnsupportedSchema):
            writer.publish([Record("bad", b"x", codec="unsupported")], submission_id="bad", dependencies=[old])

        def fail(at):
            if at == "after_data_sync":
                raise OSError(errno.EIO, "complete handoff remains uncertain")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            writer.publish([Record("new", b"new")], submission_id="new", dependencies=[needed])
        writer.close()
        source.release_publications([old, needed])
        gc = source.collect_garbage()
        assert gc["reclaimed_files"] == 1  # abandoned old preparation only
        assert next(source.read(needed)).payload == b"needed"
        # The intentionally closed complete handoff is not recoverable yet;
        # its retained data remains in pytest's private fixture, not normal GC.
