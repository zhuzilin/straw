"""A sealed reservation may have no directory after failed writer setup."""

import errno
import json
import struct

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import IndeterminateCommit, StorageUnavailable


def make(root, target=0):
    return SharedFilesystemStore(root, "missing-parent", online_gc=True, segment_target_bytes=target)


def events(root):
    raw = (root / "storage.log").read_bytes()
    offset, result = 0, []
    while offset < len(raw):
        length = struct.unpack_from("<Q", raw, offset + 16)[0]
        result.append(json.loads(raw[offset + 56 : offset + 56 + length]))
        offset += 96 + length
    return result


def failed_reservation(root):
    writer = make(root, 64 * 1024 * 1024)
    blocked = False

    def obstruct(phase):
        nonlocal blocked
        if phase == "after_storage_catalog_sync" and not blocked:
            blocked = True
            (root / "raw").write_bytes(b"private test obstruction")

    writer.backend.fault = obstruct
    try:
        with pytest.raises((StorageUnavailable, IndeterminateCommit)):
            writer.write_records([Record("never-written", b"x" * 2048)], submission_id="failed")
        assert blocked
    finally:
        (root / "raw").unlink(missing_ok=True)
        writer.backend.fault = lambda _: None
        writer.close()
    (name,) = {name for event in events(root) for name in event.get("packs", {})}
    # Current live-writer setup cleanup repairs the parent itself. Reconstruct
    # the empty missing-prefix state left by historical writers for GC replay.
    for parent in (root / name).parents:
        if parent == root:
            break
        if parent.is_dir():
            parent.rmdir()
    assert not (root / name).parent.exists()
    return root / name


def no_revisit(phase):
    assert phase not in {"before_gc_stat", "before_gc_unlink", "before_gc_directory_sync", "before_gc_receipt"}


@pytest.mark.parametrize("missing", [1, 2, 3])
@pytest.mark.parametrize("size", [2048, 8 * 1024 * 1024])
def test_failed_setup_does_not_block_other_gc_or_replay(tmp_path, missing, size):
    orphan = failed_reservation(tmp_path)
    # Missing leaf, writer-prefix pair, or entire raw subtree are all legal
    # outcomes of setup failure. No payload was ever created for this name.
    orphan.parents[missing].mkdir(parents=True, exist_ok=True)
    with make(tmp_path) as owner:
        kept = owner.write_records([Record("kept", b"keep")], submission_id="kept")
        garbage = owner.write_records([Record("garbage", b"x" * size)], submission_id="garbage")
        owner.release_publications(garbage)
        with make(tmp_path) as collector:
            gc = collector.collect_garbage()
            assert gc["open_packs"] == 0 and gc["reclaimed_files"] == 1
            assert owner.read_record(kept[0]).payload == b"keep"
            assert orphan.parent.is_dir() and not orphan.exists()
        with make(tmp_path) as restarted:
            restarted.backend.fault = no_revisit
            assert restarted.collect_garbage()["reclaimed_files"] == 0
        owner.release_publications(kept)
        assert owner.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize(
    "phase,occurrence",
    [
        ("before_gc_parent_repair", 1),
        ("before_directory_sync", 1),
        ("before_directory_sync", 3),
        ("after_gc_parent_repair", 1),
        ("before_gc_receipt", 1),
    ],
)
def test_failed_repair_keeps_receipt_pending_across_new_client(tmp_path, phase, occurrence):
    orphan = failed_reservation(tmp_path)
    seen = 0

    def fail(at):
        nonlocal seen
        if at == phase:
            seen += 1
            if seen == occurrence:
                raise OSError(errno.EIO, "private parent repair interruption")

    with make(tmp_path) as collector:
        collector.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="parent repair interruption"):
            collector.collect_garbage()
        assert seen == occurrence
        assert not any("reclaimed" in event for event in events(tmp_path))
    with make(tmp_path) as restarted:
        gc = restarted.collect_garbage()
        assert gc["reclaimed_files"] == 0 and gc["tombstones"] == 1
        assert orphan.parent.is_dir() and not orphan.exists()
        assert any("reclaimed" in event for event in events(tmp_path))
        restarted.backend.fault = no_revisit
        assert restarted.collect_garbage()["reclaimed_files"] == 0


def test_existing_parent_uses_one_barrier_without_recreating(tmp_path):
    with make(tmp_path) as owner:
        refs = owner.write_records([Record("garbage", b"x" * 2048)], submission_id="garbage")
        owner.release_publications(refs)
        phases = []
        owner.backend.fault = phases.append
        assert owner.collect_garbage()["reclaimed_files"] == 1
        assert phases.count("before_gc_directory_sync") == 1
        assert "before_gc_parent_repair" not in phases and "before_directory_sync" not in phases
