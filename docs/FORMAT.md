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
are checked before returning a record. The envelope length is limited by its
u32 field; payload/index lengths use u64 and checked frame arithmetic. The
current implementation has no 10,000-record, 64 KiB-envelope, 8 MiB-index or
256/512 MiB record/publication admission cap. Index/envelope JSON and complete
record reads still require corresponding memory; streaming payload access does
not make those allocations bounded.

For native caller-provided `Payload` streams, the writer captures `len()` once
for the envelope and uses that same value in the frame and accounting. Both
hash and write traversals must produce exactly that many bytes with matching
content checksums. Oversized chunks are rejected before writing them, short
traversals before the footer, and callback errors remain errors even if the
provider ignores them. Earlier writes may remain as an unpublished tail; no
rollback or preemption of arbitrary provider code is promised.

`SegmentRef` includes run ID, relative POSIX path, segment ID, extent size,
body checksum, checksum algorithm and format version. Absolute paths, `..`,
empty path components and symlinks escaping the local run root are rejected.
`RecordRef` adds a zero-based ordinal. A client chooses its own mount root;
absolute machine paths never appear in these references.

Standalone sealed publication writes a private file, flushes and fsyncs it, checks close, renames
to the unique sealed path, then fsyncs its directory. Newly created directories
and the entire resolved ancestor chain through the filesystem root are synced
on every mkdir call, including existing names left by a failed attempt. Stable
namespace/aliases and accessible ancestors are required. A rename error triggers verification of the
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
does not merge tasks. Manifest/dependency traversal has no 10,000-root admission cap; it uses a deduplicated heap worklist, so graph/index/cache memory still grows but dependency depth no longer adds a native call frame per node.

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
| Payload | Nonempty JSON array of typed state-transition events; u64 body-length field, no 8 MiB implementation cap |
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
kind, an encoded chunk size and SHA-256 checksums for every chunk in its envelope.
Readers validate byte count against shape/dtype and verify all chunks intersecting
a requested row slice. Payloads are uncompressed; bfloat16 is supported through
PyTorch. Tensor lifetime follows explicit publication, queue, reader or
checkpoint ownership; a Python reference alone does not retain its pack.

New `tensor_record` / `publish_tensors` records use a fixed 4 MiB chunk size,
including tensors over 2 GiB. Earlier producers doubled the size to keep at most
512 digests. Readers honor the stored size, so existing records remain readable
without rewriting their immutable metadata. A nonempty range of `N` bytes in a
new record authenticates at most `N + 2 * (4 MiB - 1)` payload bytes; a one-byte
range authenticates at most 4 MiB. Empty native ranges authenticate no payload.
These bounds exclude metadata reads, allocations and retry attempts. Legacy or
externally produced records have the corresponding bound for their stored size.

The digest list now grows with tensor size: 100 decimal GB needs 23,842 digests.
Envelope/index memory and processing grow accordingly. Metadata contributes to
publication identity: regenerating a large tensor's metadata under the new
policy can conflict with an earlier attempt using the same submission ID.
Finish uncertain retries with the original metadata; changing the submission
ID creates a separate operation, not a retry of the old one.

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

Automatic participation in an existing GC pool uses fresh open, not stat
presence. Once a Config family requires GC, a later missing/erroring name cannot
downgrade it to legacy. Auto-discovered initialization opens an existing log
without recreating it. Initial open NotFound must still mean unmanaged storage,
with pool mode established before clients start; live legacy-to-GC migration is
outside this protocol guarantee. Participants requiring GC should explicitly
set `online_gc=True`.

A GC-enabled pool adds exactly one `storage.log`. It is both an append log and
the stable advisory-lock inode; do not replace it while any client exists.
Transactions reopen their I/O handle after acquiring the distributed lock.
Each frame uses a 56-byte header and 40-byte trailer:

```
header = "STRGC001" | u64-le sequence | u64-le JSON-length
         | sha256(first 24 header bytes)
body = compact JSON {run_id, owners?, packs?, deleted?, reclaimed?}
trailer = sha256(header | body) | "STRGEND1"
```

