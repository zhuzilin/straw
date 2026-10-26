"""Certified path keys avoid replay lookups without reinterpreting legacy keys."""

import errno
import hashlib
import json
import struct
from dataclasses import replace

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, StorageUnavailable


def store(root):
    return SharedFilesystemStore(root, "encoding", online_gc=True, segment_target_bytes=0)


def events(path):
    data = path.read_bytes()
    offset = 0
    result = []
    while offset < len(data):
        sequence, length = struct.unpack_from("<QQ", data, offset + 8)
        assert sequence == len(result)
        result.append(json.loads(data[offset + 56 : offset + 56 + length]))
        offset += 96 + length
    assert offset == len(data)
    return result


def write_events(path, values):
    frames = []
    for sequence, value in enumerate(values):
        body = json.dumps(value, separators=(",", ":")).encode()
        prefix = b"STRGC001" + struct.pack("<QQ", sequence, len(body))
        header = prefix + hashlib.sha256(prefix).digest()
        frames.append(header + body + hashlib.sha256(header + body).digest() + b"STRGEND1")
    # Quiescent offline format fixture, never a concurrent catalog replacement.
    path.write_bytes(b"".join(frames))


def prepare(root):
    with store(root) as writer:
        (ref,) = writer.write_records([Record("r", b"x" * 2048)], submission_id="source")
        (root / "alias.sealed").symlink_to(ref.segment.path)
        alias = replace(ref, segment=replace(ref.segment, path="alias.sealed"))
        writer.retain("reader", [alias])
        writer.release_publications([ref])
    return ref, alias


def test_current_writer_certifies_normalized_keys_in_all_fields(tmp_path):
    ref, alias = prepare(tmp_path)
    with store(tmp_path) as reader:
        assert reader.collect_garbage()["reclaimed_files"] == 0
        assert reader.read_record(alias).payload == b"x" * 2048
        reader.release("reader")
        assert reader.collect_garbage()["reclaimed_files"] == 1
    seen = set()
    for event in events(tmp_path / "storage.log"):
        assert event["path_encoding"] == 1
        for paths in event.get("owners", {}).values():
            if paths is not None:
                assert paths == [ref.segment.path]
                seen.add("owners")
        for field in ("packs", "deleted", "reclaimed"):
            if field in event:
                assert set(event[field]) == {ref.segment.path}
                seen.add(field)
    assert seen == {"owners", "packs", "deleted", "reclaimed"}


@pytest.mark.parametrize("marker", [0, 2, -1, None, "1", True, 1.5, 2**64])
def test_unknown_or_malformed_encoding_fails_before_applying_owner(tmp_path, marker):
    ref, _ = prepare(tmp_path)
    log = tmp_path / "storage.log"
    values = events(log)
    values.append({"run_id": "encoding", "path_encoding": marker, "owners": {"reader": None}})
    write_events(log, values)
    before = log.read_bytes()
    with pytest.raises(CorruptData, match="path encoding"):
        with store(tmp_path) as reader:
            reader.collect_garbage()
    assert log.read_bytes() == before
    assert (tmp_path / ref.segment.path).is_file()


@pytest.mark.parametrize("phase", ["owned", "tombstone", "receipt"])
def test_legacy_raw_alias_fields_still_resolve_after_upgrade(tmp_path, phase):
    ref, alias = prepare(tmp_path)
    if phase != "owned":
        with store(tmp_path) as writer:
            writer.release("reader")
            if phase == "tombstone":

                def stop(point):
                    if point == "after_gc_tombstone":
                        raise OSError(errno.EIO, "stop before unlink")

                writer.backend.fault = stop
                with pytest.raises(StorageUnavailable, match="stop before unlink"):
                    writer.collect_garbage()
            else:
                assert writer.collect_garbage()["reclaimed_files"] == 1
    log = tmp_path / "storage.log"
    values = events(log)
    for event in values:
        event.pop("path_encoding", None)
        for owner, paths in event.get("owners", {}).items():
            if paths is not None:
                event["owners"][owner] = ["alias.sealed" for _ in paths]
        if "packs" in event:
            event["packs"] = {"alias.sealed": event["packs"][ref.segment.path]}
        for field in ("deleted", "reclaimed"):
            if field in event:
                event[field] = ["alias.sealed"]
    write_events(log, values)
    with store(tmp_path) as reader:
        if phase == "owned":
            assert reader.collect_garbage()["reclaimed_files"] == 0
            assert reader.read_record(alias).payload == b"x" * 2048
            reader.release("reader")
        assert reader.collect_garbage()["reclaimed_files"] == int(phase != "receipt")
    with store(tmp_path) as reader:
        assert reader.collect_garbage()["reclaimed_files"] == 0


@pytest.mark.parametrize(
    ("field", "malformed"),
    [(field, value) for field in ("owners", "packs", "deleted") for value in (None, False, 3, "invalid")]
    + [("owners", []), ("packs", []), ("deleted", {})],
)
def test_wrong_field_type_is_not_replayed_as_an_empty_update(tmp_path, field, malformed):
    ref, _ = prepare(tmp_path)
    log = tmp_path / "storage.log"
    values = events(log)
    values.append({"run_id": "encoding", "path_encoding": 1, field: malformed})
    write_events(log, values)
    before = log.read_bytes()
    with pytest.raises(CorruptData):
        with store(tmp_path) as reader:
            reader.collect_garbage()
    assert log.read_bytes() == before
    assert (tmp_path / ref.segment.path).is_file()
