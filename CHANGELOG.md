# Changelog

[中文版](CHANGELOG_zh.md)

## 0.1.7

### Batched reads and validation

- Add `TensorRef.load_many(..., reader=reader, workers=4)` for caller-bounded,
  ordered tensor batches. Native workers reuse extent handles and coalesce nearby
  reads in bounded windows (1 MiB by default, at most 8 MiB per worker), avoiding
  read-ahead across gaps larger than 4 KiB. Each result has independent writable
  storage; typed layout, selected frames and touched checksum chunks remain
  authenticated. Failed attempts discard handles and windows before retrying.
- Read tensor member envelopes through one native call and reuse canonical path
  information within a publication, reducing Python/native transitions and
  repeated shared-filesystem path resolution.
- Add `store.validation_scope(coordinator)` to reuse immutable dependency proofs
  during one local publication/queue handoff. New roots still check descriptors
  and task/attempt identity; catalog changes still check ownership, namespace
  and tombstones under lock. Scopes are thread-local, do not pin data, and close
  on exit or participating errors. Explicit validation and payload reads retain
  their normal checks.
- Sample the coordinator lease clock after acquiring its call lock, so queued
  calls do not submit stale timestamps. Log slow calls with separate lock-wait
  and operation times.

### Compatibility and limits

- Existing Python call signatures and stored WAL/packed-record formats remain
  compatible; the new batch and scope APIs require 0.1.7. Keep input owners alive
  throughout reads and validation scopes. Callers must bound the total payload
  bytes returned by a batch; the read-ahead limit only bounds worker scratch.
- These changes reduce small-read calls and repeated immutable-graph work.
  Throughput depends on request locality, cache state and storage; they do not
  establish a shared-filesystem bandwidth guarantee or change GC durability
  requirements.

## 0.1.6

- Reuse confirmed writer-registry state and index operation lookup; rebuild after uncertain outcomes. Cache only successfully synchronized writer directories with pinned inode identity. Keep existing publication WAL barriers and wire formats.
- Move physical GC deletion and directory synchronization outside the pool catalog lock. Bind receipts to the completed plan and support overlapping collectors without consuming newly added pending work.
- Cache up to eight authenticated extent indexes for alternating reads. Extend protocol models and concrete regressions; add real Rust mutation checks to CI and a reproducible metadata-scaling probe. Metadata history/receipt retirement and compaction remain out of scope.

## 0.1.5

### Ownership safety

- Fix reactivation reusing an existing public owner without checking that it protects the requested root, including owner changes after authentication outside the lock. Check the retained pack closure both initially and under the fresh lock; internal queue owners bind the reference digest. Correct reuse still succeeds; public owner reuse adds closure/metadata checks.
- Preserve public APIs, WAL/pack formats and durability ordering. Add regressions for existing-owner and post-authentication changes, and for staging remaining protected until queue WAL confirmation.

### Specification, verification and maintenance

- Remove all Verus source, inline specifications, ghost state, library contracts, runner, negative controls and CI; simplify Rust wrappers. Remove six deterministic local TLA+ models while retaining complete protocol models and product fault regressions.
- Extend cross-log attempt/epoch and checkpoint rewind, WAL caller/cache, reactivation, authenticated read, writer retirement and conditional-progress scenarios. Distinguish model checking, execution tests and source review without claiming complete implementation equivalence.
- Merge design, protocol, environmental premises and model-to-code correspondence into the [design and spec](docs/DESIGN.md). Move application integration into usage and checks/builds/releases into contributing; update both languages and add the `straw-protocol-review` skill.
- Delete per-iteration proof, measurement and acceptance reports. Retain only three public benchmarks cited by the performance guide and explicitly list them in source-package manifests. Their results describe the original 0.1.4 candidate, not this version's performance.

## 0.1.4

### Recovery and continuation

- Preserve producer continuation payloads and independent reader, consumer and checkpoint roots during online GC. Sampling can resume from its durable continuation after process failure. Training applications must select and load a checkpoint with matching model/optimizer state and data cursors, and retain data needed for replay; straw does not automatically load a model checkpoint or recreate reclaimed data.
- Resolve durable receipts before reopening old references when retrying accepted consumer-state, batch-plan, batch-ready and checkpoint-registration requests. Identical retries can succeed after those references have been legitimately reclaimed, without creating new ownership.
- Persist staging cleanup intent, dependency recipes and generation-specific completion receipts. Recovery can finish an accepted handoff after a lost reply without releasing a later staging generation of the same reference.
- Quarantine uncertain WAL writes, partial state application and caught Rust callback panics until recovery. Reject malformed journal/catalog events before appending, preserve confirmed prefixes during rebuild and reject changed pool/run or log identity. Harden counter arithmetic and deduplicate repeated task IDs before lease selection.