The initial sequence is zero; the body-length field is u64. The current catalog
implementation has no 64 MiB body cap and allocates a complete frame during replay. Header checksum,
sequence, body checksum and trailer must validate before an event is applied.
A complete corrupt frame is fatal. An incomplete final frame can be truncated
under the exclusive lock; a surviving complete prefix is synced before allowing
another mutation to depend on it.

`owners` replaces named owners' complete transitive pack-path sets (null deletes
an owner), `packs` records open=false/sealed=true, and `deleted` appends durable
pack tombstones. These are durable holder sets, not an integer refcount driven
by Python destructors. Paths cannot be reopened after sealing or retained after
tombstoning. A reclaimed pack path is never reused.

`reclaimed` is an optional array of tombstoned paths whose deletion is already
durable. Writers append it only after every relevant parent-directory sync
succeeds, then sync the catalog frame before retiring the retry obligation.
Replay accepts only tombstoned paths, removes them from the pending-deletion and
pack-scan indexes, and retains their permanent tombstones. A surviving complete
uncertain receipt can be promoted during normal catalog recovery; a torn/missing
receipt leaves work to retry. Absence alone is not a receipt. Later repeated GC
calls do not stat/unlink/sync these completed packs or append empty receipts.
A possibly stale stat NotFound cannot replace unlink: attempt unlink and require
its success/NotFound to establish current absence before the parent barrier.

This is an additive JSON field in the existing checksummed frame. The preceding
batched implementation ignores it and safely repeats deletion checks; new clients
read catalogs without receipts and finish their remaining work. A four-client
mixed-binary check exercises both directions. This is evidence for those pinned
binaries, not a claim about arbitrary future or external catalog writers.
Tombstone memory and catalog history still grow; no online log compaction or
bounded cold-replay cost is introduced.


Namespaced queue metadata lives under `queues/<sha256(JSON(queue_id))>/` while
payload references remain relative to the common pool root. A queue's storage
owner prefix includes its control root identity; only that queue reconciles its
prefix. Checkpoint, explicit-reader, staging and other-queue owners are separate.
Queue leases also retain input-read roots in replayed queue state. Timeout,
cancellation and epoch change preserve those roots until actual read completion
or confirmed worker retirement.

See [PROTOCOL_AND_VERIFICATION.md](PROTOCOL_AND_VERIFICATION.md) for the ordering,
filesystem assumptions, expiration of replay and known safe-leak cases.

An internal `publication:<writer>:<id>` owner accumulates the union of roots
from incomplete/rejected attempts; a conflicting retry must not replace prior
protection. Release this holder only after durable staging of the successful
result. Public `retain` continues to replace a named owner's roots. These policies
use the existing replacement-frame encoding; there is no new format field.
Abandoned attempts may retain extra roots indefinitely, and peers must not mutate
another writer's private owner IDs.

Standalone completion (`segment_target_bytes=0`) stages references before
recording the file sealed in the catalog. This applies to raw `write_records`,
the internal manifest write of `record_set`, and `publish`. A seal error is
returned even if its durable catalog effect occurred; retry the same publication
to finish, retaining staging. Packed publication stays open until rotation or
explicit seal. Historical open entries are not automatically repaired, since
an open entry may still belong to an uncertain/live writer.

A complete publication's private description is retained before attempting file
sync. Same-writer retries reuse it; an unknown sync result requires a new file
barrier before standalone rename/directory completion. A confirmed file sync can
be reused, but retry still authenticates the complete extent. Another new
identity is fenced while pending. This is process-local recovery after complete
framing; it does not recover arbitrary earlier partial writes or resurrect an
abandoned writer's descriptors. Old orphan partials/open entries remain protected.

