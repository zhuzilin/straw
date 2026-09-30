"""Logical records outlive any individual writer scratch buffer."""

import pytest

from straw import Publication, Record, SharedFilesystemStore
from straw.backend import FilesystemBackend


@pytest.mark.parametrize("kind", [bytes, bytearray, memoryview])
def test_large_record_and_batch_are_transparent_to_caller(tmp_path, kind):
    data = bytes(range(256)) * 2048
    store = SharedFilesystemStore(
        tmp_path,
        "stream",
        max_record_bytes=len(data),
        max_buffer_bytes=8192,
        segment_target_bytes=16384,
        online_gc=True,
    )
    refs = store.publish_many(
        [Publication((Record("large", kind(data)),)), Publication((Record("root", b"root"),), (0,))],
        submission_id="large-sample",
    )
    assert next(store.read(refs[0])).payload == data
    assert next(store.read(refs[1])).payload == b"root"
    assert refs[1].payload_bytes == len(data) + 4
    # No format change: existing extent readers and whole-file verification work.
    store.inspect_segment(refs[0].manifest.segment, verify_payload=True)
    assert (
        store.publish_many(
            [Publication((Record("large", kind(data)),)), Publication((Record("root", b"root"),), (0,))],
            submission_id="large-sample",
        )
        == refs
    )
    store.retain("checkpoint", [refs[1]])
    store.release_publications(refs)
    store.seal()
    store.collect_garbage()
    assert next(store.read(refs[0])).payload == data
    store.release("checkpoint")
    assert store.collect_garbage()["reclaimed_files"] > 0


def test_legacy_size_cap_ignored_but_noncontiguous_input_rejected(tmp_path):
    store = SharedFilesystemStore(tmp_path, "limits", max_record_bytes=1024, max_buffer_bytes=128)
    ref = store.publish([Record("large", b"a" * 1025)], submission_id="big")
    assert next(store.read(ref)).payload == b"a" * 1025
    with pytest.raises(ValueError, match="C-contiguous"):
        store.publish([Record("strided", memoryview(bytearray(10))[::2])], submission_id="strided")


@pytest.mark.parametrize("phase", ["after_payload_chunk", "before_footer", "after_data_sync"])
def test_failure_mid_large_publication_never_exposes_partial_record(tmp_path, phase):
    backend = FilesystemBackend(tmp_path)
    store = SharedFilesystemStore(tmp_path, "failure", backend=backend, max_buffer_bytes=8192)
    good = store.publish([Record("good", b"committed")], submission_id="good")

    def fail(observed):
        if observed == phase:
            raise RuntimeError("interrupted stream")

    backend.fault = fail
    with pytest.raises(RuntimeError, match="interrupted stream"):
        store.publish([Record("large", b"x" * 100_000)], submission_id="large")
    restarted = SharedFilesystemStore(tmp_path, "failure", max_buffer_bytes=8192)
    assert next(restarted.read(good)).payload == b"committed"
    final = restarted.publish([Record("new", b"z" * 100_000)], submission_id="new")
    assert next(restarted.read(final)).payload == b"z" * 100_000
    restarted.inspect_segment(final.manifest.segment, verify_payload=True)


def test_mutated_input_is_rejected_before_commit(tmp_path):
    data = bytearray(b"a" * 100_000)
    backend = FilesystemBackend(tmp_path)
    store = SharedFilesystemStore(tmp_path, "mutation", backend=backend, max_buffer_bytes=8192)

    def mutate(phase):
        if phase == "after_segment_header":
            data[:] = b"b" * len(data)

    backend.fault = mutate
    with pytest.raises(ValueError, match="changed during publication"):
        store.publish([Record("mutable", data)], submission_id="mutable")
    backend.fault = lambda _: None
    ref = store.publish([Record("mutable", data)], submission_id="retry")
    assert next(store.read(ref)).payload == bytes(data)


def test_tensor_rows_cross_write_chunks_without_client_partitioning(tmp_path):
    import torch

    from straw.tensor import TensorRef, tensor_record

    tensor = torch.arange(100_000, dtype=torch.int32).reshape(1000, 100)
    store = SharedFilesystemStore(tmp_path, "tensor", codecs=("tensor.v1",), max_buffer_bytes=8192)
    ref = store.publish([tensor_record(tensor, "tensor")], submission_id="tensor")
    lazy = TensorRef.from_record_set(store, ref)
    assert torch.equal(lazy[7:41], tensor[7:41])
    assert torch.equal(lazy.load(), tensor)


def test_tensor_checksum_metadata_stays_bounded_for_large_records(tmp_path, monkeypatch):
    import torch

    import straw.tensor as tensor_api

    monkeypatch.setattr(tensor_api, "CHUNK_BYTES", 1024)
    data = torch.arange(1024 * 1024, dtype=torch.int32)
    record = tensor_api.tensor_record(data, "adaptive")
    assert len(record.metadata["chunks"]) <= 512
    assert record.metadata["chunk_bytes"] > 1024
    store = SharedFilesystemStore(tmp_path, "adaptive", codecs=("tensor.v1",), max_buffer_bytes=8192)
    ref = store.publish([record], submission_id="adaptive")
    lazy = tensor_api.TensorRef.from_record_set(store, ref)
    assert torch.equal(lazy[100:10_000], data[100:10_000])


def test_process_exit_during_pipeline_preserves_committed_records(tmp_path):
    import subprocess
    import sys

    store = SharedFilesystemStore(tmp_path, "crash", max_buffer_bytes=8192)
    good = store.publish([Record("good", b"committed")], submission_id="good")
    code = """
import os, sys
from straw import Record, SharedFilesystemStore
from straw.backend import FilesystemBackend
seen = 0
def crash(phase):
    global seen
    if phase == "after_payload_chunk":
        seen += 1
        if seen == 2:
            os._exit(23)
store = SharedFilesystemStore(sys.argv[1], "crash", max_buffer_bytes=8192,
                              backend=FilesystemBackend(sys.argv[1], fault=crash))
store.publish([Record("incomplete", bytearray(512 * 1024))], submission_id="incomplete")
"""
    child = subprocess.run([sys.executable, "-c", code, str(tmp_path)], timeout=30)
    assert child.returncode == 23
    assert next(store.read(good)).payload == b"committed"
    recovered = SharedFilesystemStore(tmp_path, "crash", max_buffer_bytes=8192)
    new = recovered.publish([Record("new", b"z" * 100_000)], submission_id="new")
    assert next(recovered.read(new)).payload == b"z" * 100_000
    recovered.inspect_segment(new.manifest.segment, verify_payload=True)
