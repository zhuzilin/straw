"""Deletion receipts survive replay without discarding permanent tombstones."""

import hashlib
import json
import struct

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, InvalidReference, StorageUnavailable


def store(root):
    return SharedFilesystemStore(root, "receipt", online_gc=True)


def prepare(s):
    ref = s.publish([Record("data", b"garbage")], submission_id="source")
    s.seal()
    s.release_publications([ref])
    return ref


def no_payload_io(phase):
    assert phase not in {"before_gc_stat", "before_gc_unlink", "before_gc_directory_sync", "before_gc_receipt"}


def test_completed_deletion_is_not_revisited_after_reopen(tmp_path):
    with store(tmp_path) as s:
        ref = prepare(s)
        assert s.collect_garbage()["reclaimed_files"] == 1
        s.backend.fault = no_payload_io
        assert s.collect_garbage()["reclaimed_files"] == 0
        with pytest.raises(InvalidReference, match="reclaimed"):
            s.retain("late-owner", [ref])
    before = (tmp_path / "storage.log").read_bytes()
    with store(tmp_path) as recovered:
        recovered.backend.fault = no_payload_io
        result = recovered.collect_garbage()
        assert result["tombstones"] == 1
        assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
    assert (tmp_path / "storage.log").read_bytes() == before


@pytest.mark.parametrize(
    "phase,occurrence,complete_frame",
    [
        ("before_storage_catalog_write", 1, False),
        ("after_storage_catalog_part", 1, False),
        ("after_storage_catalog_part", 2, False),
        ("after_storage_catalog_part", 3, True),
        ("before_storage_catalog_sync", 1, True),
        ("after_storage_catalog_sync", 1, True),
    ],
)
def test_uncertain_receipt_replays_or_retries(tmp_path, phase, occurrence, complete_frame):
    with store(tmp_path) as s:
        ref = prepare(s)
        armed = False
        seen = 0

        def fail_receipt(at):
            nonlocal armed, seen
            if at == "before_gc_receipt":
                armed = True
            if armed and at == phase:
                seen += 1
                if seen == occurrence:
                    raise OSError(5, "injected receipt failure")

        s.backend.fault = fail_receipt
        with pytest.raises(StorageUnavailable, match="receipt failure"):
            s.collect_garbage()
        assert not (tmp_path / ref.manifest.segment.path).exists()
        effects = []
        s.backend.fault = effects.append
        assert s.collect_garbage()["reclaimed_files"] == 0
        assert effects.count("before_gc_stat") == (0 if complete_frame else 1)
        assert effects.count("before_gc_directory_sync") == (0 if complete_frame else 1)
        assert effects.count("before_gc_receipt") == (0 if complete_frame else 1)
        s.backend.fault = no_payload_io
        assert s.collect_garbage()["reclaimed_files"] == 0


def test_receipt_all_byte_prefixes_and_corruption(tmp_path):
    path = tmp_path / "storage.log"
    with store(tmp_path) as s:
        prepare(s)
        prefixes = []
        s.backend.fault = lambda at: prefixes.append(path.read_bytes()) if at == "before_gc_receipt" else None
        s.collect_garbage()
        before = prefixes[0]
    after = path.read_bytes()
    delta = len(after) - len(before)
    assert delta > 96
    for length in range(delta + 1):
        path.write_bytes(after[: len(before) + length])
        with store(tmp_path) as recovered:
            effects = []
            recovered.backend.fault = effects.append
            assert recovered.collect_garbage()["reclaimed_files"] == 0
            assert effects.count("before_gc_stat") == (0 if length == delta else 1)
            assert effects.count("before_gc_directory_sync") == (0 if length == delta else 1)
    for index in range(len(before), len(after)):
        damaged = bytearray(after)
        damaged[index] ^= 0x80
        path.write_bytes(damaged)
        with pytest.raises(CorruptData):
            with store(tmp_path) as recovered:
                recovered.collect_garbage()
        assert path.read_bytes() == damaged
    path.write_bytes(after)


def append_catalog_event(path, event):
    raw = path.read_bytes()
    offset = sequence = 0
    while offset < len(raw):
        sequence, length = struct.unpack_from("<QQ", raw, offset + 8)
        offset += 96 + length
    body = json.dumps({"run_id": "receipt", **event}, separators=(",", ":")).encode()
    prefix = b"STRGC001" + struct.pack("<QQ", sequence + 1, len(body))
    header = prefix + hashlib.sha256(prefix).digest()
    path.write_bytes(raw + header + body + hashlib.sha256(header + body).digest() + b"STRGEND1")


def test_duplicate_tombstone_does_not_requeue_completed_deletion(tmp_path):
    with store(tmp_path) as s:
        ref = prepare(s)
        s.collect_garbage()
    append_catalog_event(tmp_path / "storage.log", {"deleted": [ref.manifest.segment.path]})
    with store(tmp_path) as recovered:
        recovered.backend.fault = no_payload_io
        assert recovered.collect_garbage()["tombstones"] == 1


def test_receipt_cannot_retire_an_unmarked_live_pack(tmp_path):
    with store(tmp_path) as s:
        ref = s.publish([Record("live", b"keep")], submission_id="source")
        s.seal()
    append_catalog_event(tmp_path / "storage.log", {"reclaimed": [ref.manifest.segment.path]})
    with pytest.raises(CorruptData, match="without tombstone"):
        with store(tmp_path) as recovered:
            recovered.collect_garbage()
    assert (tmp_path / ref.manifest.segment.path).exists()
