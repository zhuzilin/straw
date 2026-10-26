"""Reproduce file length becoming visible before a published extent's header."""

import os
import threading
import time
from contextlib import contextmanager

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData


@contextmanager
def restore_after_retry(restore):
    """Keep the stale view until native code actually observes and retries it.

    Forward stderr to pytest's existing capture descriptor. A wall-clock timer
    can restore before a descheduled reader starts, making the test vacuous.
    This handshake supplies a test observation, not a filesystem latency bound.
    """
    saved = os.dup(2)
    incoming, outgoing = os.pipe()
    os.dup2(outgoing, 2)
    os.close(outgoing)
    failures = []

    def observe():
        pending = b""
        restored = False
        marker = b"straw extent visibility retry"
        try:
            while chunk := os.read(incoming, 4096):
                remaining = memoryview(chunk)
                while remaining:
                    remaining = remaining[os.write(saved, remaining) :]
                pending += chunk
                if not restored and marker in pending:
                    restore()
                    restored = True
                pending = pending[-len(marker) :]
        except Exception as error:
            failures.append(error)
        finally:
            os.close(incoming)

    worker = threading.Thread(target=observe)
    worker.start()
    try:
        yield
    finally:
        # Closing the pipe writer produces EOF, even if the tested operation
        # never attempted a read or raised before producing a retry message.
        os.dup2(saved, 2)
        worker.join()
        os.close(saved)
        restore()
        if failures:
            raise failures[0]


@contextmanager
def delayed_header(root, segment, zero_positions=range(12)):
    path = root / segment.path
    with path.open("r+b") as f:
        f.seek(segment.offset)
        header = f.read(12)
        f.seek(segment.offset)
        visible_header = bytearray(header)
        for position in zero_positions:
            visible_header[position] = 0
        f.write(visible_header)
        f.flush()
        os.fsync(f.fileno())

    def restore():
        with path.open("r+b") as f:
            f.seek(segment.offset)
            f.write(header)
            f.flush()
            os.fsync(f.fileno())

    with restore_after_retry(restore):
        yield


def test_delayed_header_waits_until_reader_observes_it(tmp_path, capfd):
    with SharedFilesystemStore(tmp_path, "visibility") as store:
        ref = store.publish([Record("r", b"payload")], submission_id="source")
        segment = ref.manifest.segment
        with delayed_header(tmp_path, segment):
            # Exceed the former 50 ms restoration timer before starting I/O.
            time.sleep(0.08)
            with (tmp_path / segment.path).open("rb") as stream:
                stream.seek(segment.offset)
                assert stream.read(12) == bytes(12)
            assert store.validate(ref)
    assert "straw extent visibility retry" in capfd.readouterr().err


@pytest.mark.parametrize("entry", ["inspect", "read_record", "read", "validate", "session_read", "dependency"])
@pytest.mark.parametrize(
    "zero_positions",
    [
        tuple(range(12)),
        tuple(range(4)),
        tuple(range(4, 12)),
        (3, 4, 5),
        (0, 3, 6, 8),
        # The last three header bytes are already zero. Exercise each nonzero
        # byte independently as well as contiguous and scattered gaps above.
        *((position,) for position in range(9)),
    ],
)
def test_published_zero_header_is_reopened_and_revalidated(tmp_path, capfd, entry, zero_positions):
    writer = SharedFilesystemStore(tmp_path, "visibility")
    writer.publish([Record("prefix", b"prefix")], submission_id="prefix")
    ref = writer.publish([Record("result", b"immutable payload")], submission_id="result")
    member = writer.validate(ref)[0]
    writer.seal()
    parent = writer.publish([Record("parent", b"parent")], dependencies=[ref], submission_id="parent")
    reader = SharedFilesystemStore(tmp_path, "visibility")
    assert ref.manifest.segment.offset > 0
    with delayed_header(tmp_path, ref.manifest.segment, zero_positions):
        if entry == "inspect":
            reader.inspect_segment(ref.manifest.segment, verify_payload=True)
        elif entry == "read_record":
            assert reader.read_record(member).payload == b"immutable payload"
        elif entry == "read":
            assert list(reader.read(ref))[0].payload == b"immutable payload"
        elif entry == "validate":
            assert reader.validate(ref) == (member,)
        elif entry == "dependency":
            assert list(reader.read(parent))[0].payload == b"parent"
        else:
            with reader.read_session() as session:
                assert list(session.read(ref))[0].payload == b"immutable payload"
    assert "straw extent visibility retry" in capfd.readouterr().err


