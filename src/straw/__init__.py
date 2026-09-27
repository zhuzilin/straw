"""Persistent work queue and replayable experience log on a shared filesystem.

The core does not import an application framework. Tensor helpers are available
from straw.tensor; see the project documentation for lifetime and GC contracts.
"""

from .coordinator import Coordinator
from .protocol import Limits, Publication, Record, RecordRef, RecordSetRef, SegmentRef, TaskSpec
from .store import SharedFilesystemStore

__all__ = [
    "Coordinator",
    "Limits",
    "Publication",
    "Record",
    "RecordRef",
    "RecordSetRef",
    "SegmentRef",
    "SharedFilesystemStore",
    "TaskSpec",
]
