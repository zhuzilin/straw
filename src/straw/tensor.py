"""NumPy/PyTorch tensor publication and checked lazy row reads.

A publication groups tensors into an existing pack. TensorRef is pickleable
metadata and does not keep a file descriptor or import any application classes.
"""

from __future__ import annotations

import dataclasses
import hashlib
import math
import sys
from pathlib import Path

import numpy as np
import torch

from .errors import CorruptData, InvalidReference, UnsupportedSchema
from .protocol import Record, RecordRef, RecordSetRef, digest
from .store import SharedFilesystemStore

DTYPES = {
    name: getattr(torch, name)
    for name in (
        "uint8",
        "int8",
        "int16",
        "int32",
        "int64",
        "float16",
        "bfloat16",
        "float32",
        "float64",
        "bool",
    )
}
CHUNK_BYTES = 4 * 1024**2
MAX_TENSOR_BYTES = 16 * 1024**3
MAX_PUBLICATION_BYTES = 16 * 1024**2


def tensor_record(value, identity, *, kind=None):
    if sys.byteorder != "little":
        raise UnsupportedSchema("tensor.v1 requires a little-endian host")
    if isinstance(value, np.ndarray):
        value = torch.from_numpy(np.array(value, copy=True))
    value = value.detach().cpu().contiguous()
    dtype = str(value.dtype).removeprefix("torch.")
    if dtype not in DTYPES:
        raise UnsupportedSchema(f"Unsupported tensor dtype: {dtype}")
    payload = memoryview(value.reshape(-1).view(torch.uint8).numpy()).cast("B") if value.numel() else memoryview(b"")
    chunk_bytes = CHUNK_BYTES
    while (len(payload) + chunk_bytes - 1) // chunk_bytes > 512:
        chunk_bytes *= 2
    chunks = [hashlib.sha256(payload[i : i + chunk_bytes]).hexdigest() for i in range(0, len(payload), chunk_bytes)]
    metadata = {
        "shape": list(value.shape),
        "dtype": dtype,
        "kind": kind,
        "chunk_bytes": chunk_bytes,
        "chunks": chunks,
    }
    return Record(identity, payload, "tensor.v1", metadata)


def release_tensor_publications(store, tensors):
    """Release staging for discarded tensor publications, preserving adopted owners.

    The caller relinquishes the entire source publication, including any sibling
    tensors published together. Other readers must already hold their own owner.
    This never unlinks a pack; normal catalog GC decides when deletion is safe.
    """
    publications = set()
    for tensor in tensors:
        if tensor.shares_storage(store):
            publication, _ = tensor.share(
                store,
                submission_id=f"discard:{digest(dataclasses.asdict(tensor.record_ref))}",
            )
            publications.add(publication)
    store.release_publications(list(publications))