@pytest.mark.parametrize("zero_positions", [tuple(range(12)), tuple(range(4)), (3, 4, 5), (0, 3, 6, 8)])
def test_persistent_zero_header_still_fails_after_bounded_retries(tmp_path, capfd, zero_positions):
    store = SharedFilesystemStore(tmp_path, "persistent")
    ref = store.publish([Record("r", b"payload")], submission_id="r")
    segment = ref.manifest.segment
    with (tmp_path / segment.path).open("r+b") as f:
        for position in zero_positions:
            f.seek(segment.offset + position)
            f.write(b"\x00")
    with pytest.raises(CorruptData, match="Invalid segment header"):
        store.validate(ref)
    log = capfd.readouterr().err
    assert log.count("straw extent visibility retry") == 5
    assert "visible after" not in log


@pytest.mark.parametrize("corrupt_position", range(12))
def test_nonzero_corruption_is_not_treated_as_delayed_publication(tmp_path, capfd, corrupt_position):
    store = SharedFilesystemStore(tmp_path, "corrupt")
    ref = store.publish([Record("r", b"payload")], submission_id="r")
    segment = ref.manifest.segment
    with (tmp_path / segment.path).open("r+b") as f:
        f.seek(segment.offset)
        f.write(bytes(12))
        f.seek(segment.offset + corrupt_position)
        f.write(b"\xff")
    with pytest.raises(CorruptData, match="Invalid segment header"):
        store.validate(ref)
    assert "visibility retry" not in capfd.readouterr().err


def test_recovered_header_does_not_bypass_payload_validation(tmp_path, capfd):
    store = SharedFilesystemStore(tmp_path, "corrupt-payload")
    ref = store.publish([Record("r", b"immutable payload")], submission_id="r")
    segment = ref.manifest.segment
    path = tmp_path / segment.path
    data = path.read_bytes()
    payload_offset = data.index(b"immutable payload")
    with path.open("r+b") as f:
        f.seek(payload_offset)
        f.write(b"X")
    with delayed_header(tmp_path, segment, range(4)):
        with pytest.raises(CorruptData, match="checksum mismatch"):
            store.inspect_segment(segment, verify_payload=True)
    assert "visibility retry" in capfd.readouterr().err


@contextmanager
def delayed_region(path, offset, size):
    """Expose an intact header while a different committed region is stale."""
    with path.open("r+b") as f:
        f.seek(offset)
        committed = f.read(size)
        assert len(committed) == size and any(committed)
        f.seek(offset)
        f.write(bytes(size))
        f.flush()
        os.fsync(f.fileno())

    def restore():
        with path.open("r+b") as f:
            f.seek(offset)
            f.write(committed)
            f.flush()
            os.fsync(f.fileno())

    with restore_after_retry(restore):
        yield


