"""Typed Python boundary for the Rust packed-record store.

Payloads can be bytes, NumPy buffers, or contiguous PyTorch CPU byte views.
Rust owns validation, framing, hashes, durable publication and checked reads.
"""

import contextlib
import json
import time
import uuid
from dataclasses import asdict

from ._native import MAX_RECORDS as MAX_RECORDS
from ._native import NativeStore
from .backend import FilesystemBackend
from .protocol import Publication, Record, RecordRef, RecordSetRef, SegmentRef, encode
from .tracing import emit


def _read_tensor_range(native, root, ref, start, stop):
    started = time.monotonic()
    envelope, payload, checked = native.read_tensor_range(encode(asdict(ref)).decode(), start, stop)
    emit(
        "read_range",
        root=root,
        size=len(payload),
        checked_bytes=checked,
        seconds=time.monotonic() - started,
    )
    return json.loads(envelope), bytearray(payload)


class _RecordReader:
    def read_record(self, ref):
        started = time.monotonic()
        envelope, payload = self._native.read(encode(asdict(ref)).decode())
        envelope = json.loads(envelope)
        emit(
            "read",
            root=self.backend.root,
            size=len(payload),
            seconds=time.monotonic() - started,
        )
        return Record(
            envelope["record_id"],
            payload,
            envelope["codec"],
            envelope["metadata"],
            envelope["tokens"],
        )

    def manifest(self, ref):
        return json.loads(self._native.manifest(encode(asdict(ref)).decode()))

    def validate(self, ref, *, task_id=None, attempt_id=None):
        return tuple(
            RecordRef.from_dict(r)
            for r in json.loads(self._native.validate(encode(asdict(ref)).decode(), task_id, attempt_id))
        )

    def read(self, ref):
        for member in self.validate(ref):
            yield self.read_record(member)


class _ReadSession(_RecordReader):
    def __init__(self, store):
        self.backend, self.run_id = store.backend, store.run_id
        self._native = store._native.read_session()

    def read_tensor_range(self, ref, start, stop):
        return _read_tensor_range(self._native, self.backend.root, ref, start, stop)

    def envelope(self, ref):
        return json.loads(self._native.envelope(encode(asdict(ref)).decode()))

    def close(self):
        self._native.close()


