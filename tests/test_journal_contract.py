"""An acknowledged journal input must remain recoverable."""

import pytest

from straw.backend import FilesystemBackend
from straw.journal import Journal


@pytest.mark.parametrize("events", [[], {}, None, [None], [{"type": 7}], [{"missing_type": "x"}]])
def test_invalid_event_list_is_rejected_without_changing_journal(tmp_path, events):
    backend = FilesystemBackend(tmp_path)
    identity = {"run_id": "journal-contract"}
    journal = Journal(backend, identity)
    before = journal.path.read_bytes()
    try:
        with pytest.raises(ValueError, match="event list"):
            journal.append(events)
        assert journal.path.read_bytes() == before
        assert not journal.poisoned
        valid = [{"type": "Example", "value": "recoverable"}]
        journal.append(valid)
    finally:
        journal.close()
    recovered = Journal(backend, identity, recover=True)
    try:
        assert recovered.transactions[-1] == valid
    finally:
        recovered.close()
