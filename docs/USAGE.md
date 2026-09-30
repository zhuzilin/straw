# Using straw

[中文版](USAGE_zh.md)

Install a matching [wheel](BUILDING.md). The examples below use the public
Python API; the Rust crate owns the storage and queue protocol. Complete,
CPU-only programs are in [examples](../examples).

## Open a storage pool

```python
from straw import SharedFilesystemStore

store = SharedFilesystemStore(
    "/shared/new-job", "new-job",
    online_gc=True,
    segment_target_bytes=1024**3,      # Rotation target, not a hard file-size limit.
    max_record_bytes=16 * 1024**3,
    max_buffer_bytes=16 * 1024**2,
)
```

Use a unique directory/run ID for each new job. Participants in one pool share
the directory and run ID. Keep a writer alive across many publications and
create new store/coordinator objects after a process fork. `close()` seals the
writer; `seal()` seals its current pack and allows later writes into a new one.
Both store and coordinator support `with` blocks.

Enable online GC from pool creation. Do not turn an existing untracked queue
into a GC-enabled pool and assume its old references have owners. GC is opt-in;
when a catalog exists, all participants must honor it. None of these calls
create a scheduler or start an automatic GC thread.

## Large records and bounded write buffers

`max_record_bytes` is a compatibility argument and no longer caps records.
`max_buffer_bytes` bounds temporary payload copies inside the native writer,
independently of record/publication size. The writer pins Python input buffers
and copies at most 4 MiB per chunk; large records pipeline copying and I/O using
at most two chunks. Applications do not need to split payloads to fit scratch
memory. Metadata/index allocations, caller-owned inputs, Python serialization
and full-record reads are outside this payload-copy budget. There are no fixed
record-count, dependency-count, metadata, manifest or index size quotas.

Do not mutate input buffers until publication returns. The writer rechecks each
payload while writing and rejects detected changes before committing. Chunked
I/O retains the existing record framing and tensor slice API: it does not expose
partial records or alter durable publication, retry, ownership or GC ordering.
A pack rotation target is soft; a large record/publication may exceed it. Reads
of old files retain their stored checksum chunk sizes. A reader configured with
an explicit smaller record limit must raise it to read larger records.

## Concurrency: processes and threads

Published extents are immutable, but a shared-filesystem reader can briefly see
the new file length before the appended header bytes. The native reader closes
and reopens the file on a short extent or an all-zero 12-byte header, with delays
of 100, 250, 500, 1000 and 2000 ms. It still validates the header, index and record
data normally. Persistent failures remain errors; nonzero bad headers and
checksum failures are not retried by this visibility handling. This applies to
store and read-session entry points, including dependency validation. Successful
reads incur no extra I/O, and applications should not stack another retry loop
around the resulting corruption error.

The API is synchronous; straw does not launch an I/O process pool. Applications
can read/write from multiple processes and machines using the same pool. Each
writer needs its own store instance and pack stream. Keep these instances alive
across publications; do not pass mutable stores/coordinators through a fork.

Rust releases the GIL during native I/O, checksum and journal work, so Python
threads can overlap those operations. Python serialization, tensor conversion
and copying input buffers still have costs. A store permits one write in flight;
concurrent writes through the same instance are rejected with
`ResourceLimitExceeded`, so serialize them or give each writer its own instance.
Readers can run concurrently while their references remain protected.

Metadata is not entirely parallel: each queue has one externally fenced
coordinator, and catalog mutations/GC share a cross-client lock. More processes
do not remove these serialization points or per-transaction fsync latency.
Batch small publications where semantics permit, and use a bounded executor
when calling from an async application. The [multiprocess example](../examples/multiprocess.py)
and [benchmark topology](BENCHMARKS.md) demonstrate distinct execution layouts.

## Records and custom data

```python
import json
from straw import Record

ref = store.publish([
    Record("text", "hello".encode()),
    Record("config", json.dumps({"temperature": 0.7}).encode(), codec="json.v1"),
], submission_id="publication-1")
records = list(store.read(ref))
assert records[0].payload == b"hello"
assert json.loads(records[1].payload) == {"temperature": 0.7}
```

`Record.payload` accepts `bytes`, `bytearray`, or a contiguous byte `memoryview`.
Reads return a `Record` containing bytes, codec name, JSON metadata and a token
count. IDs, metadata, codec and record order contribute to logical identity.
JSON metadata must be finite JSON values; put large content in payload records.

