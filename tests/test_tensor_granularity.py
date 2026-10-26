"""Large tensor metadata must not silently enlarge the authentication unit."""

import dataclasses
import hashlib
import struct

import pytest
import torch

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, IdempotencyConflict
from straw.protocol import encode
from straw.tensor import TensorRef, publish_tensors


@pytest.mark.parametrize("chunks", [0, 512, 513, 1025])
def test_fixed_granularity_past_the_old_digest_budget(tmp_path, monkeypatch, chunks):
    import straw.tensor as tensor

    # Scale the policy threshold, not the reader's metadata or hashing logic.
    monkeypatch.setattr(tensor, "CHUNK_BYTES", 17)
    length = chunks * 17 + (7 if chunks else 0)
    value = torch.arange(length, dtype=torch.uint8)
    with SharedFilesystemStore(tmp_path, "grain", codecs=("tensor.v1",)) as store:
        (ref,) = publish_tensors(store, {"value": value}, submission_id="fixed")
        segment = store.inspect_segment(ref.record_ref.segment)
        envelope = segment["records"][ref.record_ref.ordinal]["envelope"]
        assert envelope["metadata"]["chunk_bytes"] == 17
        assert len(envelope["metadata"]["chunks"]) == (length + 16) // 17
        # A retry under the same policy preserves the publication descriptor.
        (again,) = publish_tensors(store, {"value": value}, submission_id="fixed")
        assert again.record_ref == ref.record_ref
        raw = encode(dataclasses.asdict(ref.record_ref)).decode()
        with store.read_session() as reader:
            for start, stop in [(0, min(length, 1)), (length // 2, min(length, length // 2 + 35)), (length, length)]:
                _, data, checked = reader._native.read_tensor_range(raw, start, stop)
                assert data == bytes(value[start:stop].tolist())
                assert checked <= stop - start + 2 * 16
                if start == stop:
                    assert checked == 0


def test_legacy_chunk_size_is_honored_by_new_readers(tmp_path):
    payload = bytes(range(251)) * 4
    chunk = 513
    metadata = {
        "shape": [len(payload)],
        "dtype": "uint8",
        "chunk_bytes": chunk,
        "chunks": [hashlib.sha256(payload[i : i + chunk]).hexdigest() for i in range(0, len(payload), chunk)],
    }
    with SharedFilesystemStore(tmp_path, "legacy", codecs=("tensor.v1",)) as store:
        publication = store.publish([Record("old", payload, "tensor.v1", metadata)], submission_id="legacy")
        ref = TensorRef.from_record_set(store, publication)
        raw = encode(dataclasses.asdict(ref.record_ref)).decode()
        _, data, checked = store._native.read_tensor_range(raw, 512, 515)
        assert data == payload[512:515]
        assert checked == len(payload)


def test_retry_does_not_reinterpret_metadata_from_another_policy(tmp_path, monkeypatch):
    import straw.tensor as tensor

    value = torch.arange(1025, dtype=torch.uint8)
    with SharedFilesystemStore(tmp_path, "upgrade", codecs=("tensor.v1",)) as store:
        monkeypatch.setattr(tensor, "CHUNK_BYTES", 34)
        original = tensor.tensor_record(value, "same:value", kind="value")
        (ref,) = publish_tensors(store, {"value": value}, submission_id="same")
        monkeypatch.setattr(tensor, "CHUNK_BYTES", 17)
        with pytest.raises(IdempotencyConflict):
            publish_tensors(store, {"value": value}, submission_id="same")
        # Retrying the original serialized metadata still identifies the same
        # immutable publication, and the original reader remains valid.
        assert store.publish([original], submission_id="same") == ref.blob_set
        assert torch.equal(ref[31:37], value[31:37])


def test_late_small_chunk_still_authenticates_unreturned_bytes(tmp_path, monkeypatch):
    import straw.tensor as tensor

    monkeypatch.setattr(tensor, "CHUNK_BYTES", 17)
    value = torch.arange(1025 * 17 + 7, dtype=torch.uint8)
    with SharedFilesystemStore(tmp_path, "corrupt", codecs=("tensor.v1",)) as store:
        (ref,) = publish_tensors(store, {"value": value}, submission_id="fixed")
        entry = store.inspect_segment(ref.record_ref.segment)["records"][ref.record_ref.ordinal]
        assert entry["envelope"]["metadata"]["chunk_bytes"] == 17
        begin = 700 * 17
        assert ref[begin : begin + 1].item() == value[begin].item()
        with open(ref.path, "r+b") as file:
            file.seek(ref.record_ref.segment.offset + entry["offset"])
            meta, _ = struct.unpack("<IQ", file.read(12))
            position = file.tell() + meta + begin + 16
            file.seek(position)
            byte = file.read(1)
            file.seek(position)
            file.write(bytes([byte[0] ^ 1]))
        with pytest.raises(CorruptData, match="chunk checksum"):
            ref[begin : begin + 1]
