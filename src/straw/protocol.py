"""Versioned, JSON-compatible control types; payload bytes are never RPC arguments."""

from __future__ import annotations

import hashlib
import json
from dataclasses import asdict, dataclass, field
from typing import Any

from .errors import CorruptData


def encode(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode()


def decode(value: bytes) -> Any:
    try:
        return json.loads(value, parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    except (ValueError, UnicodeError, RecursionError) as error:
        raise CorruptData("Invalid JSON metadata") from error


def digest(value: Any) -> str:
    return hashlib.sha256(encode(value)).hexdigest()


def bounded_metadata(value: Any, limit: int) -> bytes:
    """Compatibility alias for encode; the former metadata quota is ignored."""
    return encode(value)


@dataclass(frozen=True)
class SegmentRef:
    run_id: str
    path: str
    segment_id: str
    size: int
    checksum: str
    version: int = 1
    checksum_algorithm: str = "sha256"
    offset: int = 0


@dataclass(frozen=True)
class RecordRef:
    segment: SegmentRef
    ordinal: int

    @classmethod
    def from_dict(cls, value):
        return cls(SegmentRef(**value["segment"]), value["ordinal"])


@dataclass(frozen=True)
class RecordSetRef:
    """A bounded reference to an immutable ordered-record manifest."""

    manifest: RecordRef
    digest: str
    records: int
    payload_bytes: int
    tokens: int

    @classmethod
    def from_dict(cls, value):
        return cls(
            RecordRef.from_dict(value["manifest"]),
            **{k: v for k, v in value.items() if k != "manifest"},
        )


@dataclass(frozen=True)
class Record:
    record_id: str
    payload: bytes
    codec: str = "bytes.v1"
    metadata: dict = field(default_factory=dict)
    tokens: int = 0


@dataclass(frozen=True)
class TaskSpec:
    task_id: str
    input_ref: RecordSetRef | None = None
    metadata: dict = field(default_factory=dict)
    allow_empty: bool = False
    max_attempts: int = 3
    estimated_records: int = 1
    estimated_bytes: int = 0
    estimated_tokens: int = 0
    control: bool = False
    priority: int = 0
    scheduling_key: int = 0

    @classmethod
    def from_dict(cls, value):
        value = dict(value)
        if value["input_ref"] is not None:
            value["input_ref"] = RecordSetRef.from_dict(value["input_ref"])
        return cls(**value)


@dataclass(frozen=True)
class Lease:
    run_id: str
    queue_id: str
    task_id: str
    attempt_id: str
    generation: int
    coordinator_epoch: str
    token: str
    worker_id: str


@dataclass(frozen=True)
class Assignment:
    task: TaskSpec
    lease: Lease


@dataclass(frozen=True)
class AcquireResult:
    status: str  # acquired / empty / backpressured / end_of_input / draining
    assignments: tuple[Assignment, ...] = ()


@dataclass(frozen=True)
class CommitReceipt:
    commit_id: str
    task_id: str
    submission_id: str
    digest: str
    position: int
    result_ref: RecordSetRef

    @classmethod
    def from_dict(cls, value):
        value = dict(value)
        value["result_ref"] = RecordSetRef.from_dict(value["result_ref"])
        return cls(**value)


@dataclass(frozen=True)
class CommitPage:
    commits: tuple[CommitReceipt, ...]
    cursor: int
    end_of_input: bool


@dataclass(frozen=True)
class Limits:
    """Legacy queue identity fields, retained for caller/recovery compatibility.

    These values no longer reject submissions, results or control messages, or
    throttle acquisition. The application owns admission/concurrency policy;
    disk-backed result sizes are not bounded by an in-memory queue budget.
    """

    pending_tasks: int = 10000
    inflight_tasks: int = 256
    control_tasks: int = 1
    accepted_records: int = 100000
    accepted_bytes: int = 64 * 1024**3
    accepted_tokens: int = 100000000
    ready_bytes: int = 64 * 1024**3
    max_result_bytes: int = 4 * 1024**3
    max_result_tokens: int = 100000000
    metadata_bytes: int = 256 * 1024

    def __post_init__(self):
        if any(value <= 0 for value in asdict(self).values()):
            raise ValueError("Queue limits must be positive")


@dataclass(frozen=True)
class Publication:
    """One logical result in a packed segment.

    Dependencies are durable RecordSetRefs or indices of earlier Publications
    in the same publish_many call. Local indices keep digests layout-independent.
    """

    records: tuple[Record, ...]
    dependencies: tuple[RecordSetRef | int, ...] = ()