The built-in `bytes.v1` and `json.v1` names do not run a serializer for you.
Register a custom name with `codecs=("bytes.v1", "json.v1", "document.v1")`
when opening the store, then write a `Record(..., codec="document.v1")`.
Your application owns encoding, schema validation and decoding. straw checks
the envelope, configured codec, framing and checksums; it never imports payload
classes or unpickles data. Version your custom codec when its meaning changes.

`publish_many([Publication(...), ...])` packs multiple logical results into a
single durable write while returning separate references. Dependencies may
point to an existing `RecordSetRef` or to an earlier publication's integer
index in the same call. Physical packing does not merge task identities.

`RecordSetRef` addresses an ordered manifest. Use `store.manifest(ref)` for its
record/dependency descriptors, `store.validate(ref)` for checked member refs,
and `store.read_record(record_ref)` for one record. `read(ref)` returns that
manifest's ordered records, not an implicit flattening of dependency payloads.
An empty logical publication is permitted; its manifest still occupies bytes.

References serialize with `dataclasses.asdict(ref)` and restore with
`RecordSetRef.from_dict(value)`. They contain relative paths, not payload bytes.
Do not construct pack paths or unlink records in application code.

## Tensors

```python
import numpy as np
import torch
from straw.tensor import publish_tensors

# A tensor store must explicitly include the tensor codec.
tensors = SharedFilesystemStore(
    "/shared/new-tensor-job", "new-tensor-job",
    codecs=("bytes.v1", "json.v1", "tensor.v1"), online_gc=True,
)
features, scores = publish_tensors(tensors, {
    "features": np.arange(24, dtype=np.float32).reshape(6, 4),
    "scores": torch.ones(6, dtype=torch.float32),
}, submission_id="features-1")
assert features[1:3].shape == (2, 4)
assert features.load().shape == (6, 4)
```

Supported dtypes: `uint8`, `int8`, `int16`, `int32`, `int64`, `float16`,
`bfloat16`, `float32`, `float64`, `bool`. The format is little-endian,
contiguous and uncompressed. CUDA tensors are copied to CPU before publication;
autograd history, sparse layouts, object arrays and complex dtypes are not
encoded by this helper. Empty tensors and scalars can be loaded; row slicing
requires a leading dimension. `.load()` returns a CPU PyTorch tensor; call
`.numpy()` or `.to(device)` explicitly if needed.

`ref[start:stop]` reads contiguous rows and verifies intersecting 4 MiB chunks.
Only slices with step 1 are supported. These are file reads, not mmap views.
`load(pin_memory=True)` requests pinned CPU memory where PyTorch supports it.
Large tensors stream through bounded write scratch without record/publication quotas.

To restore several tensor descriptors from one publication efficiently:

```python
from straw.tensor import TensorRef

dependency, ordinal = features.share(tensors, submission_id="share-features")
restored = TensorRef.from_record_set_many(tensors, dependency)
assert restored[ordinal].shape == features.shape
```

The mapping contains tensor members by ordinal; non-tensor records are omitted.
`from_record_set(store, ref, ordinal)` restores a single member. Prefer batched
restoration when many tensors share an extent. A `TensorRef` also carries its
local mount root: reconstruct it against the destination store rather than
editing its path fields to use a different mount.

## Sharing and copy-on-write

Several namespaced queues can share one pool:

```python
from straw import Coordinator, TaskSpec

a = Coordinator(tensors, queue_id="stage-a", namespace=True, exclusive_owner="owner-a")
b = Coordinator(tensors, queue_id="stage-b", namespace=True, exclusive_owner="owner-b")
a.submit_tasks("a-1", [TaskSpec("task-a", input_ref=dependency)])
b.submit_tasks("b-1", [TaskSpec("task-b", input_ref=dependency)])
```

These queue owners retain the same packs. `TensorRef.share` itself only returns
a descriptor: keep the source owner until the destination publication/queue
has durably adopted it. Use `dependencies=[dependency]` when a new application
manifest refers to a shared tensor. Dependencies must be in the same pool/run.

