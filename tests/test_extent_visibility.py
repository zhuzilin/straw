"""Reproduce file length becoming visible before a published extent's header."""

import os
import threading
from contextlib import contextmanager

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData


@contextmanager
def delayed_header(root, segment):
    path = root / segment.path
    with path.open("r+b") as f:
        f.seek(segment.offset)
        header = f.read(12)
        f.seek(segment.offset)
        f.write(bytes(12))
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
def test_published_zero_header_is_reopened_and_revalidated(tmp_path, capfd, entry):
    writer = SharedFilesystemStore(tmp_path, "visibility")
    writer.publish([Record("prefix", b"prefix")], submission_id="prefix")
    ref = writer.publish([Record("result", b"immutable payload")], submission_id="result")
    member = writer.validate(ref)[0]
    writer.seal()
    parent = writer.publish([Record("parent", b"parent")], dependencies=[ref], submission_id="parent")
    reader = SharedFilesystemStore(tmp_path, "visibility")
    assert ref.manifest.segment.offset > 0
    with delayed_header(tmp_path, ref.manifest.segment):
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
    assert "Straw extent visibility retry" in capfd.readouterr().err


def test_persistent_zero_header_still_fails_after_bounded_retries(tmp_path, capfd):
    store = SharedFilesystemStore(tmp_path, "persistent")
    ref = store.publish([Record("r", b"payload")], submission_id="r")
    segment = ref.manifest.segment
    with (tmp_path / segment.path).open("r+b") as f:
        f.seek(segment.offset)
        f.write(bytes(12))
    with pytest.raises(CorruptData, match="Invalid segment header"):
        store.validate(ref)
    log = capfd.readouterr().err
    assert log.count("Straw extent visibility retry") == 5
    assert "visible after" not in log


def test_nonzero_corruption_is_not_treated_as_delayed_publication(tmp_path, capfd):
    store = SharedFilesystemStore(tmp_path, "corrupt")
    ref = store.publish([Record("r", b"payload")], submission_id="r")
    segment = ref.manifest.segment
    with (tmp_path / segment.path).open("r+b") as f:
        f.seek(segment.offset)
        f.write(b"BROKENHEADER")
    with pytest.raises(CorruptData, match="Invalid segment header"):
        store.validate(ref)
    assert "visibility retry" not in capfd.readouterr().err