GC catalog path keys use resolved pool-relative identities for stable in-pool
symlinks. Ownership closures, `packs`, `deleted` and `reclaimed` all normalize;
replay normalizes legacy raw spellings, including names whose targets were already
unlinked. This changes identity handling without changing frame layout or rewriting
old bytes. Public references and staging-owner digests retain their existing encoding.
Relevant aliases/prefixes must remain stable for history replay; absent private
prefixes may be created as ordinary directories/files. All GC collectors must use
the repaired policy: an old collector can still delete an aliased owned file.
This is not inode/bind-mount unification or repair of already lost data. See
[the namespace environment contract](ENVIRONMENT_MODELS.md#namespace-contract-and-the-implementation-boundary).

### Recorded namespace encoding

New catalog bodies include `path_encoding: 1`. Before writing that marker, the
writer normalizes every path in `owners`, `packs`, `deleted` and `reclaimed`, even
if an internal caller supplied a marker. Conflicting pack states that collapse
to the same identity are rejected. The reader validates the marker's JSON type
and version before applying any event field. Version 1 keys still undergo lexical
relative-path validation, then enter the indexes directly; they need no repeated
filesystem resolution. This marker is a producer contract, not a signature or a
proof that arbitrary external log writers normalized correctly.

Absent markers retain the legacy resolver, including dangling aliases. Null,
Boolean, string, noninteger, out-of-u64 and unknown numeric markers fail closed.
No old bytes are rewritten and old histories retain their lookup cost. The
preceding identity-aware binary ignores the new field and resolves the already
normalized keys; measured mixed-version compatibility applies to those exact
binaries. Older collectors with the raw-path ownership bug remain unsafe.
Frame checksums cover the marker and keys; framing itself is unchanged. Stable
namespace bindings and compliant producers remain required. A false version-1
marker containing aliases can split ownership identities and is outside the
producer contract.

### Cached staging dependency edges

New consumption frames can contain `staging_edges`, mapping `staged:<reference SHA-256>` to direct child staging-key arrays; leaves have empty arrays. The field and `owners` releases share one complete frame. Replay checks key syntax, child definitions and conflicts with existing nodes before reconstructing the graph. Immutable edges support repeated/regrouped releases and restaged dependencies without re-reading a deleted root manifest.

Older frames without this field still replay. First consumption can learn surviving old references, but already collected legacy manifests with no recorded graph cannot be recovered. Older clients ignore the additional field and their own graphless releases can still fail on retry; mixed versions do not gain a uniform retry guarantee. Existing frame/checksum encoding is unchanged. Direct edges avoid storing each node's transitive closure, but indexes/history grow with unique references and no graph retirement or compaction is implemented. This is not constant memory.

Direct raw-record roots already identify their only staging owner and add no cache nodes when released. A raw record reached from a manifest is recorded as that graph's leaf. Empty manifests still need their own empty edge definition.

## Queue staging handoff receipts

With online GC enabled, a transaction containing record-set references ends with one `{"type":"StagingHandoff","id":...}` event in its existing WAL frame. The ID is the lowercase SHA-256 of canonical JSON `[storage_prefix, sequence, business_events]`; business events exclude the marker and include acceptance/deduplication. Recovery validates position, uniqueness, binding and nonempty roots. Queue path identity retains the existing stable-namespace contract.

The catalog release frame carries `consumed_handoffs: [id]` together with `owners` deletions and new `staging_edges`. IDs must be 64 lowercase hexadecimal characters; a receipt frame must have an object-valued `owners` whose values are all null. A completed ID skips appending, manifest reads and consumption of subsequent staging. Rebuild the receipt index with other unconfirmed catalog state; a tentative cache entry is not durable completion.

Recovery finishes outstanding handoffs before its new epoch; live operations and cached responses cross the same barrier. Unmarked legacy transactions recover without automatic consumption because old leaks cannot be distinguished from later staging. Old coordinators reject the new unknown WAL event: do not downgrade a queue writer after this format is used. Older catalog readers ignoring extension fields does not imply old coordinator support for the WAL. Logs, recipes and receipts grow without compaction.
