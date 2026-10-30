import dataclasses
import struct
from pathlib import Path

import pytest
import torch

from straw import SharedFilesystemStore
from straw.errors import CorruptData, InvalidReference
from straw.tensor import publish_tensors


def make_store(root, run_id="reader"):
    return SharedFilesystemStore(root, run_id, codecs=("tensor.v1",), online_gc=True)


def tensors(store, name="source"):
    return publish_tensors(
        store,
        {"a": torch.arange(6).reshape(2, 3), "b": torch.ones(3)},
        submission_id=name,
    )


def flip(path, offset):
    with Path(path).open("r+b") as stream:
        stream.seek(offset)
        byte = stream.read(1)
        assert len(byte) == 1
        stream.seek(offset)
        stream.write(bytes([byte[0] ^ 1]))


def corrupt_index(ref):
    segment = ref.record_ref.segment
    with Path(ref.path).open("rb") as stream:
        stream.seek(segment.offset + segment.size - 80)
        length = int.from_bytes(stream.read(8), "little")
    flip(ref.path, segment.offset + segment.size - 80 - length)


def test_read_session_roundtrip_close_and_typed_descriptor(tmp_path):
    with make_store(tmp_path) as store:
        a, b = tensors(store)
        with store.read_session() as reader:
            assert torch.equal(a.load(reader=reader), torch.arange(6).reshape(2, 3))
            assert torch.equal(b.load(reader=reader), torch.ones(3))
            with pytest.raises(CorruptData, match="Typed tensor"):
                dataclasses.replace(a, shape=(6,)).load(reader=reader)
        reader.close()
        with pytest.raises(RuntimeError, match="closed"):
            a.load(reader=reader)
        assert a.load().shape == (2, 3)


@pytest.mark.parametrize("change", [{"checksum": "0" * 64}, {"offset": 1}, {"run_id": "wrong"}])
def test_read_session_keys_the_complete_extent_descriptor(tmp_path, change):
    with make_store(tmp_path) as store:
        a, b = tensors(store)
        altered = dataclasses.replace(
            b,
            record_ref=dataclasses.replace(
                b.record_ref,
                segment=dataclasses.replace(b.record_ref.segment, **change),
            ),
        )
        with store.read_session() as reader:
            a.load(reader=reader)
            with pytest.raises((CorruptData, InvalidReference)):
                altered.load(reader=reader)
            assert torch.equal(b.load(reader=reader), torch.ones(3))


@pytest.mark.parametrize("part", ["envelope", "payload"])
def test_cached_index_does_not_skip_record_validation(tmp_path, part):
    with make_store(tmp_path) as store:
        a, b = tensors(store)
        entry = store.inspect_segment(b.record_ref.segment)["records"][b.record_ref.ordinal]
        offset = b.record_ref.segment.offset + entry["offset"]
        with Path(b.path).open("rb") as stream:
            stream.seek(offset)
            envelope_size, _ = struct.unpack("<IQ", stream.read(12))
        with store.read_session() as reader:
            a.load(reader=reader)
            flip(b.path, offset + 12 + (envelope_size if part == "payload" else 0))
            with pytest.raises(CorruptData, match="chunk checksum|envelope differs"):
                b.load(reader=reader)


def test_new_session_rechecks_index(tmp_path):
    with make_store(tmp_path) as store:
        a, b = tensors(store)
        with store.read_session() as reader:
            a.load(reader=reader)
        corrupt_index(b)
        with store.read_session() as reader:
            with pytest.raises(CorruptData, match="index checksum"):
                b.load(reader=reader)


def test_alternating_extents_reuse_authenticated_indexes_until_eviction(tmp_path):
    with make_store(tmp_path) as store:
        a, b = tensors(store)
        other, _ = tensors(store, "other")
        with store.read_session() as reader:
            a.load(reader=reader)
            corrupt_index(b)
            other.load(reader=reader)
            assert torch.equal(b.load(reader=reader), torch.ones(3))
            # A fresh session still authenticates the on-disk index. The cache
            # authorizes only the previously authenticated exact descriptor.
            with store.read_session() as fresh, pytest.raises(CorruptData, match="index checksum"):
                b.load(reader=fresh)
            for i in range(8):
                newer, _ = tensors(store, f"newer-{i}")
                newer.load(reader=reader)
            with pytest.raises(CorruptData, match="index checksum"):
                b.load(reader=reader)


