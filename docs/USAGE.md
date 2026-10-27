# Using straw

[中文版](USAGE_zh.md)

The public Python API uses the Rust storage and queue protocol.
[Examples](../examples) contains complete CPU-only programs.

<a id="install"></a>

## Install

```sh
python -m pip install straw-queue
```

Supported platforms install a wheel containing the Rust extension without a Rust
toolchain. Release targets are Linux x86_64, CPython 3.10–3.13 and glibc 2.28+;
the actual wheel tag is authoritative (`cp312` cannot run on Python 3.10).
For offline installation use `python -m pip install ./straw_queue-*.whl` with only
a compatible wheel in the directory. Install the appropriate CPU/CUDA PyTorch
build first if needed; NumPy/PyTorch are runtime dependencies. Other platforms
and source development use the [contributor instructions](../CONTRIBUTING.md#build).

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
of old files retain their stored checksum chunk sizes. Full-record reads still allocate the returned payload; use authenticated ranges
when only part of a large record is needed.

## Concurrency: processes and threads

Published extents are immutable, but a shared-filesystem reader may see the
length or header before other committed bytes. Short reads, zero-filled header
gaps (including partial gaps), and footer/index/envelope/payload/chunk validation
failures are retried with fresh opens. One requested extent/record/range read
shares five waits of 100, 250, 500, 1000 and 2000 ms across all validation stages.
Each returned record/chunk must still pass authentication. Persistent failures
return `CorruptData`; nonzero bad headers, identity/schema errors and unrelated
I/O errors fail without this retry. Checksum corruption can therefore take the
full retry window to report. The waits do not impose a deadline on filesystem
I/O, and dependency traversal can perform several individually bounded reads.
This covers store and read-session entry points without extra I/O on successful
reads. See the [TLA+ specification](DESIGN.md#review) and avoid stacking an
unbounded application retry loop around corruption errors.

The API is synchronous; straw does not launch an I/O process pool. Applications
can read/write from multiple processes and machines using the same pool. Each
writer needs its own store instance and pack stream. Keep these instances alive
across publications; do not pass mutable stores/coordinators through a fork.

Rust releases the GIL during native I/O, checksum and journal work, so Python
threads can overlap those operations. Python serialization, tensor conversion
and copying input buffers still have costs. A store permits one write in flight;
concurrent writes through the same instance are rejected with
`ResourceLimitExceeded`, so serialize them or give each writer its own instance.
`metrics`, `seal()` and `close()` use the same nonblocking writer admission: they
also raise `ResourceLimitExceeded` while a write or another writer operation is
in flight. Retry after that operation returns. This prevents a synchronous fault
callback, or a thread that must release that callback, from waiting on the writer
it already holds. A rejected close/seal leaves the writer unchanged. Readers can
run concurrently while their references remain protected. This does not make
catalog/coordinator/journal callbacks generally reentrant.

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
Public retain, release and reactivation APIs reject `queue:` and `publication:`
owner names, which are reserved for internal protocols. Do not use internal
`staged:` owner names either.

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
multi-tenant service. See [application integration](USAGE.md#integration).

### What resumes after a restart

| State | Durable source | Application action |
|---|---|---|
| Dataset/task production position | `producer_state` submitted in the same transaction as tasks | Read that state and retry the same request ID after an uncertain result |
| Partially sampled task | Last committed `save_task_progress` or `yield_task` input reference | Restore its consistent tokens/RNG/application state; work after that point may repeat. An interrupted active lease can consume retry budget |
| Training | One committed model/optimizer/RNG checkpoint and its matching consumer state | Select that checkpoint externally and explicitly restore its data position, pending work and sparse progress before reading again |

`recover=True` replays the queue WAL; `load_consumer_state` returns its latest saved consumer state. Neither automatically chooses or loads the latest external model checkpoint. For example, if the model checkpoint is at step 100 but the latest processed cursor is 110, resuming the old model at cursor 110 can skip work that must be repeated.

Online GC makes this distinction essential. Results produced after an immutable checkpoint cannot already be its dependencies. Before acknowledging those results as processed, keep them under explicit replay owners, for example one unique owner per checkpoint/receipt via `store.retain(owner, [receipt.result_ref])`. Reusing one owner requires retaining its complete accumulated reference set. Keep these owners until a newer coherent checkpoint is durably committed. Another valid policy is to delay processing acknowledgement until checkpoint commit, at the cost of retaining more runtime budget.

On recovery, first restore the chosen checkpoint's model/optimizer/RNG and its matching consumer state, including pending/prefetched work. Saving the restored consumer state re-establishes required queue ownership before temporary replay owners are released. Missing or reclaimed history is rejected; changing a cursor cannot recreate it. The online-GC/mock-optimizer and abrupt sampling-process regressions are in [test_resume_workflow.py](../tests/test_resume_workflow.py).


## Scoped batch reads

Use one `with store.read_session() as reader:` block for adjacent immutable publications. The reader provides `validate`, `manifest`, `read`, `read_record`, `envelope`, and checked tensor reads. Pass it to `TensorRef.from_record_set_many(store, publication, reader=reader)` and `tensor.load(reader=reader)` to avoid repeatedly parsing a shared extent index.

The Rust reader caches up to eight authenticated indexes using an LRU keyed by the complete extent descriptor, with an 8 MiB encoded-metadata budget. A single oversized active index evicts all other entries. Each manifest and record read still authenticates its frame and payload; tensor reads authenticate touched chunks. `validate` rechecks logical descriptors and task/attempt authorization on every call. Index authentication runs again on a cache miss or in a new session. Calls within one session serialize; use independent sessions for parallel reads. Keep a durable owner or `store.pin` for the entire operation: a read session is not an ownership pin. Sessions close on context exit and reject all inherited operations after fork.

Publication dependency validation, continuation batches, consumer snapshots and GC traversal use the same bounded native reader internally. Formats, WAL commit ordering, owner-release rules remain unchanged; logical capacity quotas are no longer enforced.

`store.read(publication)` also owns a scoped session for the lifetime of its iterator. Exhaust or close the iterator before releasing its data owner. Separate iterators reauthenticate the index.


### Persistent task scheduling

`acquire(..., task_ids=[...])` treats IDs as a set filter: repeated IDs do not
create additional assignments or consume batch slots. Unknown/nonpending IDs
are ignored; scheduling order still determines which matching tasks are issued.

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

### Request identity after a publication error

Once a complete file exists, retry sync, staging, sealing or internal-owner handoff errors on the same writer with the same API, `submission_id` and contents. Another ID/API returns `IndeterminateCommit`; conflicting contents return `IdempotencyConflict`; explicit `seal()` also refuses. The next request is accepted after the original succeeds. `close()`, abandonment or process death may retain protected files/owners; registered abandoned requests can be handled by the explicit retirement protocol below. Dependency holds created before file completion can also remain.

### Retrying publication release

### Recover a writer publication after a lost reply

Record `store.writer_id` with the application's request identity before publishing. A completed internal handoff does not prove that the caller received its reply. If `Results` was persisted before the error or process exit, a new client in the same pool can query it using all three identity fields:

```python
receipt = peer.publication_receipt(
    writer_id,
    api="publish",          # records, record_set, or publish
    submission_id="publication-1",
)
```

`records` identifies `write_records`; `record_set` identifies `record_set`; `publish` identifies both `publish` and `publish_many`. The query returns a list of the latest recorded results for that exact writer/API/submission ID, including internally completed or retired operations. `None` means no result was recorded for that identity. The query holds the writer's gate and can raise `WriterBusy`; fenced writers remain queryable.

This query does not stage data or change ownership. A returned old reference may already have been consumed and garbage collected; querying its receipt cannot restore the bytes or renew protection. For an unfinished registered publication, `peer.retire_writer(writer_id, policy="recover")` first fences the incarnation, recovers complete results into staging, and seals its reservations. A held gate returns `WriterBusy`; an old writer that resumes after fencing receives `WriterFenced`. Create a new writer for subsequent publications. Explicit `policy="abandon"` abandons unfinished operations while preserving existing queue, reader and staging roots. Unknown legacy preparation ownership is retained and reported rather than guessed from timeouts.

Retirement depends on the deployment's gate-lock, fence-read coherence and in-flight-operation drain contracts described in [environment models](DESIGN.md#environment). It does not infer death from a PID, heartbeat or lease timeout.

`release_publications(refs)` consumes root/dependency staging while preserving independent reader/checkpoint/queue owners. Once new code records the release, even if its response is lost, retry/reopen/regrouping can succeed after manifest GC; it also consumes later staging of recorded dependencies. Release only publications you no longer need. Legacy logs without dependency edges cannot recover already collected manifests and still return an error.

Validation/write errors before a complete publication preserve established dependency holds for the original retry, including across other requests. Explicit `seal()`/`close()` abandons unfinished preparations and cleans their holds; their original retries are no longer guaranteed afterward. Cleanup errors return `IndeterminateCommit` and preserve the current identity; subsequent operations finish started cleanup first. A complete uncertain handoff still requires its original API/submission ID and remains protected even on close. Process death requires explicit retirement; unregistered historical owners still require separate recovery evidence.

## Early typed-tensor reference checks

`TensorRef.load()`, slices and `validate()` bind dtype, shape and byte length to the authenticated envelope before payload allocation/read. Mismatches raise `CorruptData` without first reading a large tensor. Empty slices also validate the reference, so they can perform metadata I/O and report mismatched references or storage errors. They are not evidence that no file was accessed. Matching legacy records continue using their encoded checksum granularity.

The lower-level `store.read_tensor_range(ref, start, stop, expected=...)` and read-session method optionally accept `{"dtype": ..., "shape": [...], "length": ...}` and bind those fields to the envelope. Omitting it preserves the byte-range API. `TensorRef` additionally checks its own shape/byte-count relation. Python validation, JSON/FFI correspondence and adapter ordering are reviewed and regression-tested; finite protocol models do not prove their implementation equivalence. Early rejection concerns payload work; metadata reads, parsing and allocations still cost resources.

<a id="integration"></a>

## Application integration

Exchange JSON references through the existing control transport. Keep custom
codecs, scheduling, batching, conversion and framework adapters in the consuming
application. Use explicitly versioned sample/session schemas; reject unknown
versions instead of importing or pickling arbitrary classes. Reuse one writer
per process, publish large tensors separately with dependencies, decode batches
with `from_record_set_many`, and persist intermediate work incrementally.

Rollout routing/score captures must be complete and internally consistent before
publishing a continuation; token generation ending is insufficient. Acknowledge
raw outputs after every required consumer finishes, and ready batches after all
training ranks finish reading. The application supervisor must latch/propagate
background GC failures, stop admitting work, retain uncertain storage and fence
remote operations still in flight.

Joint checkpoint commit/rewind follows [C4](DESIGN.md#c4): model/optimizer/RNG and
matching queue view, pending/buffered/prefetched dependencies and post-checkpoint
replay owners. A latest marker or queue acknowledgement cannot replace it. See
[restart APIs](#what-resumes-after-a-restart). Application tests cover real schemas,
abort/resume, shared tensors, multiple-reader completion and whole-job recovery.