`features.updated(tensors, replacement, rows=slice(...), submission_id=...)`
materializes and writes a new tensor version. The original remains immutable;
unmodified tensors continue sharing their existing references. See the complete
[two-queue tensor example](../examples/tensors.py).

## Tasks and accepted results

The [work queue example](../examples/work_queue.py) covers the full flow:

1. Publish input records; submit `TaskSpec(task_id, input_ref=...)` under a stable
   request ID. Task metadata and producer cursor state are small JSON values.
2. `acquire(worker_id)` returns assignments with inputs and leases. Interpret
   `empty`, `backpressured`, `end_of_input` and `draining` as distinct states.
3. Read the input under its lease's reader ownership. Publish output records
   with `task_id` and `attempt_id` metadata from the lease.
4. `complete_task(lease, submission_id=..., result_ref=..., result_digest=...)`
   journals acceptance and returns a stable receipt. After a lost reply,
   retry that exact identity/reference or use `lookup_submission`.
5. `read_commits(cursor, limit)` returns an ordered page and next cursor.
   Several readers can replay independently while the data is retained.

Keep physical publication retry IDs and logical completion IDs stable across
uncertain replies. Changing content under an idempotency key is an error.
Do not assume `submission_id` makes arbitrary writes globally deduplicated:
queue acceptance is the authority. A crashed execution may run again.

Renew leases with `heartbeat`. Use `save_task_progress`/`save_task_progress_many`
for durable intermediate inputs while retaining the lease; `yield_task` or
`release_tasks` returns unfinished work without spending a failure retry.
`fail_task` and timeouts use the task's attempt budget. Only publish a
continuation when all its fields describe one complete, consistent prefix.

`Limits` retains its fields solely for API and stored queue-identity compatibility.
They no longer reject pending/in-flight work, result bytes/tokens/records, ready
batches or control messages. Applications control concurrency through requested
acquisition sizes and their own schedulers. Accounting metrics remain available;
large immutable results are accepted by reference without rewriting payloads.
Pass the same legacy limits, codecs, lease duration, run ID and queue ID when
recovering an existing queue. In protocol v1 the default queue ID is `rollout`.

The removed fixed quotas include 4 GiB per result, 64 GiB of outstanding results,
100 million tokens, 10,000 records/dependency nodes, 64 KiB record envelopes,
256 KiB control metadata, 8 MiB indexes/manifests/journal transactions and 64 MiB
catalog transactions. The existing frame uses a u32 envelope length; that format
boundary, u64 counters, authenticated file extents, checksums, task/attempt
ownership, retry counts, leases and writer exclusion remain enforced. Removing
quotas does not provide unlimited RAM/disk; metadata is still materialized in
memory and payload writes retain bounded scratch buffers.

## Lifetime and online GC

| Owner | Establish it | End it |
|---|---|---|
| Publication staging | Successful publish | Queue adoption, or `release_publications` |
| Explicit application/checkpoint | `store.retain(unique_owner, refs)` | `store.release(owner)` |
| Temporary reader | `with store.pin(refs): ...` | Exit only after reads actually finish |
| Queue input and leased reader | Submit / acquire | Completion/yield/failure, plus explicit stopped-reader handling where required |
| Accepted result | Queue acceptance | Designated consumer acknowledges processing and GC retires replay |
| Ready batch | `batch_ready` | Consumer progress lists `finished_batches` after all ranks/readers finish |
| Registered checkpoint | `register_checkpoint` with its validated root | `release_checkpoint` |

Retention is a durable owner set, not Python reference counting. Garbage
collection never depends on `del`, object destruction or lease timeout.
`retain` replaces that owner's roots; use a different owner ID per independently
retained version and retain a successor before releasing its predecessor.
Do not use internal `queue:` or `staged:` owner names.

`store.collect_garbage()` reclaims already unowned, sealed packs. For queues,
call `queue.collect_garbage()` so acknowledged queue history first retires its
roots durably. It also checks that every live queue root still has an owner.
Missing ownership, corruption and storage errors must reach the application;
do not treat them as harmless cleanup warnings.

Protocol v1 uses the designated consumer ID **`training`** for consumption
accounting, even in a non-training application. Open it with `open_consumer`,
then `save_consumer_state` with a published state reference, `fetch_cursor`
and `processed_cursor`. Processing acknowledgment must cover every required
reader. Optional `progress_ref` is one `json.v1` record with `version: 1`,
`processed_positions` for sparse acknowledgments and `finished_batches` for
completed batch reads. Other consumer IDs retain their saved states but do
not independently hold every fetched result. Pin data for a slower replay
reader before the primary processing cursor advances. The example includes
a checkpoint that prevents reclamation after processing.

