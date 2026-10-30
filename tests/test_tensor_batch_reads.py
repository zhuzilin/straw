import dataclasses
import struct
from pathlib import Path

import pytest
import torch

from straw import SharedFilesystemStore
from straw.errors import CorruptData, InvalidReference
from straw.tensor import TensorRef, publish_tensors


def store(root):
    return SharedFilesystemStore(root, "batched", codecs=("tensor.v1",))


@pytest.mark.parametrize("workers", [1, 2, 4])
def test_batch_order_empty_tensors_and_independent_buffers(tmp_path, workers):
    with store(tmp_path) as source:
        a, b = publish_tensors(source, {"a": torch.arange(12).reshape(4, 3), "b": torch.empty(0)}, submission_id="one")
        (c,) = publish_tensors(source, {"c": torch.arange(3, dtype=torch.float32)}, submission_id="two")
        refs = [c, a, b, a, c]
        expected = [ref.load() for ref in refs]
        with source.read_session() as reader:
            values = TensorRef.load_many(refs, reader=reader, workers=workers)
            assert all(torch.equal(value, old) for value, old in zip(values, expected, strict=True))
            assert TensorRef.load_many([], reader=reader, workers=workers) == []
            values[1][0, 0] = -10
            assert values[3][0, 0] == 0
        with pytest.raises(RuntimeError, match="closed"):
            TensorRef.load_many([a], reader=reader, workers=workers)


@pytest.mark.parametrize("workers", [1, 4])
@pytest.mark.parametrize("part", ["payload", "envelope", "shape", "dtype", "root"])
def test_batch_retains_individual_record_and_typed_validation(tmp_path, workers, part):
    with store(tmp_path) as source:
        a, b = publish_tensors(source, {"a": torch.arange(12), "b": torch.ones(4)}, submission_id="one")
        if part in {"payload", "envelope"}:
            entry = source.inspect_segment(b.record_ref.segment)["records"][b.record_ref.ordinal]
            offset = b.record_ref.segment.offset + entry["offset"]
            with Path(b.path).open("r+b") as stream:
                stream.seek(offset)
                length, _ = struct.unpack("<IQ", stream.read(12))
                target = offset + 12 + (length if part == "payload" else 0)
                stream.seek(target)
                byte = stream.read(1)
                stream.seek(target)
                stream.write(bytes([byte[0] ^ 1]))
        elif part == "shape":
            b = dataclasses.replace(b, shape=(2, 2))
        elif part == "dtype":
            b = dataclasses.replace(b, dtype="int32")
        else:
            b = dataclasses.replace(b, root=str(tmp_path / "other"))
        with source.read_session() as reader:
            with pytest.raises((CorruptData, InvalidReference)):
                TensorRef.load_many([a, b], reader=reader, workers=workers)


def test_batch_reader_worker_bounds(tmp_path):
    with store(tmp_path) as source:
        with source.read_session() as reader:
            for workers in (0, 33):
                with pytest.raises(ValueError, match="between 1 and 32"):
                    TensorRef.load_many([], reader=reader, workers=workers)


def test_batch_typed_preflight_precedes_corrupt_payload(tmp_path):
    with store(tmp_path) as source:
        (a,) = publish_tensors(source, {"a": torch.arange(12)}, submission_id="one")
        entry = source.inspect_segment(a.record_ref.segment)["records"][a.record_ref.ordinal]
        with Path(a.path).open("r+b") as stream:
            stream.seek(a.record_ref.segment.offset + entry["offset"])
            length, _ = struct.unpack("<IQ", stream.read(12))
            stream.seek(length, 1)
            stream.write(b"bad payload")
        with source.read_session() as reader:
            with pytest.raises(CorruptData, match="Typed tensor reference"):
                TensorRef.load_many([dataclasses.replace(a, shape=(3, 4))], reader=reader)


def test_batch_retry_closes_cached_view_before_wait_and_reopens(tmp_path, capfd):
    import os
    import time
    from concurrent.futures import ThreadPoolExecutor

    with store(tmp_path) as source:
        a, b = publish_tensors(source, {"a": torch.arange(12), "b": torch.arange(8)}, submission_id="one")
        entry = source.inspect_segment(b.record_ref.segment)["records"][b.record_ref.ordinal]
        path = Path(b.path)
        original = path.read_bytes()
        reader = source.read_session()
    with path.open("r+b") as stream:
        stream.seek(b.record_ref.segment.offset + entry["offset"])
        length, _ = struct.unpack("<IQ", stream.read(12))
        stream.seek(length, 1)
        stream.write(b"corrupt!")
    inode = path.stat().st_ino

    def repair():
        deadline = time.monotonic() + 10
        output = ""
        while time.monotonic() < deadline:
            output += capfd.readouterr().err
            if "extent visibility retry" in output:
                for fd in Path("/proc/self/fd").iterdir():
                    try:
                        assert fd.stat().st_ino != inode, "failed file view remained open during retry wait"
                    except FileNotFoundError:
                        pass
                replacement = tmp_path / "repaired.pack"
                replacement.write_bytes(original)
                os.replace(replacement, path)
                return
            time.sleep(0.002)
        raise AssertionError("No failing read was observed")

    with reader as reader, ThreadPoolExecutor(max_workers=1) as pool:
        future = pool.submit(repair)
        values = TensorRef.load_many([a, b], reader=reader, workers=1)
        future.result()
        assert torch.equal(values[0], torch.arange(12))
        assert torch.equal(values[1], torch.arange(8))