### Storage and reads

- Retry independent visibility delays in the header, footer, index, envelope, payload and tensor checksum chunks within one bounded budget. Every returned record remains authenticated; persistent corruption fails after the budget expires.
- Reuse the authenticated index descriptor within a read attempt and buffer catalog replay with at most 64 KiB of read-ahead. Failed read attempts reopen their files. Replace recursive dependency walks with deduplicated worklists to support deep graphs.
- Validate tensor dtype, shape and length before reading payloads. New tensors use 4 MiB checksum chunks; older tensors retain their stored chunk sizes. Checkpoint reactivation authenticates all reachable payloads outside the catalog lock with at most 1 MiB of payload scratch, then rechecks ownership and tombstones under the lock.
- Preserve exact finite JSON floating-point values through serialization and recovery. Reject native payload providers whose traversals have the wrong length or swallow callback failures.

### Garbage collection and publication safety

- Retry directory barriers after unlink, including already absent files and missing parents. Batch shared-parent barriers and persist reclamation receipts only after deletion is durable. Preserve permanent tombstones; a cold-cache retain of a reclaimed root or dependency now returns `InvalidReference` before trying to read its missing payload.
- Protect canonical paths and aliases, peer queues, active readers, checkpoints, producer continuations and pending publications. Public owner APIs reject internal `queue:` and `publication:` names. Offline cleanup preserves control files, rejects symlink cycles on Python 3.13 as well as earlier versions, and fails closed on uncertain shared-pool discovery; dangling orphan links remain eligible for cleanup.
- Preserve dependency holds and retry identities through failed publication setup, synchronization, staging and sealing. Repair missing ancestor barriers, roll back unpublished tails before writer reuse and seal completed standalone reservations so they can be collected. A failed GC hook with partial WAL effects forces fresh recovery before further writes.

### Writer retirement

- Add explicit `retire_writer(..., policy="recover" | "abandon")` with a durable fence, operation registry and reusable recovery receipts. Recovery stages complete results before releasing preparation ownership; abandonment preserves existing queue, reader and staging roots. A writer holding its gate reports `WriterBusy`, and a fenced writer cannot resume writing. Retirement depends on the documented lock, fence-visibility and in-flight-I/O contracts; timeouts alone do not establish death.
- Reject busy writer metrics, seal and close operations without waiting on an in-flight callback. Unknown legacy preparation owners remain protected and are reported for separate recovery.

### Compatibility and migration

