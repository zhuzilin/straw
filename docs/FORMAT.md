# Packed records and queue journal

[中文版](FORMAT_zh.md)

All integer framing fields are unsigned little-endian. JSON is UTF-8, sorted
keys, compact separators, with non-finite numbers rejected. The core stores
opaque bytes and explicitly configured codec names; it never unpickles payloads.
Unknown framing/schema/codec versions fail closed. Version 1 is uncompressed;
offsets refer to physical bytes. Changing these rules requires a new version.

## Immutable extents in append packs

Each writer incarnation owns a UUID directory. With a nonzero segment target,
files are `raw/<writer-prefix>/<writer-UUID>/<pack-UUID>.pack`. Each publication
appends a self-contained extent with the framing below. `SegmentRef.version=2`
identifies `(path, offset, size, checksum)`; `size` is the extent length, not the
current file length. Later appends cannot change an earlier extent. A writer
rotates when the next publication would exceed the target; one oversized
publication may exceed the target. Restarted writers allocate a new incarnation
and never reuse a possibly interrupted tail.

Target zero selects standalone `.sealed` files with `SegmentRef.version=1`, offset
zero and an exact file-size check. Readers accept both representations. Pack
publication fsyncs the file and directory before returning a reference. An
uncertain sync fences the writer until retrying the same publication identity
resolves it. No temporary file, manifest file or directory is allocated per sample.

Byte layout:

| Part | Encoding |
|---|---|
| Header | 8 bytes `SLMSEG01`, u32 version = 1 |
| Repeated frame | u32 envelope length, u64 payload length, envelope JSON, payload bytes |
| Index | JSON object described below |
| Trailer | u64 index length, 32-byte index SHA-256, 32-byte body SHA-256, 8 bytes `SLMEND01` |

The body SHA-256 covers all bytes before the trailer (header, every frame, and
index). The index SHA-256 covers only the index JSON bytes. An envelope contains
`version`, `record_id`, `codec`, `metadata`, `tokens`, `length` and hex SHA-256
`checksum` of the payload. Empty payloads are valid, with the SHA-256 of empty
bytes. Empty physical segments are disallowed; empty logical results have an
explicit manifest containing zero records.

The index contains `version`, `run_id`, `segment_id` and an ordered `records`
array of `{offset, envelope}`. Every offset points to the frame's length fields.
Frames must exactly cover the region between header and index, without gaps,
overlap, or trailing garbage. Length, file size, framing, identity and checksums
are checked before returning a record. Maximums are 10,000 records, 64 KiB per
envelope, 8 MiB index, and by default 256 MiB per record / 512 MiB per write.

`SegmentRef` includes run ID, relative POSIX path, segment ID, extent size,
body checksum, checksum algorithm and format version. Absolute paths, `..`,
empty path components and symlinks escaping the local run root are rejected.
`RecordRef` adds a zero-based ordinal. A client chooses its own mount root;
absolute machine paths never appear in these references.

Standalone sealed publication writes a private file, flushes and fsyncs it, checks close, renames
to the unique sealed path, then fsyncs its directory. Newly created directories
and their parents are synced too. A rename error triggers verification of the
same publication identity, including its body checksum, before retrying directory
sync. Unresolved outcomes return `IndeterminateCommit`. No successful result is
returned when file/directory sync fails. Each data writer owns its file; GC-enabled
pools additionally require the shared catalog's advisory lock.

## Ordered record sets

`RecordSetRef` is bounded control metadata: one manifest RecordRef plus logical
digest, record count, payload-byte count and token count. Byte/token budgets
include declared dependencies transitively (shared dependencies may be counted
conservatively more than once). Its manifest is a
`record-set.v1` record containing `version`, ordered `records`, ordered
`dependencies` (other RecordSetRefs), and `digest`.

Ordinary publications use `record-set.v2` manifests in the same extent as their
records. Local members are earlier record ordinals; local dependencies carry
an earlier manifest ordinal plus bounded logical descriptor. Public readers
normalize these into ordinary RecordRefs and RecordSetRefs. External subsets
can still use `record-set.v1`. Both paths compute logical digests in Rust.

The logical digest is SHA-256 of Rust serde_json compact JSON with sorted object keys:

```
{"records": [envelope, ...], "dependencies": [logical dependency digest, ...]}
```

Thus a physical re-layout preserves logical identity. Order, record identity,
task/attempt metadata, codec, tokens and payload content remain significant.
External dependencies must already be durable. Embedded dependencies precede their parent manifest within the extent. The entire
result is accepted in one coordinator transaction; sharing a physical segment
does not merge tasks. Manifest/dependency traversal is bounded to 10,000 roots.

The coordinator validates small manifests and indices under a trusted-writer
contract. Readers checksum the actual bytes they consume. Sealed orphan data
are never inferred to be logically accepted.

## Authoritative journal

Only the externally guaranteed single coordinator writes `control/journal.log`.
It is opened afresh on recovery. Consumers access the coordinator API rather
than tailing a long-lived file descriptor across mounts.

Every transaction consists of:

| Part | Encoding |
|---|---|
| Prefix | 8 bytes `SLMTXN01`, u64 sequence, u64 JSON payload length |
| Header checksum | 32 bytes SHA-256 of the prefix |
| Payload | Nonempty JSON array of typed state-transition events, at most 8 MiB |
| Commit trailer | 32 bytes SHA-256 of prefix + header checksum + payload, 8 bytes `SLMTEND1` |

Sequences start at zero and are contiguous. Transaction zero binds run ID,
queue ID, schema, limits, lease duration and backend profile to `run.json`.
There is no journal rotation. Batch acquire and batch submit each use one
transaction/sync. Single-task completion atomically includes the terminal task,
result reference, receipt, deduplication key and accepted-log position.