@pytest.mark.parametrize("region", ["footer", "index", "frame", "envelope", "payload", "manifest"])
@pytest.mark.parametrize("cached", [False, True])
def test_visible_header_does_not_imply_visible_record(tmp_path, capfd, region, cached):
    writer = SharedFilesystemStore(tmp_path, "independent-regions")
    writer.publish([Record("prefix", b"prefix")], submission_id="prefix")
    ref = writer.publish([Record("r", b"immutable payload")], submission_id="r")
    member = writer.validate(ref)[0]
    segment = member.segment
    index = writer.inspect_segment(segment)
    path = tmp_path / segment.path
    data = path.read_bytes()
    footer = segment.offset + segment.size - 80
    index_length = int.from_bytes(data[footer : footer + 8], "little")
    frame = segment.offset + index["records"][member.ordinal]["offset"]
    meta_length = int.from_bytes(data[frame : frame + 4], "little")
    manifest_frame = segment.offset + index["records"][ref.manifest.ordinal]["offset"]
    manifest_meta = int.from_bytes(data[manifest_frame : manifest_frame + 4], "little")
    offset, size = {
        "footer": (footer, 80),
        "index": (footer - index_length, index_length),
        "frame": (frame, 12),
        "envelope": (frame + 12, meta_length),
        "payload": (frame + 12 + meta_length, len(b"immutable payload")),
        "manifest": (manifest_frame + 12 + manifest_meta, 8),
    }[region]
    reader = SharedFilesystemStore(tmp_path, "independent-regions")
    with reader.read_session() as session:
        if cached:
            session.envelope(member)  # authenticate/cache the index before stale data
        with delayed_region(path, offset, size):
            if region == "manifest":
                assert session.validate(ref) == (member,)
            elif cached:
                assert session.read_record(member).payload == b"immutable payload"
            else:
                assert reader.read_record(member).payload == b"immutable payload"
    # An already authenticated index need not read the footer/index again.
    if not (cached and region in {"footer", "index"}):
        assert "straw extent visibility retry" in capfd.readouterr().err


@pytest.mark.parametrize("entry", ["range", "tensor", "session_tensor"])
def test_visible_index_does_not_imply_visible_tensor_chunk(tmp_path, capfd, entry):
    import torch

    from straw.tensor import publish_tensors

    store = SharedFilesystemStore(tmp_path, "chunks", codecs=("tensor.v1",))
    (tensor,) = publish_tensors(store, {"x": torch.arange(24).reshape(6, 4)}, submission_id="x")
    ref = tensor.record_ref
    index = store.inspect_segment(ref.segment)
    envelope = index["records"][ref.ordinal]["envelope"]
    frame = ref.segment.offset + index["records"][ref.ordinal]["offset"]
    path = tmp_path / ref.segment.path
    with path.open("rb") as f:
        f.seek(frame)
        meta_length = int.from_bytes(f.read(4), "little")
    with store.read_session() as session:
        session.envelope(ref)
        with delayed_region(path, frame + 12 + meta_length, tensor.nbytes):
            if entry == "range":
                payload = store.read_range(
                    ref,
                    32,
                    64,
                    chunk_bytes=envelope["metadata"]["chunk_bytes"],
                    checksums=envelope["metadata"]["chunks"],
                )
            else:
                reader = session if entry == "session_tensor" else store
                _, payload = reader.read_tensor_range(ref, 32, 64)
            assert payload == torch.arange(4, 8).numpy().tobytes()
    assert "straw extent visibility retry" in capfd.readouterr().err


@pytest.mark.parametrize("region", ["index", "payload"])
def test_persistent_region_corruption_is_not_returned(tmp_path, capfd, region):
    store = SharedFilesystemStore(tmp_path, "persistent-region")
    ref = store.publish([Record("r", b"immutable payload")], submission_id="r")
    member = store.validate(ref)[0]
    path = tmp_path / member.segment.path
    data = path.read_bytes()
    footer = member.segment.offset + member.segment.size - 80
    index_length = int.from_bytes(data[footer : footer + 8], "little")
    offset = footer - index_length if region == "index" else data.index(b"immutable payload")
    with path.open("r+b") as f:
        f.seek(offset)
        f.write(b"\xff")
    with pytest.raises(CorruptData, match="checksum mismatch"):
        store.read_record(member)
    assert capfd.readouterr().err.count("straw extent visibility retry") == 5
