import dataclasses
import pickle

import numpy as np
import pytest
import torch

from straw import SharedFilesystemStore
from straw.errors import CorruptData
from straw.tensor import CHUNK_BYTES, publish_tensors


def test_batch_tensor_restore_keeps_ordinals_in_a_mixed_publication(tmp_path):
    from straw import Record
    from straw.tensor import TensorRef, tensor_record

    store = SharedFilesystemStore(tmp_path, "mixed", codecs=("tensor.v1", "json.v1"))
    ref = store.publish(
        [tensor_record(torch.arange(3), "a"), Record("metadata", b"{}", "json.v1"), tensor_record(torch.ones(2), "b")],
        submission_id="mixed",
    )
    restored = TensorRef.from_record_set_many(store, ref)
    assert set(restored) == {0, 2}
    assert restored[0].load().tolist() == [0, 1, 2]
    assert restored[2].load().tolist() == [1.0, 1.0]
    assert restored[2].share(store, submission_id="reuse") == (ref, 2)


def test_packed_numpy_torch_views_survive_append_and_pickle(tmp_path):
    store = SharedFilesystemStore(tmp_path, "tensor", codecs=("tensor.v1",), segment_target_bytes=64 * 1024**2)
    values = {
        "routes": np.arange(120, dtype=np.int32).reshape(10, 3, 4),
        "scores": torch.arange(60, dtype=torch.bfloat16).reshape(10, 6),
        "empty": torch.empty((0, 8)),
        "scalar": torch.tensor(1.5),
    }
    refs = publish_tensors(store, values, submission_id="first")
    for i in range(100):
        publish_tensors(store, {"scalar": np.array(i, dtype=np.int64)}, submission_id=str(i))
    assert len(list(tmp_path.rglob("*.pack"))) == 1
    store.close()
    for ref, value in zip(pickle.loads(pickle.dumps(refs)), values.values(), strict=True):
        expected = torch.from_numpy(value) if isinstance(value, np.ndarray) else value
        assert torch.equal(ref.load(), expected)
        if expected.ndim:
            assert torch.equal(ref[2:5], expected[2:5])
    with pytest.raises(CorruptData):
        dataclasses.replace(refs[0], shape=(120,)).load()


def test_range_checks_only_intersecting_authenticated_chunks(tmp_path):
    store = SharedFilesystemStore(tmp_path, "tensor", codecs=("tensor.v1",), segment_target_bytes=64 * 1024**2)
    value = torch.arange(CHUNK_BYTES * 2 + 16, dtype=torch.uint8)
    (ref,) = publish_tensors(store, {"data": value}, submission_id="large")
    entry = store.inspect_segment(ref.record_ref.segment)["records"][ref.record_ref.ordinal]
    import struct

    with open(ref.path, "r+b") as stream:
        stream.seek(ref.record_ref.segment.offset + entry["offset"])
        envelope_size, _ = struct.unpack("<IQ", stream.read(12))
        start = stream.tell() + envelope_size
        stream.seek(start + CHUNK_BYTES + 7)
        stream.write(b"\xff")
    assert torch.equal(ref[:16], value[:16])
    with pytest.raises(CorruptData):
        ref[CHUNK_BYTES : CHUNK_BYTES + 16]


def test_range_trace_distinguishes_returned_and_checked_bytes(tmp_path, monkeypatch):
    import json

    trace = tmp_path / "trace"
    monkeypatch.setenv("STRAW_TRACE_DIR", str(trace))
    store = SharedFilesystemStore(tmp_path / "data", "trace", codecs=("tensor.v1",))
    (ref,) = publish_tensors(store, {"data": torch.zeros(CHUNK_BYTES + 16, dtype=torch.uint8)}, submission_id="tensor")
    assert ref[:16].shape == (16,)
    events = [json.loads(line) for path in trace.glob("*.jsonl") for line in path.read_text().splitlines()]
    event = next(e for e in events if e["op"] == "read_range")
    assert event["bytes"] == 16 and event["checked_bytes"] == CHUNK_BYTES
