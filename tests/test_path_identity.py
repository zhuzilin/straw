"""An in-pool symlink must not split read identity from GC ownership."""

import errno
import hashlib
import os
from dataclasses import replace
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import InvalidReference, StorageUnavailable


def alias_reference(root, ref, kind):
    segment = ref.segment if hasattr(ref, "segment") else ref.manifest.segment
    suffix = Path(segment.path).suffix
    if kind == "directory":
        (root / "alias").symlink_to(Path(segment.path).parent, target_is_directory=True)
        path = "alias/" + Path(segment.path).name
    else:
        path = "alias" + suffix
        target = segment.path
        if kind == "chain":
            (root / ("middle" + suffix)).symlink_to(target)
            target = "middle" + suffix
        (root / path).symlink_to(target)
    if hasattr(ref, "segment"):
        return replace(ref, segment=replace(segment, path=path))
    return replace(ref, manifest=replace(ref.manifest, segment=replace(segment, path=path)))


@pytest.mark.parametrize("target", [0, 1024 * 1024])
@pytest.mark.parametrize("kind", ["file", "directory", "chain"])
@pytest.mark.parametrize("api", ["record", "publication"])
def test_alias_owner_protects_canonical_file_across_reopen(tmp_path, target, kind, api):
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True, segment_target_bytes=target) as store:
        record = Record("r", b"x" * 2048)
        ref = (
            store.write_records([record], submission_id="source")[0]
            if api == "record"
            else store.publish([record], submission_id="source")
        )
        alias = alias_reference(tmp_path, ref, kind)
        read = store.read_record(alias) if api == "record" else next(store.read(alias))
        assert read.payload == record.payload
        store.retain("reader", [alias])
        store.seal()
        store.release_publications([ref])
        assert store.collect_garbage()["reclaimed_files"] == 0
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True) as fresh:
        assert fresh.collect_garbage()["reclaimed_files"] == 0
        read = fresh.read_record(alias) if api == "record" else next(fresh.read(alias))
        assert read.payload == record.payload
        fresh.release("reader")
        assert fresh.collect_garbage()["reclaimed_files"] == 1
    # Replay visits historical aliases after the target has gone. The alias
    # symlink remains, including dangling chains; receipt replay stays valid.
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True) as fresh:
        assert fresh.collect_garbage()["reclaimed_files"] == 0


@pytest.mark.parametrize("target", [0, 1024 * 1024])
def test_writer_directory_alias_uses_same_identity_as_canonical_reader(tmp_path, target):
    (tmp_path / "physical").mkdir()
    (tmp_path / "raw").symlink_to("physical", target_is_directory=True)
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True, segment_target_bytes=target) as store:
        (ref,) = store.write_records([Record("r", b"data")], submission_id="source")
        path = str((tmp_path / ref.segment.path).resolve().relative_to(tmp_path))
        canonical = replace(ref, segment=replace(ref.segment, path=path))
        store.retain("reader", [canonical])
        store.seal()
        store.release_publications([ref])
        assert store.collect_garbage()["reclaimed_files"] == 0
        assert store.read_record(canonical).payload == b"data"
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True) as fresh:
        assert fresh.collect_garbage()["reclaimed_files"] == 0
        fresh.release("reader")
        assert fresh.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("warm", [False, True])
def test_alias_cannot_bypass_tombstone_before_unlink(tmp_path, warm):
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True) as store:
        (ref,) = store.write_records([Record("r", b"data")], submission_id="source")
        alias = alias_reference(tmp_path, ref, "file")
        if warm:
            store.retain("warm", [alias])
            store.release("warm")
        store.seal()
        store.release_publications([ref])

        def stop(phase):
            if phase == "after_gc_tombstone":
                raise OSError(errno.EIO, "pause before unlink")

        store.backend.fault = stop
        with pytest.raises(StorageUnavailable, match="pause before unlink"):
            store.collect_garbage()
        assert store.read_record(alias).payload == b"data"
        before = (tmp_path / "storage.log").read_bytes()
        with pytest.raises(InvalidReference, match="reclaimed"):
            store.retain("late", [alias])
        assert (tmp_path / "storage.log").read_bytes() == before
        store.backend.fault = lambda _: None
        assert store.collect_garbage()["reclaimed_files"] == 1


def test_alias_dependencies_are_protected_by_new_publication(tmp_path):
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True, segment_target_bytes=0) as store:
        source = store.publish([Record("source", b"dependency")], submission_id="source")
        alias = alias_reference(tmp_path, source, "file")
        dependent = store.publish([Record("result", b"result")], dependencies=[alias], submission_id="dependent")
        store.release_publications([source])
        assert store.collect_garbage()["reclaimed_files"] == 0
        assert next(store.read(alias)).payload == b"dependency"
        store.release_publications([dependent])
        assert store.collect_garbage()["reclaimed_files"] == 2


@pytest.mark.parametrize("offset", [2**31 - 5, 2**32 - 5, 2**41 - 5])
def test_alias_ownership_and_range_read_at_sparse_large_offset(tmp_path, offset):
    with SharedFilesystemStore(tmp_path, "identity", online_gc=True) as store:
        payload = b"x" * 2048
        (original,) = store.write_records([Record("r", payload)], submission_id="source")
        path = tmp_path / original.segment.path
        extent = path.read_bytes()[original.segment.offset : original.segment.offset + original.segment.size]
        # A format fixture appends one valid extent at a sparse offset. It does
        # not write or authenticate the intervening GiB/TiB of holes.
        with path.open("r+b") as stream:
            stream.seek(offset)
            stream.write(extent)
            stream.flush()
            os.fsync(stream.fileno())
        shifted = replace(original, segment=replace(original.segment, offset=offset))
        alias = alias_reference(tmp_path, shifted, "file")
        store.retain("reader", [alias])
        store.seal()
        store.release_publications([original])
        assert store.collect_garbage()["reclaimed_files"] == 0
        assert (
            store.read_range(alias, 7, 23, chunk_bytes=len(payload), checksums=[hashlib.sha256(payload).hexdigest()])
            == payload[7:23]
        )
        store.release("reader")
        result = store.collect_garbage()
        assert result["reclaimed_files"] == 1
        assert result["reclaimed_bytes"] == offset + original.segment.size