@pytest.mark.parametrize("different", ["root", "run"])
def test_reader_cannot_cross_storage_identity(tmp_path, different):
    with make_store(tmp_path / "source") as store:
        a, _ = tensors(store)
        root = tmp_path / "other" if different == "root" else tmp_path / "source"
        run = "other" if different == "run" else "reader"
        with make_store(root, run) as other, other.read_session() as reader:
            with pytest.raises(InvalidReference, match="different storage"):
                a.load(reader=reader)


def test_failed_context_closes_reader(tmp_path):
    with make_store(tmp_path) as store:
        a, _ = tensors(store)
        with pytest.raises(ValueError, match="cancel"):
            with store.read_session() as reader:
                a.load(reader=reader)
                raise ValueError("cancel")
        with pytest.raises(RuntimeError, match="closed"):
            a.load(reader=reader)


@pytest.mark.parametrize(
    "operation", ["read", "batch_read", "close", "manifest", "validate", "member_envelopes", "envelope", "read_record"]
)
def test_inherited_session_rejects_before_locking(tmp_path, operation):
    import multiprocessing

    if "fork" not in multiprocessing.get_all_start_methods():
        pytest.skip("requires fork")
    context = multiprocessing.get_context("fork")
    with make_store(tmp_path) as store:
        a, _ = tensors(store)
        with store.read_session() as reader:
            assert a.load(reader=reader).shape == (2, 3)
            receive, send = context.Pipe(duplex=False)

            def child():
                try:
                    if operation == "read":
                        reader.read_tensor_range(a.record_ref, 0, a.nbytes)
                    elif operation == "batch_read":
                        reader.read_tensor_ranges([(a.record_ref, 0, a.nbytes)])
                    elif operation == "close":
                        reader.close()
                    elif operation in {"manifest", "validate", "member_envelopes"}:
                        getattr(reader, operation)(a.blob_set)
                    else:
                        getattr(reader, operation)(a.record_ref)
                except RuntimeError as error:
                    send.send(str(error))
                else:
                    send.send("unexpected success")
                finally:
                    send.close()

            process = context.Process(target=child)
            process.start()
            send.close()
            try:
                assert receive.poll(5), "Inherited reader did not fail promptly"
                assert "cannot be reused after fork" in receive.recv()
                process.join(timeout=5)
                assert process.exitcode == 0
            finally:
                if process.is_alive():
                    process.kill()
                    process.join(timeout=5)
                receive.close()
            # The child's rejected read/close must not alter the parent session.
            assert a.load(reader=reader).shape == (2, 3)


def test_metadata_session_rechecks_authorization_and_logical_descriptors(tmp_path):
    from straw import Publication, Record

    with SharedFilesystemStore(tmp_path, "metadata") as store:
        refs = store.publish_many(
            [
                Publication(
                    (
                        Record(
                            str(i),
                            b"payload",
                            metadata={"task_id": "task", "attempt_id": "attempt"},
                        ),
                    )
                )
                for i in range(4)
            ],
            submission_id="batch",
        )
        with store.read_session() as reader:
            for ref in refs:
                members = reader.validate(ref)
                assert [r.payload for r in reader.read(ref)] == [b"payload"]
                assert reader.envelope(members[0])["metadata"]["task_id"] == "task"
                assert reader.manifest(ref)["digest"] == ref.digest
                assert reader.validate(ref, task_id="task", attempt_id="attempt") == members
                with pytest.raises(InvalidReference, match="task/attempt"):
                    reader.validate(ref, task_id="other")
                with pytest.raises(InvalidReference, match="digest"):
                    reader.validate(dataclasses.replace(ref, digest="0" * 64))
                with pytest.raises(InvalidReference, match="usage"):
                    reader.validate(dataclasses.replace(ref, payload_bytes=99))
        for call in (
            lambda: reader.manifest(refs[0]),
            lambda: reader.validate(refs[0]),
            lambda: reader.envelope(members[0]),
            lambda: reader.read_record(members[0]),
        ):
            with pytest.raises(RuntimeError, match="closed"):
                call()


