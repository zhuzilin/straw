"""Reproduce file length becoming visible before a published extent's header."""

import os
import threading
from contextlib import contextmanager

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData


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

    timer = threading.Timer(0.05, restore)
    timer.start()
    try:
        yield
    finally:
        timer.join()


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