- Packed-record and WAL byte framing remain unchanged, and legacy catalog entries still replay. New queue staging-handoff events require updated coordinators: do not downgrade a queue writer after those events are stored. Upgrade all GC participants before relying on canonical-alias protection; an old collector can still delete an aliased owned file. Mixed versions do not gain a uniform cleanup/retry guarantee; historical leaks and already reclaimed legacy manifests are not automatically repaired. See [format compatibility](docs/FORMAT.md#queue-staging-handoff-receipts).
- Rust callers must use read-only coordinator accessors (`state()`, `journal()`, `store()`, `limits()`, `epoch()`), publish through a separate `Store`, and replace explicitly typed `&mut Catalog` callbacks with `&mut LockedCatalog<'_>`. `Config::gc_enabled` returns `Result<bool>`. Existing Python call signatures are retained, with the retirement APIs added.
- Previously rounded floats cannot be reconstructed. Old rounded request hashes and retries generated under a different tensor chunk policy can conflict after upgrade; preserve retry identities and review the [stored-format rules](docs/FORMAT.md). Catalog recipes and receipts currently grow without compaction.

### Known performance regressions

- In the same JuiceFS workload with online GC, median write throughput fell 20.7% and acceptance time for 64 tasks rose 34.8% versus 0.1.3; reads and synchronous WAL appends showed no comparable decline. This short comparison used two rounds per build with warm caches. It was not profiled, so the changes cannot be attributed to a particular safety protocol or proof-related refactoring. See the [complete samples and build hashes](verification/measurements/20261026-release-comparison.json).

### CI and verification

- Scan source distributions and wheel contents, including native libraries, for local paths and recognizable credentials. Remap Rust build paths before compilation so released binaries omit local source, home and toolchain locations; CI checks artifacts before upload and rejects findings without printing matched secrets.
- Require TLA+/Verus checks, semantic negative controls, environment-contract audits, Rust/Python tests, formatting and Clippy before release builds and publishing. TLC records its jar hash without enforcing a checksum; Verus logs distinguish successful proofs from expected semantic rejections. Verification files are organized by protocol, WAL, GC, storage and offline recovery, with independently reviewable environment assumptions.
- Freeze new formal-verification work for this release. Integrated proofs cover specific production bodies and models; complete retain/reactivation, cross-log recovery, authentication and liveness composition remain deferred and receive regression/fault-injection coverage. That scope describes the historical release. Passing tests does not complete these proofs, and prior performance measurements do not establish this build's throughput or latency.

## 0.1.3

- Retry temporarily zero-filled bytes anywhere in a published extent's header,
  including partial and noncontiguous visibility gaps. Previously, only an
  entirely zero-filled header triggered this retry. The retry limit and checksum
  validation remain unchanged; other incorrect header bytes fail immediately.

Existing callers and stored data remain compatible. Packed-record and WAL
formats are unchanged.

## 0.1.2

- Stream large record payloads through bounded native write buffers. Records and
  publications no longer need to fit the writer's scratch buffer; existing
  packed-record framing and tensor reads are unchanged.
- Remove fixed logical quotas on record and dependency counts, result sizes,
  queue work, control messages, and metadata. Legacy limit arguments remain
  accepted for API and queue-identity compatibility; `max_buffer_bytes` still
  bounds temporary payload copies.
- Retry transient shared-filesystem visibility gaps when a published extent's
  length is visible before its header bytes. The reader reopens the file after
  short delays; persistent failures, nonzero bad headers, and checksum errors
  remain errors.

Existing stored data and callers remain compatible. These write and read
behaviors require 0.1.2; packed-record and WAL formats are unchanged.

## 0.1.1

- Persistent task scheduling with `TaskSpec.priority` (higher first),
  `scheduling_key` (lower first), then FIFO. Both keys default to zero and accept
  signed 64-bit integers. The Rust pending index selects tasks without reading
  their payloads and reconstructs scheduling order during WAL recovery.
- `Coordinator.yield_tasks()` atomically saves a batch of continuation inputs
  and optional scheduling fields, ends their leases, and returns the tasks to
  the pending queue without spending failure retries. Returned tasks join the
  tail of their scheduling key's FIFO order. Identical requests are idempotent;
  a stale lease rejects the entire batch.
- `Coordinator.pending_tasks()` exposes pending task specifications in
  acquisition order, with optional task-prefix and minimum-priority filters.
  Applications coordinate paused snapshots and retain the referenced inputs.
- Regression coverage for priority/FIFO recovery, stale leases, invalid
  scheduling keys, and all-or-nothing batch recovery across WAL crash windows.

Existing callers may omit the new scheduling fields. Packed-record and WAL
framing are unchanged; the new APIs and scheduling behavior require 0.1.1.
Pending-task snapshots alone do not provide a joint application checkpoint or
arbitrary checkpoint rollback.

## 0.1.0

Initial release of straw, a filesystem-based durable queue and shared tensor
store for AI applications, distributed as `straw-queue` under the MIT license.

- Rust storage, WAL, queue and ownership protocols with Python APIs. Multiple
  machines can read and write through a shared filesystem; the initial shared
  storage target is JuiceFS.
- Packed immutable records and embedded manifests, batched publication and
  size-based file rotation, avoiding a file per sample or tensor.
- Bytes and custom codecs, NumPy/PyTorch tensors, checked lazy row reads and
  native read sessions that reuse authenticated indices.
- Shared tensor references across queues, tensor-level copy-on-write, explicit
  reader/checkpoint ownership and optional online GC of unreferenced sealed packs.
- Durable task leases, continuation progress, completion receipts and consumer
  state, with WAL recovery and retained-checkpoint restore.
- Local and multi-host I/O benchmarks, workload profiling and replay, an
  independent protocol model, and process-crash/recovery checks.
- Linux x86_64 wheels for CPython 3.10–3.13, automated artifact tests and PyPI
  Trusted Publishing, standalone examples and paired English/Chinese guides.

Each queue requires one externally fenced coordinator. Automatic failover,
live-pack compaction, journal compaction and a general distributed application
checkpoint transaction are not provided. Additional network filesystems require
deployment validation. See the [filesystem requirements](docs/FILESYSTEM.md) and
[protocol guarantees](docs/DESIGN.md).