Fetching is not acknowledgment. Model update completion is not checkpoint
durability. After replay retirement, old receipt metadata can remain in the
WAL while its payload is unavailable; retain checkpoint/replay roots beforehand.
Close/release of a consumer token alone does not discard its saved state.

Queue lease expiry, cancellation and recovery do not prove its reader stopped.
`outstanding_reads()` exposes remaining read leases. A supervisor may call
`release_task_reads` or `retire_worker` only after confirming those readers
have finished or their processes are dead. Uncertain readers, abandoned open
packs and staging are retained conservatively. There is no TTL-based deletion.

## Recovery and checkpoints

Stop the old coordinator and establish that it cannot resume before opening
`Coordinator(..., recover=True, exclusive_owner=<real external guarantee>)`.
Recovery replays the WAL, persists a new epoch and rejects old leases. Stable
accepted receipts survive; unfinished attempts may be retried. Reopen client
objects after process restart. [The multiprocess example](../examples/multiprocess.py)
kills and joins its owned coordinator before recovering and replaying results.

An application checkpoint must retain all referenced data before publishing
its final durable pointer. Record the exact application version, queue/consumer
positions and data dependencies; publish a committed manifest only when all
components are durable. Retire the previous checkpoint explicitly afterward.
straw cannot infer external optimizer or database commit durability.

`queue.snapshot()` is an inspection artifact, not an authoritative startup
checkpoint and not a transitive data pin. The queue always recovers from its
journal. The advanced `register_checkpoint` batch-checkpoint schema is specified
in [the format](FORMAT.md); it still requires application-side finalization.

All storage/coordinator calls are synchronous. Use a bounded executor from an
async application. Choose your own control transport; the small JSON HTTP
transport in `straw.rpc` is for examples/private job networks, not a hosted
multi-tenant service. See [application integration](APPLICATIONS.md).

## Scoped batch reads

Use one `with store.read_session() as reader:` block for adjacent immutable publications. The reader provides `validate`, `manifest`, `read`, `read_record`, `envelope`, and checked tensor reads. Pass it to `TensorRef.from_record_set_many(store, publication, reader=reader)` and `tensor.load(reader=reader)` to avoid repeatedly parsing a shared extent index.

The Rust reader retains only the last authenticated index, keyed by the complete extent descriptor. Each manifest and record read still authenticates its frame and payload; tensor reads authenticate touched chunks. `validate` rechecks logical descriptors and task/attempt authorization on every call. Index authentication runs again after an extent switch or in a new session. Keep a durable owner or `store.pin` for the entire operation: a read session is not an ownership pin. Sessions close on context exit and reject all inherited operations after fork.

Publication dependency validation, continuation batches, consumer snapshots and GC traversal use the same bounded native reader internally. Formats, WAL commit ordering, owner-release rules remain unchanged; logical capacity quotas are no longer enforced.

`store.read(publication)` also owns a scoped session for the lifetime of its iterator. Exhaust or close the iterator before releasing its data owner. Separate iterators reauthenticate the index.


### Persistent task scheduling

`TaskSpec.priority` (default 0) is ordered descending, then `scheduling_key`
(default 0) ascending, then FIFO. Both keys are signed 64-bit integers. Keys and
submission/return order survive WAL recovery; the Rust coordinator maintains an
ordered pending index, without loading payloads to select the next task.

Publish continuation inputs before calling `yield_tasks(updates, request_id=...)`.
Each update contains `lease` and `input_ref`, and may replace `priority`,
`scheduling_key` and `metadata`. The whole batch validates before one WAL commit;
all leases end together and the tasks become pending at the tail of their key's
FIFO order. A stale lease rejects the whole batch. Retrying the identical request
is idempotent. Publish again only if you intend to create a different input.
This is distinct from `save_task_progress_many`, which keeps leases active.
`pending_tasks()` returns ordered task specifications for a paused snapshot;
applications must coordinate the pause and retain the referenced inputs.

These APIs require `straw-queue>=0.1.1`.
