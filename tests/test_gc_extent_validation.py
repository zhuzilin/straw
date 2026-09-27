import dataclasses

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, InvalidReference
from straw.protocol import RecordRef


def published_records(root):
    writer = SharedFilesystemStore(root, "extent", online_gc=True)
    ref = writer.publish([Record(str(i), bytes([i])) for i in range(4)], submission_id="source")
    members = [RecordRef.from_dict(value) for value in writer.manifest(ref)["records"]]
    writer.close()
    return ref, members


@pytest.mark.parametrize("change", [{"checksum": "0" * 64}, {"offset": 1}, {"run_id": "wrong"}])
def test_retention_distinguishes_full_extent_descriptor(tmp_path, change):
    _, members = published_records(tmp_path)
    reader = SharedFilesystemStore(tmp_path, "extent", online_gc=True)
    altered = dataclasses.replace(members[1], segment=dataclasses.replace(members[1].segment, **change))
    try:
        with pytest.raises((CorruptData, InvalidReference)):
            reader.retain("reader", [members[0], altered])
    finally:
        reader.close()


def test_new_member_rechecks_index_in_a_later_retention_operation(tmp_path):
    _, members = published_records(tmp_path)
    reader = SharedFilesystemStore(tmp_path, "extent", online_gc=True)
    reader.retain("first", [members[0]])
    segment = members[1].segment
    path = tmp_path / segment.path
    with path.open("r+b") as stream:
        stream.seek(segment.offset + segment.size - 80)
        index_bytes = int.from_bytes(stream.read(8), "little")
        offset = segment.offset + segment.size - 80 - index_bytes
        stream.seek(offset)
        byte = stream.read(1)
        stream.seek(offset)
        stream.write(bytes([byte[0] ^ 1]))
    try:
        with pytest.raises(CorruptData, match="index checksum"):
            reader.retain("second", [members[1]])
    finally:
        reader.close()


def test_shared_extent_remains_live_until_all_owners_release(tmp_path):
    ref, members = published_records(tmp_path)
    reader = SharedFilesystemStore(tmp_path, "extent", online_gc=True)
    try:
        reader.retain("first", members[:2])
        reader.retain("second", members[2:])
        reader.release_publications([ref])
        assert reader.collect_garbage()["reclaimed_files"] == 0
        reader.release("first")
        assert reader.collect_garbage()["reclaimed_files"] == 0
        assert reader.read_record(members[3]).payload == bytes([3])
        reader.release("second")
        assert reader.collect_garbage()["reclaimed_files"] == 1
    finally:
        reader.close()


def test_staging_retirement_authenticates_later_manifest_before_releasing(tmp_path):
    import struct

    from straw import Publication

    with SharedFilesystemStore(tmp_path, "retirement", online_gc=True) as store:
        refs = store.publish_many(
            [Publication((Record(str(i), bytes([i])),)) for i in range(4)], submission_id="batch"
        )
        store.retain("consumer", refs)
        # consume visits in reverse order: authenticate a good manifest first,
        # then reject corruption in another manifest sharing the cached index.
        ref = refs[0].manifest
        entry = store.inspect_segment(ref.segment)["records"][ref.ordinal]
        start = ref.segment.offset + entry["offset"]
        with store.backend.path(ref.segment.path).open("r+b") as stream:
            stream.seek(start)
            length, _ = struct.unpack("<IQ", stream.read(12))
            stream.seek(start + 12 + length)
            original = stream.read(1)
            stream.seek(-1, 1)
            stream.write(bytes([original[0] ^ 1]))
        before = (tmp_path / "storage.log").read_bytes()
        with pytest.raises(CorruptData, match="payload checksum"):
            store.release_publications(refs)
        assert (tmp_path / "storage.log").read_bytes() == before
