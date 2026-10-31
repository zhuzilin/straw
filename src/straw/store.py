"""Typed Python boundary for the Rust packed-record store.

Payloads can be bytes, NumPy buffers, or contiguous PyTorch CPU byte views.
Rust owns validation, framing, hashes, durable publication and checked reads.
"""

import contextlib
import json
import threading
import time
import uuid
from dataclasses import asdict

from ._native import MAX_RECORDS as MAX_RECORDS
from ._native import NativeStore
from .backend import FilesystemBackend
from .protocol import Publication, Record, RecordRef, RecordSetRef, SegmentRef, encode
from .tracing import emit


def _read_tensor_range(native, root, ref, start, stop, expected=None):
    started = time.monotonic()
    envelope, payload, checked = native.read_tensor_range(
        encode(asdict(ref)).decode(), start, stop, None if expected is None else encode(expected).decode()
    )
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

    def read_tensor_range(self, ref, start, stop, *, expected=None):
        return _read_tensor_range(self._native, self.backend.root, ref, start, stop, expected)

    def envelope(self, ref):
        return json.loads(self._native.envelope(encode(asdict(ref)).decode()))

    def member_envelopes(self, ref, *, codec=None):
        """Read authenticated member descriptors through one native boundary."""
        return json.loads(self._native.member_envelopes(encode(asdict(ref)).decode(), codec))

    def read_tensor_ranges(self, requests, *, workers=4, expected=None, read_ahead_bytes=1024 * 1024):
        """Read an ordered batch with up to 8 MiB of read-ahead per worker.

        Zero read-ahead selects ordinary scalar reads for comparison. Typed
        descriptors, when supplied, are checked before payload allocation.
        """
        started = time.monotonic()
        envelopes, payloads, checked = self._native.read_tensor_ranges(
            encode([(asdict(ref), start, stop) for ref, start, stop in requests]).decode(),
            workers,
            None if expected is None else encode(expected).decode(),
            read_ahead_bytes,
        )
        emit(
            "read_ranges",
            root=self.backend.root,
            size=sum(map(len, payloads)),
            checked_bytes=checked,
            seconds=time.monotonic() - started,
        )
        return list(zip(json.loads(envelopes), map(bytearray, payloads), strict=True))

    def close(self):
        self._native.close()


class SharedFilesystemStore(_RecordReader):
    """Immutable payload storage with bounded copy scratch.

    max_record_bytes is retained for API compatibility and is not enforced.
    max_buffer_bytes controls temporary copy memory, never logical record size.
    """

    supports_writer_staging_discard = True

    def __init__(
        self,
        root,
        run_id,
        *,
        backend=None,
        codecs=("bytes.v1", "json.v1"),
        max_record_bytes=16 * 1024**3,
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
        self._validation_context = threading.local()

    def _active_validation_scope(self):
        return getattr(self._validation_context, "scope", None)

    def _invalidate_validation_scope(self):
        scope = self._active_validation_scope()
        if scope is not None:
            scope.close()

    @contextlib.contextmanager
    def validation_scope(self, coordinator):
        """Reuse immutable graph checks only across one local ready transition.

        Publications remain staged or accepted while this scope is live. Explicit
        reads/validate calls keep their normal checks. A failed operation closes
        the scope immediately; exiting also destroys all dependency proofs.
        """
        if coordinator.store is not self:
            raise ValueError("Validation scope requires the coordinator's own store")
        if (
            self._active_validation_scope() is not None
            or getattr(coordinator._validation_context, "scope", None) is not None
        ):
            raise RuntimeError("Nested validation scopes are not supported")
        scope = self._native.validation_scope()
        self._validation_context.scope = coordinator._validation_context.scope = scope
        try:
            yield scope
        finally:
            self._validation_context.scope = coordinator._validation_context.scope = None
            scope.close()

    def validate(self, ref, *, task_id=None, attempt_id=None):
        try:
            return super().validate(ref, task_id=task_id, attempt_id=attempt_id)
        except BaseException:
            self._invalidate_validation_scope()
            raise

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

    @property
    def writer_id(self):
        """Persistent incarnation used to explicitly retire this writer."""
        return self._native.writer_id()

    def retire_writer(self, writer_id, *, policy="recover", discard_staging=False):
        """Fence an exact incarnation and reconcile its unfinished publications.

        A busy writer raises WriterBusy. Recovery returns staged results;
        explicit abandonment preserves existing queue, reader and staging roots.
        The same writer cannot resume after retirement. Create a fresh writer.
        discard_staging=True explicitly relinquishes that writer's registered
        transfer publications after retirement. The supervisor must first stop
        its readers and adopt accepted results. Other durable owners survive;
        returned references are not protected by staging in this mode.
        """
        values = json.loads(self._native.retire_writer(writer_id, policy, self._fault, discard_staging))
        return [
            RecordSetRef.from_dict(value) if "manifest" in value else RecordRef.from_dict(value) for value in values
        ]

    def publication_receipt(self, writer_id, *, api, submission_id):
        """Look up a recorded result, including after reply loss or retirement.

        ``api`` is ``records`` (write_records), ``record_set``, or ``publish``
        (publish/publish_many). Returns the latest recorded result list for the
        exact writer/API/submission ID, or None when no result was recorded.
        This only reads the receipt: it does not renew data protection, so old
        references may already have been consumed and collected. A writer
        holding the gate raises WriterBusy; fenced writers can still be queried.
        """
        values = json.loads(self._native.publication_receipt(writer_id, api, submission_id, self._fault))
        if values is None:
            return None
        return [
            RecordSetRef.from_dict(value) if "manifest" in value else RecordRef.from_dict(value) for value in values
        ]

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
        """Reclaim unowned sealed packs; retry deletions without a durable receipt."""
        return self._storage("collect")

    def retention_summary(self):
        """Report owner/pack metadata with bounded examples and no payload reads.

        Category pack counts can overlap and must not be added together.
        Open writers and retained ownership require lifecycle confirmation;
        elapsed file age alone does not authorize reclamation.
        """
        return self._storage("retention_summary")

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
        try:
            return self._publish_many(publications, submission_id=submission_id)
        except BaseException:
            self._invalidate_validation_scope()
            raise

    def _publish_many(self, publications, *, submission_id):
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
                    self._active_validation_scope(),
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

    def read_tensor_range(self, ref, start, stop, *, expected=None):
        """Return the authenticated envelope and touched tensor chunk bytes."""
        return _read_tensor_range(self._native, self.backend.root, ref, start, stop, expected)

    @contextlib.contextmanager
    def read_session(self):
        """Reuse a bounded LRU of authenticated immutable extent indexes.

        Record envelopes and touched chunks are checked on every read. This
        does not retain storage ownership; callers must keep their existing pin
        or durable owner until all reads finish. Closing drops the cached indexes.
        Sessions belong to their creating process and must not cross a fork.
        Calls within a session serialize; use independent sessions for parallel reads.
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
