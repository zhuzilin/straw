"""Inspection interface to the Rust transaction journal."""

import json

from ._native import NativeJournal
from .protocol import encode


class Journal:
    def __init__(self, backend, identity, *, recover=False, read_only=False):
        self.backend = backend
        self.path = backend.path("control/journal.log")
        self.read_only = read_only
        self._native = NativeJournal(str(backend.root), encode(identity).decode(), recover, read_only, backend.fault)
        self.stream = self

    def __getattr__(self, key):
        return json.loads(self._native.info())[key]

    def append(self, events):
        return self._native.append(encode(events).decode(), self.backend.fault)

    def close(self):
        self._native.close()
