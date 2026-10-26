"""Native WAL recovery must leave a durable, appendable frame boundary."""

import errno

import pytest

from straw.backend import FilesystemBackend
from straw.errors import IndeterminateCommit, UnsafeRecovery
from straw.journal import Journal


def test_every_torn_tail_recovers_then_appends_and_reopens(tmp_path):
    backend = FilesystemBackend(tmp_path)
    identity = {"run_id": "wal-prefix"}
    event = [{"type": "Example", "request": "uncertain"}]
    later = [{"type": "Example", "request": "after-recovery"}]
    journal = Journal(backend, identity)
    before = journal.path.read_bytes()
    assert journal.append(event) == 1
    path = journal.path
    journal.close()
    after = path.read_bytes()
    frame_size = len(after) - len(before)
    for length in range(frame_size + 1):
        path.write_bytes(after[: len(before) + length])
        recovered = Journal(backend, identity, recover=True)
        try:
            complete = length == frame_size
            expected = [[{"type": "Run", "identity": identity}]] + ([event] if complete else [])
            assert recovered.transactions == expected
            assert path.read_bytes() == (after if complete else before)
            assert recovered.append(later) == len(expected)
        finally:
            recovered.close()
        reopened = Journal(backend, identity, recover=True, read_only=True)
        try:
            assert reopened.transactions == expected + [later]
            assert reopened.incomplete_tail_bytes == 0
        finally:
            reopened.close()


@pytest.mark.parametrize(
    "phase,occurrence,complete",
    [
        ("before_journal_write", 1, False),
        ("after_journal_part", 1, False),
        ("after_journal_part", 2, False),
        ("after_journal_part", 3, True),
        ("before_journal_sync", 1, True),
        ("before_file_sync", 1, True),
        ("after_journal_sync", 1, True),
    ],
)
def test_uncertain_append_requires_recovery_before_next_append(tmp_path, phase, occurrence, complete):
    backend = FilesystemBackend(tmp_path)
    identity = {"run_id": "wal-errors"}
    journal = Journal(backend, identity)
    event = [{"type": "Example", "request": "uncertain"}]
    later = [{"type": "Example", "request": "later"}]
    seen = 0

    def fault(at):
        nonlocal seen
        if at == phase:
            seen += 1
            if seen == occurrence:
                raise OSError(errno.EIO, "uncertain WAL append")

    backend.fault = fault
    try:
        with pytest.raises(IndeterminateCommit):
            journal.append(event)
        assert seen == occurrence
        with pytest.raises(UnsafeRecovery):
            journal.append(later)
    finally:
        journal.close()
    backend.fault = lambda _: None
    recovered = Journal(backend, identity, recover=True)
    try:
        expected = [[{"type": "Run", "identity": identity}]] + ([event] if complete else [])
        assert recovered.transactions == expected
        assert recovered.append(later) == len(expected)
    finally:
        recovered.close()
    reopened = Journal(backend, identity, recover=True, read_only=True)
    try:
        assert reopened.transactions == expected + [later]
    finally:
        reopened.close()
