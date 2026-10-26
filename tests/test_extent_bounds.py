"""Malformed large lengths must fail before wraparound, allocation or writing."""

import hashlib
import json
import subprocess
import sys
from dataclasses import replace

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import CorruptData, InvalidReference
from straw.protocol import SegmentRef


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def forged_extent(root, length):
    envelope = {
        "version": 1,
        "record_id": "r",
        "codec": "bytes.v1",
        "metadata": {},
        "tokens": 0,
        "length": length,
        "checksum": hashlib.sha256(b"").hexdigest(),
    }
    # No physical record frame: a wrapped logical frame size must not make this
    # layout valid merely because the index and footer checksums are authentic.
    if length == "wrap":
        envelope["length"] = 2**64 - 1
        while True:
            n = 2**64 - 12 - len(encoded(envelope))
            if envelope["length"] == n:
                break
            envelope["length"] = n
    index = encoded(
        {"version": 1, "run_id": "bounds", "segment_id": "s", "records": [{"offset": 12, "envelope": envelope}]}
    )
    body = b"SLMSEG01\x01\0\0\0" + index
    checksum = hashlib.sha256(body).digest()
    data = body + len(index).to_bytes(8, "little") + hashlib.sha256(index).digest() + checksum + b"SLMEND01"
    (root / "bad.pack").write_bytes(data)
    return SegmentRef("bounds", "bad.pack", "s", len(data), checksum.hex(), version=2)


def test_wrapped_frame_size_is_rejected_before_index_is_cached(tmp_path):
    reference = forged_extent(tmp_path, "wrap")
    store = SharedFilesystemStore(tmp_path, "bounds")
    with pytest.raises(CorruptData, match="Index overflow|boundary"):
        store.inspect_segment(reference)


def test_publication_usage_overflow_is_not_accepted_as_zero(tmp_path):
    store = SharedFilesystemStore(tmp_path, "bounds")
    with pytest.raises(ValueError, match="overflow"):
        store.publish([Record("a", b"a", tokens=2**64 - 1), Record("b", b"b", tokens=1)], submission_id="bad")
    assert not list(tmp_path.rglob("*.pack")), "Reject before payload publication"
    good = store.publish([Record("ok", b"ok", tokens=1)], submission_id="good")
    assert good.tokens == 1


@pytest.mark.parametrize("value", [-1, 2**64, 0.5, "0", None, True])
@pytest.mark.parametrize("field", ["payload_bytes", "tokens"])
def test_invalid_unsigned_accounting_is_not_coerced_to_zero(tmp_path, field, value):
    store = SharedFilesystemStore(tmp_path, "bounds")
    valid = store.publish([Record("empty", b"")], submission_id="empty")
    with pytest.raises(InvalidReference, match="Invalid result usage"):
        store.validate(replace(valid, **{field: value}))


@pytest.mark.parametrize("warm_cache", [False, True])
def test_usage_overflow_does_not_wrap_or_poison_durable_state(tmp_path, warm_cache):
    store = SharedFilesystemStore(tmp_path, "bounds")
    queue = Coordinator(store, exclusive_owner="bounds")
    try:
        queue.submit_tasks("inputs", [TaskSpec("a", estimated_tokens=2**64 - 1), TaskSpec("b", estimated_tokens=1)])
        if warm_cache:
            assert queue.metrics()["usage_with_reservations"]["tokens"] == 0
        assert len(queue.acquire("worker", max_tasks=2).assignments) == 2
        with pytest.raises(ValueError, match="overflow"):
            queue.metrics()
        queue.cancel_task("b", request_id="cancel")
        assert queue.metrics()["usage_with_reservations"]["tokens"] == 2**64 - 1
    finally:
        queue.close()
    recovered = Coordinator(store, exclusive_owner="previous owner stopped", recover=True)
    try:
        assert recovered.task_status("b")["state"] == "cancelled"
        assert recovered.metrics()["usage_with_reservations"]["tokens"] == 0
    finally:
        recovered.close()
        store.close()


def test_dependency_and_record_set_token_overflow_rejected_before_manifest_write(tmp_path):
    store = SharedFilesystemStore(tmp_path, "bounds")
    leaf = store.publish([Record("leaf", b"leaf", tokens=2**64 - 1)], submission_id="leaf")
    refs = store.write_records(
        [Record("a", b"a", tokens=2**64 - 1), Record("b", b"b", tokens=1)], submission_id="members"
    )
    before = {p: p.stat().st_size for p in tmp_path.rglob("*.pack")}
    with pytest.raises(ValueError, match="overflow"):
        store.record_set(refs, submission_id="manifest")
    with pytest.raises(ValueError, match="overflow"):
        store.publish([Record("more", b"x", tokens=1)], dependencies=[leaf], submission_id="deps")
    assert {p: p.stat().st_size for p in tmp_path.rglob("*.pack")} == before
    assert store.validate(leaf)


