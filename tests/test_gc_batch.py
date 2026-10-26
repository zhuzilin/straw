"""Real multi-pack GC keeps every parent barrier ahead of durable receipts."""

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import InvalidReference, StorageUnavailable


def store(root):
    return SharedFilesystemStore(root, "batch", online_gc=True, segment_target_bytes=0)


def prepare(root):
    refs = []
    # Each writer has its own directory; several packs share each parent.
    for writer in range(3):
        with store(root) as s:
            for pack in range(2):
                name = f"{writer}-{pack}"
                refs.append(s.publish([Record(name, b"garbage")], submission_id=name))
            s.seal()
            s.release_publications(refs[-2:])
    paths = {root / ref.manifest.segment.path for ref in refs}
    parents = {path.parent for path in paths}
    assert len(paths) == 6 and len(parents) == 3
    return refs, paths, parents


def test_gc_batch_syncs_each_shared_parent_before_receipt(tmp_path):
    refs, paths, parents = prepare(tmp_path)
    seen = []
    with store(tmp_path) as s:

        def observe(at):
            seen.append(at)
            if at == "before_gc_directory_sync":
                assert all(not p.exists() for p in paths)
            if at == "before_gc_receipt":
                assert seen.count("before_gc_unlink") == len(paths)
                assert seen.count("before_gc_directory_sync") == len(parents)

        s.backend.fault = observe
        result = s.collect_garbage()
        assert result["reclaimed_files"] == 6
        assert seen.count("before_gc_receipt") == 1
        assert all(not path.exists() for path in paths)
    history = (tmp_path / "storage.log").read_bytes()
    with store(tmp_path) as recovered:
        seen.clear()
        recovered.backend.fault = seen.append
        assert recovered.collect_garbage()["reclaimed_files"] == 0
        assert not any(at.startswith("before_gc_") and at != "before_gc_tombstone" for at in seen)
        # Durable tombstones reject even a cold retain before it attempts to
        # reconstruct a closure from an already deleted manifest.
        for ref in refs:
            with pytest.raises(InvalidReference, match="reclaimed"):
                recovered.retain("late", [ref])
        state = recovered.collect_garbage()
        assert state["owners"] == 0 and state["tombstones"] == 6
    assert (tmp_path / "storage.log").read_bytes() == history


@pytest.mark.parametrize(
    "phase,occurrence",
    [
        ("before_storage_catalog_write", 1),
        ("after_storage_catalog_part", 1),
        ("after_storage_catalog_part", 2),
        ("after_storage_catalog_part", 3),
        ("before_storage_catalog_sync", 1),
        ("after_storage_catalog_sync", 1),
    ],
)
def test_gc_mark_failure_never_starts_deletion_and_recovers_all_candidates(tmp_path, phase, occurrence):
    _, paths, _ = prepare(tmp_path)
    with store(tmp_path) as s:
        seen = []

        def stop(at):
            seen.append(at)
            if at == phase and seen.count(at) == occurrence:
                raise OSError(5, "injected mark failure")

        s.backend.fault = stop
        with pytest.raises(StorageUnavailable, match="mark failure"):
            s.collect_garbage()
        assert all(path.exists() for path in paths)
        assert not {"after_gc_tombstone", "before_gc_stat", "before_gc_unlink", "before_gc_receipt"}.intersection(seen)
    with store(tmp_path) as recovered:
        assert recovered.collect_garbage()["reclaimed_files"] == len(paths)
        assert not any(path.exists() for path in paths)
        assert recovered.collect_garbage()["reclaimed_files"] == 0


def test_gc_mark_unions_new_candidates_with_old_pending_and_preserves_live(tmp_path):
    _, old_paths, _ = prepare(tmp_path)
    with store(tmp_path) as s:

        def stop(at):
            if at == "after_gc_tombstone":
                raise OSError(5, "marked before deletion")

        s.backend.fault = stop
        with pytest.raises(StorageUnavailable, match="marked before deletion"):
            s.collect_garbage()
    with store(tmp_path) as s:
        new = s.publish([Record("new", b"new garbage")], submission_id="new")
        live = s.publish([Record("live", b"still owned")], submission_id="live")
        s.seal()
        s.retain("reader", [live])
        s.release_publications([new, live])
        result = s.collect_garbage()
        assert result["reclaimed_files"] == len(old_paths) + 1
        assert not any(path.exists() for path in old_paths)
        assert not (tmp_path / new.manifest.segment.path).exists()
        assert (tmp_path / live.manifest.segment.path).exists()
        assert next(s.read(live)).payload == b"still owned"
        assert s.collect_garbage()["reclaimed_files"] == 0
        s.release("reader")
        assert s.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize(
    "phase,occurrence,missing_parent,receipt_survives",
    [
        ("before_gc_unlink", 2, False, False),
        ("before_gc_unlink", 6, False, False),
        ("before_gc_directory_sync", 1, False, False),
        ("before_gc_directory_sync", 2, False, False),
        ("before_gc_directory_sync", 1, True, False),
        ("before_gc_receipt", 1, False, False),
        ("after_storage_catalog_sync", 1, False, True),
    ],
)
def test_gc_batch_recovers_partial_unlinks_barriers_and_receipts(
    tmp_path, phase, occurrence, missing_parent, receipt_survives
):
    _, paths, parents = prepare(tmp_path)
    with store(tmp_path) as s:
        seen = 0
        receipt = False

        def fail(at):
            nonlocal seen, receipt
            if at == "before_gc_receipt":
                receipt = True
            if at == phase and (phase != "after_storage_catalog_sync" or receipt):
                seen += 1
                if seen == occurrence:
                    raise OSError(5, "injected batch failure")

        s.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="batch failure"):
            s.collect_garbage()
        remaining = sum(path.exists() for path in paths)
        assert remaining == (7 - occurrence if phase == "before_gc_unlink" else 0)
        s.backend.fault = lambda _: None
    if missing_parent:
        # All unlinks happened, but no receipt did. Recreate and synchronize
        # a missing parent even though every child name is already absent.
        next(iter(parents)).rmdir()
    with store(tmp_path) as recovered:
        phases = []

        def observe(at):
            phases.append(at)
            if at == "before_gc_receipt":
                assert all(not p.exists() for p in paths)
                assert phases.count("before_gc_directory_sync") == len(parents)

        recovered.backend.fault = observe
        result = recovered.collect_garbage()
        assert result["reclaimed_files"] == remaining
        assert phases.count("before_gc_unlink") == (0 if receipt_survives else len(paths))
        assert phases.count("before_gc_directory_sync") == (0 if receipt_survives else len(parents))
        assert phases.count("before_gc_parent_repair") == int(missing_parent)
        assert phases.count("before_gc_receipt") == int(not receipt_survives)
        phases.clear()
        assert recovered.collect_garbage()["reclaimed_files"] == 0
        assert "before_gc_stat" not in phases
