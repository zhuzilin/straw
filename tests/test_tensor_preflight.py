"""Reject mismatched typed descriptors before payload work, including empties."""

import dataclasses
import json
import struct
import subprocess
import sys

import pytest
import torch

from straw import SharedFilesystemStore
from straw.errors import CorruptData
from straw.tensor import publish_tensors


@pytest.mark.parametrize("session", [False, True])
@pytest.mark.parametrize("change", [{"shape": (32, 2)}, {"dtype": "int8"}, {"nbytes": 32}])
@pytest.mark.parametrize("interval", [(0, 1), (0, 0), (64, 64)])
def test_mismatched_descriptor_precedes_payload_validation(tmp_path, capfd, session, change, interval):
    with SharedFilesystemStore(tmp_path, "typed", codecs=("tensor.v1",)) as store:
        (ref,) = publish_tensors(store, {"x": torch.arange(64, dtype=torch.uint8)}, submission_id="source")
        entry = store.inspect_segment(ref.record_ref.segment)["records"][ref.record_ref.ordinal]
        # An incorrect descriptor is an independent error. Payload corruption
        # must not cause retries before the descriptor is rejected.
        with open(ref.path, "r+b") as file:
            file.seek(ref.record_ref.segment.offset + entry["offset"])
            size, _ = struct.unpack("<IQ", file.read(12))
            file.seek(size, 1)
            file.write(b"\xff")
        bad = dataclasses.replace(ref, **change)
        with store.read_session() as reader:
            with pytest.raises(CorruptData, match="[Tt]yped tensor"):
                bad._bytes(*interval, reader=reader if session else None)
        assert "extent visibility retry" not in capfd.readouterr().err


@pytest.mark.parametrize("empty", [False, True])
def test_public_empty_slice_checks_descriptor(tmp_path, empty):
    shape = (0, 2) if empty else (32, 2)
    with SharedFilesystemStore(tmp_path, "typed", codecs=("tensor.v1",)) as store:
        (ref,) = publish_tensors(store, {"x": torch.empty(shape, dtype=torch.uint8)}, submission_id="source")
        bad = dataclasses.replace(ref, shape=(shape[0], 1, 2))
        with pytest.raises(CorruptData, match="Typed tensor"):
            bad[:0]
        assert ref[:0].shape == (0, 2)


@pytest.mark.skipif(sys.platform != "linux", reason="Uses Linux address-space accounting")
def test_bad_large_descriptor_is_rejected_under_small_reader_memory_budget(tmp_path):
    with SharedFilesystemStore(tmp_path, "typed", codecs=("tensor.v1",)) as store:
        (ref,) = publish_tensors(store, {"x": torch.arange(64 * 1024**2, dtype=torch.uint8)}, submission_id="source")
        code = """
import json, resource, sys
from pathlib import Path
from straw.errors import CorruptData
from straw.tensor import TensorRef
from straw.protocol import RecordRef, RecordSetRef
r = json.loads(sys.argv[1])
r['record_ref'] = RecordRef.from_dict(r['record_ref'])
r['blob_set'] = RecordSetRef.from_dict(r['blob_set'])
r['shape'] = (r['nbytes'] // 2, 2)
ref = TensorRef(**r)
resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
size = int(next(s.split()[1] for s in Path('/proc/self/status').read_text().splitlines() if s.startswith('VmSize:'))) * 1024
resource.setrlimit(resource.RLIMIT_AS, (size + 16 * 1024**2, size + 16 * 1024**2))
try:
    ref.load()
except CorruptData as error:
    assert 'Typed tensor' in str(error)
else:
    raise AssertionError('Accepted mismatched descriptor')
"""
        result = subprocess.run(
            [sys.executable, "-I", "-c", code, json.dumps(dataclasses.asdict(ref))], capture_output=True, text=True
        )
        assert result.returncode == 0, result.stdout + result.stderr