@dataclasses.dataclass(frozen=True)
class TensorRef:
    """Lazy typed view of an immutable extent in a shared pack file."""

    path: str
    shape: tuple[int, ...]
    dtype: str
    nbytes: int
    kind: str | None = None
    validated: bool = False
    offset: int | None = -1
    checksum: int | None = None
    codecs: tuple[str, ...] = ("bytes.v1", "json.v1", "tensor.v1")

    def __len__(self):
        return self.shape[0]

    @property
    def torch_dtype(self):
        if self.dtype not in DTYPES:
            raise UnsupportedSchema(f"Unsupported tensor dtype: {self.dtype}")
        return DTYPES[self.dtype]

    record_ref: RecordRef | None = None
    root: str = ""
    run_id: str = ""
    blob_set: RecordSetRef | None = None
    blob_ordinal: int = 0

    def shares_storage(self, store):
        """Whether a reference can be reused without copying tensor bytes."""
        return self.run_id == store.run_id and Path(self.root).resolve() == store.backend.root

    def share(self, store, *, submission_id):
        """Return a publication dependency and ordinal for a destination queue.

        The destination publication retains this dependency before writing its
        manifest. Keep the source owner until that publication has been adopted.
        For an existing publication this only returns its descriptor; the
        destination validates it once, before durable publication/acceptance.
        """
        if not self.shares_storage(store):
            raise InvalidReference("Tensor sharing requires the same storage pool/run")
        if self.blob_set is not None:
            return self.blob_set, self.blob_ordinal
        return store.record_set([self.record_ref], submission_id=submission_id), 0

    @classmethod
    def from_record_set(cls, store, reference, ordinal=0, *, validated=False):
        members = store.manifest(reference)["records"]
        if type(ordinal) is not int or not 0 <= ordinal < len(members):
            raise InvalidReference("Tensor ordinal outside publication")
        return cls.from_record(
            store,
            RecordRef.from_dict(members[ordinal]),
            blob_set=reference,
            ordinal=ordinal,
            validated=validated,
        )

    @classmethod
    def from_record_set_many(cls, store, reference, *, validated=False, reader=None):
        """Restore tensor members by ordinal, inspecting each packed extent once.

        Non-tensor members of a mixed publication are omitted from the mapping.
        """
        if reader is not None and (reader.run_id != store.run_id or reader.backend.root != store.backend.root):
            raise InvalidReference("Tensor reader belongs to a different storage root or run")
        members = (reader or store).manifest(reference)["records"]
        indices, tensors = {}, {}
        for ordinal, member in enumerate(members):
            record = RecordRef.from_dict(member)
            if reader is not None:
                envelope = reader.envelope(record)
            else:
                if record.segment not in indices:
                    indices[record.segment] = store.inspect_segment(record.segment)
                index = indices[record.segment]
                if not 0 <= record.ordinal < len(index["records"]):
                    raise InvalidReference("Tensor ordinal outside segment")
                envelope = index["records"][record.ordinal]["envelope"]
            if envelope["codec"] == "tensor.v1":
                tensors[ordinal] = cls._from_envelope(
                    store,
                    record,
                    envelope,
                    blob_set=reference,
                    ordinal=ordinal,
                    validated=validated,
                )
        return tensors

    @classmethod
    def from_record(cls, store, reference, *, blob_set=None, ordinal=0, validated=False):
        index = store.inspect_segment(reference.segment)
        if not 0 <= reference.ordinal < len(index["records"]):
            raise InvalidReference("Tensor ordinal outside segment")
        envelope = index["records"][reference.ordinal]["envelope"]
        return cls._from_envelope(
            store,
            reference,
            envelope,
            blob_set=blob_set,
            ordinal=ordinal,
            validated=validated,
        )

    @classmethod
    def _from_envelope(cls, store, reference, envelope, *, blob_set, ordinal, validated):
        metadata = envelope["metadata"]
        if envelope["codec"] != "tensor.v1" or metadata.get("dtype") not in DTYPES:
            raise UnsupportedSchema("Expected a supported tensor.v1 record")
        shape = tuple(metadata["shape"])
        if len(shape) > 16 or any(type(n) is not int or n < 0 for n in shape):
            raise CorruptData("Invalid tensor shape")
        if math.prod(shape) * torch.empty((), dtype=DTYPES[metadata["dtype"]]).element_size() != envelope["length"]:
            raise CorruptData("Tensor shape and byte length disagree")
        return cls(
            path=str(store.backend.path(reference.segment.path)),
            shape=shape,
            dtype=metadata["dtype"],
            nbytes=envelope["length"],
            kind=metadata.get("kind"),
            validated=validated,
            codecs=tuple(sorted(store.codecs)),
            record_ref=reference,
            root=str(store.backend.root),
            run_id=store.run_id,
            blob_set=blob_set,
            blob_ordinal=ordinal,
        )

    def updated(self, store, values, *, submission_id, rows=None):
        """Copy-on-write at tensor granularity; the source remains immutable.

        Unchanged tensors should be shared. Updating rows materializes this
        tensor and writes one new tensor version, never a per-row/sample file.
        """
        if rows is None:
            value = values
        else:
            source = SharedFilesystemStore(
                self.root,
                self.run_id,
                codecs=self.codecs,
                max_record_bytes=MAX_TENSOR_BYTES,
                max_buffer_bytes=MAX_PUBLICATION_BYTES,
            )
            with source.pin([self.blob_set or self.record_ref]):
                value = self.load()
                value[rows] = torch.as_tensor(values, dtype=self.torch_dtype)
        return publish_tensors(store, {self.kind or "tensor": value}, submission_id=submission_id)[0]

    def _bytes(self, start, stop, *, reader=None):
        if reader is not None and not self.shares_storage(reader):
            raise InvalidReference("Tensor reader belongs to a different storage root or run")
        store = (
            reader
            if reader is not None
            else SharedFilesystemStore(
                self.root,
                self.run_id,
                codecs=self.codecs,
                max_record_bytes=MAX_TENSOR_BYTES,
                max_buffer_bytes=MAX_PUBLICATION_BYTES,
            )
        )
        envelope, payload = store.read_tensor_range(self.record_ref, start, stop)
        metadata = envelope["metadata"]
        if (
            envelope["codec"] != "tensor.v1"
            or metadata["dtype"] != self.dtype
            or tuple(metadata["shape"]) != self.shape
            or envelope["length"] != self.nbytes
            or math.prod(self.shape) * torch.empty((), dtype=self.torch_dtype).element_size() != self.nbytes
        ):
            raise CorruptData("Typed tensor reference disagrees with its published envelope")
        return payload

    def validate(self):
        """Check the published tensor envelope without materializing its payload."""
        self._bytes(0, 0)

    def load(self, *, pin_memory=False, reader=None):
        if self.dtype not in DTYPES:
            raise UnsupportedSchema(f"Unsupported tensor dtype: {self.dtype}")
        data = self._bytes(0, self.nbytes, reader=reader)
        tensor = (
            torch.frombuffer(data, dtype=self.torch_dtype).reshape(self.shape)
            if self.nbytes
            else torch.empty(self.shape, dtype=self.torch_dtype)
        )
        return tensor.pin_memory() if pin_memory and not tensor.is_pinned() else tensor

    def __getitem__(self, rows):
        if not isinstance(rows, slice) or rows.step not in (None, 1):
            raise TypeError("Queue tensor reads require a contiguous row slice")
        start, stop, _ = rows.indices(len(self))
        shape = (max(0, stop - start), *self.shape[1:])
        if stop <= start or not self.nbytes:
            return torch.empty(shape, dtype=self.torch_dtype)
        row_bytes = math.prod(self.shape[1:]) * torch.empty((), dtype=self.torch_dtype).element_size()
        return torch.frombuffer(self._bytes(start * row_bytes, stop * row_bytes), dtype=self.torch_dtype).reshape(
            shape
        )

    def link(self, path):
        # Compatibility no-op: packed records share through owners, not hardlinks.
        return self


def publish_tensors(store, tensors, *, submission_id, record_prefix=None):
    """Append typed blobs once; subsequent manifest publications retain refs."""
    records = [
        tensor_record(value, f"{record_prefix or submission_id}:{name}", kind=name) for name, value in tensors.items()
    ]
    blob_set = store.publish(records, submission_id=submission_id)
    refs = store.validate(blob_set)
    return tuple(
        TensorRef(
            codecs=tuple(sorted(store.codecs)),
            path=str(store.backend.path(ref.segment.path)),
            shape=tuple(record.metadata["shape"]),
            dtype=record.metadata["dtype"],
            nbytes=len(record.payload),
            kind=record.metadata["kind"],
            validated=True,
            offset=-1,
            record_ref=ref,
            root=str(store.backend.root),
            run_id=store.run_id,
            blob_set=blob_set,
            blob_ordinal=i,
        )
        for i, (record, ref) in enumerate(zip(records, refs, strict=True))
    )