@pytest.mark.parametrize("part", ["envelope", "payload"])
def test_cached_metadata_index_authenticates_every_manifest_and_record(tmp_path, part):
    from straw import Publication, Record

    with SharedFilesystemStore(tmp_path, "metadata") as store:
        a, b = store.publish_many(
            [Publication((Record("a", b"a"),)), Publication((Record("b", b"b"),))],
            submission_id="batch",
        )
        entry = store.inspect_segment(b.manifest.segment)["records"][b.manifest.ordinal]
        start = b.manifest.segment.offset + entry["offset"]
        path = tmp_path / b.manifest.segment.path
        with path.open("rb") as stream:
            stream.seek(start)
            length, _ = struct.unpack("<IQ", stream.read(12))
        with store.read_session() as reader:
            reader.validate(a)
            flip(path, start + 12 + (length if part == "payload" else 0))
            with pytest.raises(CorruptData, match="payload checksum|envelope differs"):
                reader.validate(b)


def test_tensor_metadata_uses_scoped_reader_and_rejects_wrong_pool(tmp_path):
    from straw.tensor import TensorRef

    with make_store(tmp_path / "one") as store, make_store(tmp_path / "two") as other:
        a, b = tensors(store)
        with store.read_session() as reader:
            restored = TensorRef.from_record_set_many(store, a.blob_set, reader=reader)
            assert torch.equal(restored[0].load(reader=reader), a.load())
            assert torch.equal(restored[1].load(reader=reader), b.load())
        with other.read_session() as reader:
            with pytest.raises(InvalidReference, match="different storage"):
                TensorRef.from_record_set_many(store, a.blob_set, reader=reader)


def test_bulk_tensor_metadata_preserves_mixed_member_ordinals_and_checks_payloads(tmp_path):
    from straw import Record
    from straw.tensor import TensorRef, tensor_record

    with SharedFilesystemStore(tmp_path, "mixed", codecs=("tensor.v1", "bytes.v1")) as store:
        ref = store.publish(
            [Record("text", b"text"), tensor_record(torch.arange(6), "tensor", kind="tokens")],
            submission_id="mixed",
        )
        with store.read_session() as reader:
            values = TensorRef.from_record_set_many(store, ref, reader=reader)
            assert set(values) == {1}
            assert torch.equal(values[1].load(reader=reader), torch.arange(6))
            value = values[1]
            entry = store.inspect_segment(value.record_ref.segment)["records"][value.record_ref.ordinal]
            offset = value.record_ref.segment.offset + entry["offset"]
            with Path(value.path).open("rb") as stream:
                stream.seek(offset)
                envelope_size, _ = struct.unpack("<IQ", stream.read(12))
            flip(value.path, offset + 12 + envelope_size)
            with pytest.raises(CorruptData, match="chunk checksum"):
                value.load(reader=reader)
        with pytest.raises(RuntimeError, match="closed"):
            reader.member_envelopes(ref)


def test_bulk_metadata_rechecks_manifest_payload(tmp_path):
    with make_store(tmp_path) as store:
        a, _ = tensors(store)
        with store.read_session() as reader:
            reader.member_envelopes(a.blob_set)
            ref = a.blob_set.manifest
            entry = store.inspect_segment(ref.segment)["records"][ref.ordinal]
            offset = ref.segment.offset + entry["offset"]
            with Path(a.path).open("rb") as stream:
                stream.seek(offset)
                envelope_size, _ = struct.unpack("<IQ", stream.read(12))
            flip(a.path, offset + 12 + envelope_size)
            with pytest.raises(CorruptData, match="payload checksum"):
                reader.member_envelopes(a.blob_set)
