"""Setup failures must not lose new reservations or damage existing packs."""

import errno

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import IndeterminateCommit, StorageUnavailable

PACKED = 64 * 1024 * 1024


def make(root, target=PACKED):
    return SharedFilesystemStore(root, "preparation", online_gc=True, segment_target_bytes=target)


def data_files(root):
    return [p for p in root.rglob("*") if p.suffix in (".partial", ".pack", ".sealed")]


@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize(
    "phase", ["after_storage_catalog_part", "before_storage_catalog_sync", "after_storage_catalog_sync"]
)
@pytest.mark.parametrize("retry", [False, True])
def test_uncertain_registration_is_retired_on_close_or_retry(tmp_path, target, phase, retry):
    writer = make(tmp_path, target)
    seen = 0

    def fail(at):
        nonlocal seen
        if at == phase:
            seen += 1
            if seen == (3 if phase == "after_storage_catalog_part" else 1):
                raise OSError(errno.EIO, "registration response lost")

    writer.backend.fault = fail
    with pytest.raises(StorageUnavailable, match="registration response lost"):
        writer.write_records([Record("data", b"x" * 2048)], submission_id="data")
    writer.backend.fault = lambda _: None
    if retry:
        refs = writer.write_records([Record("data", b"x" * 2048)], submission_id="data")
        assert writer.read_record(refs[0]).payload == b"x" * 2048
        writer.release_publications(refs)
    writer.close()
    with make(tmp_path) as peer:
        gc = peer.collect_garbage()
        assert gc["open_packs"] == gc["owners"] == 0
        assert gc["reclaimed_files"] == int(retry)
    assert not data_files(tmp_path)


@pytest.mark.parametrize(
    "target,phase",
    [
        (target, phase)
        for target in (0, PACKED)
        for phase in ("before_directory_sync", "before_publication_open", "after_publication_open")
    ]
    + [(PACKED, "before_publication_offset")],
)
def test_new_file_setup_failure_leaves_no_open_reservation(tmp_path, target, phase):
    with make(tmp_path, target) as writer:
        injected = False
        phases = []

        def fail(at):
            nonlocal injected
            phases.append(at)
            if at == phase and not injected:
                injected = True
                raise OSError(errno.EIO, "setup interrupted")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="setup interrupted"):
            writer.write_records([Record("data", b"x" * 8192)], submission_id="data")
        assert injected and "after_segment_header" not in phases
        with make(tmp_path) as peer:
            gc = peer.collect_garbage()
            assert gc["open_packs"] == gc["owners"] == gc["reclaimed_files"] == 0
        assert not data_files(tmp_path)
        writer.backend.fault = lambda _: None
        refs = writer.write_records([Record("data", b"x" * 8192)], submission_id="data")
        assert writer.read_record(refs[0]).payload == b"x" * 8192
        writer.release_publications(refs)
        writer.seal()
        assert writer.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("phase", ["before_publication_open", "after_publication_open", "before_publication_offset"])
def test_existing_pack_setup_failure_preserves_its_complete_prefix(tmp_path, phase):
    with make(tmp_path) as writer:
        good = writer.write_records([Record("good", b"keep")], submission_id="good")
        path = tmp_path / good[0].segment.path
        prefix = path.read_bytes()

        def fail(at):
            if at == phase:
                raise OSError(errno.EIO, "existing setup interrupted")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            writer.write_records([Record("new", b"new")], submission_id="new")
        assert path.read_bytes() == prefix and writer.read_record(good[0]).payload == b"keep"
        with make(tmp_path) as peer:
            assert peer.collect_garbage()["reclaimed_files"] == 0
        writer.backend.fault = lambda _: None
        new = writer.write_records([Record("new", b"new")], submission_id="new")
        assert new[0].segment.offset == len(prefix)
        writer.release_publications([*good, *new])
        writer.seal()
        assert writer.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize(
    "cleanup",
    [
        "before_incomplete_unlink",
        "before_incomplete_parent_repair",
        "before_directory_sync",
        "before_incomplete_seal",
        "after_incomplete_seal",
    ],
)
def test_preparation_guard_survives_cleanup_errors_and_peer_gc(tmp_path, target, cleanup):
    with make(tmp_path, target) as writer:
        interrupted = False

        def fail(phase):
            nonlocal interrupted
            if phase == "after_storage_catalog_sync" and not interrupted:
                interrupted = True
                raise OSError(errno.EIO, "registration response lost")
            if interrupted and phase == cleanup:
                raise OSError(errno.EIO, "preparation cleanup interrupted")

        writer.backend.fault = fail
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.write_records([Record("data", b"x" * 2048)], submission_id="data")
        with make(tmp_path) as peer:
            gc = peer.collect_garbage()
            assert gc["reclaimed_files"] == 0
            if cleanup == "after_incomplete_seal":
                assert gc["tombstones"] == 1
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.write_records([Record("other", b"other")], submission_id="other")
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.seal()
        writer.backend.fault = lambda _: None
        refs = writer.write_records([Record("data", b"x" * 2048)], submission_id="data")
        writer.release_publications(refs)
        writer.seal()
        gc = writer.collect_garbage()
        assert gc["open_packs"] == gc["owners"] == 0
        assert not data_files(tmp_path)


@pytest.mark.parametrize("target", [0, PACKED])
def test_failed_directory_setup_can_be_closed_after_obstruction_is_removed(tmp_path, target):
    writer = make(tmp_path, target)
    obstructed = False

    def obstruct(phase):
        nonlocal obstructed
        if phase == "after_storage_catalog_sync" and not obstructed:
            obstructed = True
            (tmp_path / "raw").write_bytes(b"private obstruction")

    writer.backend.fault = obstruct
    with pytest.raises(IndeterminateCommit):
        writer.write_records([Record("data", b"data")], submission_id="data")
    (tmp_path / "raw").unlink()
    writer.close()
    with make(tmp_path) as peer:
        assert peer.collect_garbage()["open_packs"] == 0
    assert not data_files(tmp_path)