`Submitted` can additionally contain a producer ID and bounded versioned
producer state, so advancing a dataset cursor and submitting its tasks are one
durable transaction. `Yielded` saves an immutable continuation input and returns
a task to pending without accepting an output. A voluntary continuation does
not spend a failure retry; expired/revoked/failed attempts do. Every new acquire
still advances the authorization generation and changes attempt/token identity.

The live instance serializes validation → append → fsync → apply → reply.
Any uncertain append/sync poisons it. Recovery rejects complete checksum errors,
sequence gaps, bad headers or changed run identity. Only an incomplete final
transaction may be truncated. Complete surviving transactions are synced before
being exposed, including transactions whose reply was lost. A new epoch and
revocation of old outstanding leases are persisted before service resumes.

## State and retention

Task inputs and completed results, batch plans/outputs, opaque consumer states,
and checkpoint roots are explicit RecordSetRefs. Fetch cursor, contiguous
processed prefix, batch-ready state and checkpoint coverage are separate.
Noncontiguous unfinished work belongs in the opaque versioned consumer state.
An optional consumer `progress_ref` names one `json.v1` record with `version: 1`,
`processed_positions` (unique sparse positions between processed and fetch
cursors), and `finished_batches` (unique ready batch IDs). The single designated
`training` flow releases production budgets through its prefix and sparse
positions. Finished batches release runtime ready capacity without registering a
checkpoint. In GC-enabled pools, collection can then retire their data roots.
Restoring a consumer view does not resurrect reclaimed data: the checkpoint
must have retained its dependencies. Independent replay cursors are not pins.

`TaskSpec.control` defaults to false. Control tasks have zero production
estimates and a separately bounded pending/in-flight pool (`control_tasks`,
default one). They require explicit `acquire(control=True)` and can finish a
collection of already-produced data when production budgets are full. Ordinary
workers cannot acquire them through their default acquire call. Completed
control results still enter the same accepted log and accounting. This is a
bounded drain path, not an unlimited bypass for generation work.

Applications can record filtering, batching and checkpoint-branch decisions in
immutable manifests referenced by consumer state. Task semantics, provenance
and the meaning of a resumed application checkpoint remain application-owned.

Version 1 replays the complete journal; snapshots are immutable inspection
artifacts, not authoritative startup heads. Deduplication history is never
pruned. Offline cleanup retains every historical journal reference and its
dependencies, retaining a whole segment when any record in it is reachable.

## Numeric metadata compatibility

Python and Rust format some floating-point JSON numbers differently. Logical
hashes and manifests are generated in Rust. Clients must use the native digest
implementation rather than recomputing hashes from Python-serialized JSON.

## Typed tensors

`tensor.v1` carries little-endian contiguous CPU bytes with dtype, shape, optional
kind, 4 MiB chunk size and SHA-256 checksums for every chunk in its envelope.
Readers validate byte count against shape/dtype and verify all chunks intersecting
a requested row slice. Payloads are uncompressed; bfloat16 is supported through
PyTorch. Tensor lifetime follows explicit publication, queue, reader or
checkpoint ownership; a Python reference alone does not retain its pack.

## Registered batch checkpoints

`register_checkpoint(id, ref)` validates one `json.v1` record with:

```text
version: 1
checkpoint_id: the same id
consumer_state: a RecordSetRef
batch_ids: array of known ready batch IDs
training_dependencies: array of RecordSetRefs
```

The enclosing publication must list `consumer_state` and every
`training_dependencies` entry as dependencies, not just encode their descriptors
inside JSON. Application-specific fields can describe its durable checkpoint.
The core records and retains this root; it cannot verify external model or
optimizer files. `release_checkpoint` retires the root explicitly.

## Shared storage ownership catalog (opt-in)

A GC-enabled pool adds exactly one `storage.log`. It is both an append log and
the stable advisory-lock inode; do not replace it while any client exists.
Transactions reopen their I/O handle after acquiring the distributed lock.
Each frame uses a 56-byte header and 40-byte trailer:

```
header = "STRGC001" | u64-le sequence | u64-le JSON-length
         | sha256(first 24 header bytes)
body = compact JSON {run_id, owners?, packs?, deleted?}
trailer = sha256(header | body) | "STRGEND1"
```

The initial sequence is zero; body length is bounded at 64 MiB. Header checksum,
sequence, body checksum and trailer must validate before an event is applied.
A complete corrupt frame is fatal. An incomplete final frame can be truncated
under the exclusive lock; a surviving complete prefix is synced before allowing
another mutation to depend on it.

`owners` replaces named owners' complete transitive pack-path sets (null deletes
an owner), `packs` records open=false/sealed=true, and `deleted` appends durable
pack tombstones. These are durable holder sets, not an integer refcount driven
by Python destructors. Paths cannot be reopened after sealing or retained after
tombstoning. A reclaimed pack path is never reused.

Namespaced queue metadata lives under `queues/<sha256(JSON(queue_id))>/` while
payload references remain relative to the common pool root. A queue's storage
owner prefix includes its control root identity; only that queue reconciles its
prefix. Checkpoint, explicit-reader, staging and other-queue owners are separate.
Queue leases also retain input-read roots in replayed queue state. Timeout,
cancellation and epoch change preserve those roots until actual read completion
or confirmed worker retirement.

See [PROTOCOL_AND_VERIFICATION.md](PROTOCOL_AND_VERIFICATION.md) for the ordering,
filesystem assumptions, expiration of replay and known safe-leak cases.