def test_committed_results_survive_usage_cache_overflow_and_recovery(tmp_path):
    store = SharedFilesystemStore(tmp_path, "bounds")
    queue = Coordinator(store, exclusive_owner="bounds")
    try:
        queue.submit_tasks("tasks", [TaskSpec("a"), TaskSpec("b")])
        leases = [assignment.lease for assignment in queue.acquire("worker", max_tasks=2).assignments]
        for i, lease in enumerate(leases):
            result = store.publish(
                [
                    Record(
                        str(i),
                        b"x",
                        tokens=2**64 - 1 if i == 0 else 1,
                        metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
                    )
                ],
                submission_id=f"result-{i}",
            )
            queue.metrics()  # Exercise the cache update after a durable completion.
            queue.complete_task(lease, submission_id=f"done-{i}", result_ref=result, result_digest=result.digest)
        with pytest.raises(ValueError, match="overflow"):
            queue.metrics()
        assert len(queue.read_commits().commits) == 2
    finally:
        queue.close()
    recovered = Coordinator(store, exclusive_owner="previous owner stopped", recover=True)
    try:
        assert len(recovered.read_commits().commits) == 2
        with pytest.raises(ValueError, match="overflow"):
            recovered.metrics()
    finally:
        recovered.close()
        store.close()


@pytest.mark.parametrize("offset", [2**31 - 5, 2**32 - 5, 2**41 - 5])
def test_native_reader_authenticates_extent_across_large_offset(tmp_path, offset):
    store = SharedFilesystemStore(tmp_path, "bounds")
    (ref,) = store.write_records([Record("r", b"large offset")], submission_id="extent")
    packed = (tmp_path / ref.segment.path).read_bytes()
    with (tmp_path / "sparse.pack").open("wb") as stream:
        stream.seek(offset)
        stream.write(packed[ref.segment.offset : ref.segment.offset + ref.segment.size])
    shifted = replace(ref, segment=replace(ref.segment, path="sparse.pack", offset=offset))
    assert store.inspect_segment(shifted.segment, verify_payload=True)
    assert store.read_record(shifted).payload == b"large offset"


def test_range_crosses_verification_buffers_and_checks_unreturned_bytes(tmp_path):
    store = SharedFilesystemStore(tmp_path, "bounds")
    chunk = 3 * 1024**2 + 7
    payload = bytes(range(251)) * (2 * chunk // 251 + 1)
    (ref,) = store.write_records([Record("r", payload)], submission_id="chunks")
    checksums = [hashlib.sha256(payload[i : i + chunk]).hexdigest() for i in range(0, len(payload), chunk)]
    for start, stop in [
        (0, 0),
        (0, 13),
        (1024**2 - 3, 1024**2 + 4),
        (chunk - 3, chunk + 4),
        (len(payload) - 3, len(payload)),
    ]:
        assert store.read_range(ref, start, stop, chunk_bytes=chunk, checksums=checksums) == payload[start:stop]
    # Corrupt a distant byte in the touched chunk: tiny output must still
    # authenticate the entire chunk, including data outside the requested slice.
    entry = store.inspect_segment(ref.segment)["records"][0]
    base = ref.segment.offset + entry["offset"] + 12 + len(encoded(entry["envelope"]))
    with (tmp_path / ref.segment.path).open("r+b") as stream:
        stream.seek(base + chunk - 1)
        stream.write(bytes([payload[chunk - 1] ^ 255]))
    with pytest.raises(CorruptData, match="chunk checksum"):
        store.read_range(ref, 0, 13, chunk_bytes=chunk, checksums=checksums)


@pytest.mark.skipif(sys.platform != "linux", reason="Uses Linux address-space accounting")
def test_small_range_does_not_allocate_an_entire_large_checksum_chunk(tmp_path):
    from dataclasses import asdict

    store = SharedFilesystemStore(tmp_path, "bounds")
    payload = b"x" * (64 * 1024**2)
    (ref,) = store.write_records([Record("r", payload)], submission_id="large-chunk")
    args = json.dumps({"ref": asdict(ref), "checksum": hashlib.sha256(payload).hexdigest(), "size": len(payload)})
    # Limit only the fresh reader. Its 16-byte output plus bounded verification
    # buffer fits; allocating the entire 64 MiB chunk would exceed the limit.
    script = """
import json, resource, sys
from pathlib import Path
from straw import SharedFilesystemStore
from straw.protocol import RecordRef
data = json.loads(sys.argv[2])
store = SharedFilesystemStore(sys.argv[1], 'bounds')
ref = RecordRef.from_dict(data['ref'])
status = Path('/proc/self/status').read_text().splitlines()
size = int(next(line.split()[1] for line in status if line.startswith('VmSize:'))) * 1024
resource.setrlimit(resource.RLIMIT_AS, (size + 32 * 1024**2, size + 32 * 1024**2))
assert store.read_range(ref, 1, 17, chunk_bytes=data['size'], checksums=[data['checksum']]) == b'x' * 16
"""
    result = subprocess.run([sys.executable, "-I", "-c", script, str(tmp_path), args], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