class SharedFilesystemStore(_RecordReader):
    def __init__(
        self,
        root,
        run_id,
        *,
        backend=None,
        codecs=("bytes.v1", "json.v1"),
        max_record_bytes=256 * 1024**2,
        max_buffer_bytes=512 * 1024**2,
        segment_target_bytes=1024**3,
        online_gc=False,
    ):
        self.backend = backend or FilesystemBackend(root)
        if self.backend.root != FilesystemBackend(root).root:
            raise ValueError("Backend root differs from store root")
        self.run_id = run_id
        self.codecs = frozenset(codecs) | {"record-set.v1", "record-set.v2"}
        self.max_record_bytes, self.max_buffer_bytes = (
            max_record_bytes,
            max_buffer_bytes,
        )
        self.segment_target_bytes = segment_target_bytes
        self._config = json.dumps(
            dict(
                root=str(self.backend.root),
                run_id=run_id,
                codecs=sorted(self.codecs),
                max_record_bytes=max_record_bytes,
                max_buffer_bytes=max_buffer_bytes,
                segment_target_bytes=segment_target_bytes,
                online_gc=online_gc,
            )
        )
        self._native = NativeStore(self._config)

    def _fault(self, phase):
        self.backend.fault(phase)
        if phase in {"before_file_sync", "before_directory_sync"}:
            self.backend.metrics["syncs"] += 1

    @property
    def metrics(self):
        return json.loads(self._native.metrics())

    def close(self):
        self._native.close()

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        self.close()

    def seal(self):
        """Seal the current pack without closing this writer."""
        self._native.seal(self._fault)

    def _storage(self, method, **values):
        return json.loads(self._native.storage(method, encode(values).decode(), self._fault))

    def retain(self, owner, refs):
        """Replace a durable owner's roots. Retain the destination before release."""
        return self._storage("retain", owner=owner, refs=[asdict(r) for r in refs])

    def release(self, owner):
        """Release one explicit owner; other queues/readers/checkpoints keep theirs."""
        return self._storage("release", owner=owner)

    def release_publications(self, refs):
        """Consume staging ownership after a durable owner has adopted the roots.

        Also consumes staging for dependencies. Calling this without another
        owner explicitly discards the publication and its unpublished children.
        """
        return self._storage("consume", refs=[asdict(r) for r in refs])

    @contextlib.contextmanager
    def pin(self, refs, *, owner=None):
        """Protect a reader or an external operation across multiple storage calls."""
        owner = owner or f"reader:{uuid.uuid4().hex}"
        self.retain(owner, refs)
        try:
            yield
        finally:
            self.release(owner)

    def collect_garbage(self):
        """Unlink only sealed packs with no durable owner; retry old tombstones."""
        return self._storage("collect")

    def _inputs(self, records):
        metadata, payloads = [], []
        for record in records:
            if not isinstance(record.payload, (bytes, bytearray, memoryview)):
                raise TypeError("Store payloads must support a contiguous byte buffer")
            metadata.append(
                dict(
                    record_id=record.record_id,
                    codec=record.codec,
                    metadata=record.metadata,
                    tokens=record.tokens,
                )
            )
            payloads.append(record.payload)
        return encode(metadata).decode(), payloads

    def write_records(self, records, *, submission_id):
        metadata, payloads = self._inputs(records)
        started = time.monotonic()
        result = tuple(
            RecordRef.from_dict(r)
            for r in json.loads(self._native.write(metadata, payloads, submission_id, self._fault))
        )
        emit(
            "write",
            root=self.backend.root,
            size=result[0].segment.size,
            seconds=time.monotonic() - started,
            records=records,
        )
        return result

    def publish(self, records, *, submission_id, dependencies=()):
        return self.publish_many(
            [Publication(tuple(records), tuple(dependencies))],
            submission_id=submission_id,
        )[0]

    def publish_many(self, publications, *, submission_id):
        records, groups = [], []
        for publication in publications:
            start = len(records)
            records.extend(publication.records)
            groups.append(
                dict(
                    records=list(range(start, len(records))),
                    dependencies=[dep if type(dep) is int else asdict(dep) for dep in publication.dependencies],
                )
            )
        metadata, payloads = self._inputs(records) if records else ("[]", [])
        started = time.monotonic()
        result = tuple(
            RecordSetRef.from_dict(r)
            for r in json.loads(
                self._native.publish(
                    encode(groups).decode(),
                    metadata,
                    payloads,
                    submission_id,
                    self._fault,
                )
            )
        )
        emit(
            "write",
            root=self.backend.root,
            size=result[0].manifest.segment.size,
            seconds=time.monotonic() - started,
            records=records,
        )
        return result

    def inspect_segment(self, ref, *, verify_payload=False):
        return json.loads(self._native.inspect(encode(asdict(ref)).decode(), verify_payload))

    def read_range(self, ref, start, stop, *, chunk_bytes, checksums):
        started = time.monotonic()
        payload, checked = self._native.read_range_stats(
            encode(asdict(ref)).decode(), start, stop, chunk_bytes, checksums
        )
        emit(
            "read_range",
            root=self.backend.root,
            size=len(payload),
            checked_bytes=checked,
            seconds=time.monotonic() - started,
        )
        return bytearray(payload)

    def read_tensor_range(self, ref, start, stop):
        """Return the authenticated envelope and touched tensor chunk bytes."""
        return _read_tensor_range(self._native, self.backend.root, ref, start, stop)

    @contextlib.contextmanager
    def read_session(self):
        """Reuse the last authenticated immutable extent index for one operation.

        Record envelopes and touched chunks are checked on every read. This
        does not retain storage ownership; callers must keep their existing pin
        or durable owner until all reads finish. Closing drops the cached index.
        Sessions belong to their creating process and must not cross a fork.
        """
        reader = _ReadSession(self)
        try:
            yield reader
        finally:
            reader.close()

    def read(self, ref):
        # One iterator owns one bounded session; keep record and manifest
        # authentication while avoiding an index parse for every member.
        with self.read_session() as reader:
            yield from reader.read(ref)

    def record_set(self, refs, *, submission_id, dependencies=()):
        return RecordSetRef.from_dict(
            json.loads(
                self._native.record_set(
                    encode([asdict(r) for r in refs]).decode(),
                    encode([asdict(d) for d in dependencies]).decode(),
                    submission_id,
                    self._fault,
                )
            )
        )

    def discover(self, partition):
        # Discovery never establishes logical acceptance of an orphan.
        for reference in json.loads(self._native.discover(partition)):
            yield SegmentRef(**reference)
