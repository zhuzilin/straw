# Protocol basis and correctness verification

[中文版](PROTOCOL_AND_VERIFICATION_zh.md)

Status: experimental infrastructure, not a production durability certificate.
The Rust implementation, finite protocol model and real-filesystem tests provide
different evidence; none alone proves the whole system. This document describes
what the code implements and what still needs qualification.

## Prior work and what straw takes from it

| Area | Primary reference | Relevance and boundary |
| --- | --- | --- |
| Recovery ordering | [Mohan et al., ARIES, TODS 1992](https://research.ibm.com/publications/aries-a-transaction-recovery-method-supporting-fine-granularity-locking-and-partial-rollbacks-using-write-ahead-logging) | Make the durable log the authority and define recovery explicitly. straw uses immutable data plus redo of metadata transactions; it does **not** implement ARIES pageLSNs, undo or compensation records. |
| Packed immutable data | [Rosenblum and Ousterhout, The Design and Implementation of a Log-Structured File System, TOCS 1992](https://www.cs.cmu.edu/afs/cs/academic/class/15712-f08/www/readings/Rosenblum92.pdf) | Append many logical objects to segments; reclaim at segment granularity. straw currently deletes entirely dead packs; it does not implement a segment cleaner that moves live records. |
| Shared references | [Birrell and Wobber, Distributed Garbage Collection for Network Objects, 1993](https://www.microsoft.com/en-us/research/publication/distributed-garbage-collection-for-network-objects/) | Model holders explicitly and order reference transfer before source release. straw uses durable owner sets under a shared catalog lock, rather than implementing this paper's distributed collection algorithm. |
| Concurrent readers | [Michael, Hazard Pointers, TPDS 2004](https://research.ibm.com/publications/hazard-pointers-safe-memory-reclamation-for-lock-free-objects) | The analogous obligation is protecting an actual reader before reclaiming its data. An expired work lease does not prove the reader stopped. In-memory hazard pointers themselves do not solve durable ownership after process failure. |
| Observable consistency | [Herlihy and Wing, Linearizability, TOPLAS 1990](https://www.cs.cmu.edu/~wing/publications/HerlihyWing90.pdf) | Define one atomic observation point for each operation and respect real-time order. An operation whose reply is lost may have committed; retries must preserve its identity. |
| Protocol models | [Newcombe et al., How AWS Uses Formal Methods, CACM 2015](https://www.amazon.science/publications/how-amazon-web-services-uses-formal-methods) | Exhaustively explore small concurrent models, including recovery, before trusting a protocol. Model validity and correspondence to implementation remain separate obligations. |
| Implementation simulation | [Zhou et al., FoundationDB, SIGMOD 2021](https://www.foundationdb.org/files/fdb-paper.pdf) | Deterministic scheduling and fault simulation make difficult failures reproducible. straw does not yet have FoundationDB-style simulation of its complete implementation. |
| Filesystem crash behavior | [Pillai et al., OSDI 2014](https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai), [Mohan et al., CrashMonkey/Ace, OSDI 2018](https://www.usenix.org/conference/osdi18/presentation/mohan) | Successful process-crash tests do not establish power-loss safety. Persistence ordering and small exhaustive crash workloads must be checked against the actual filesystem contract. |

These are design and verification references, not a claim that a combination of
familiar techniques inherits their proofs. The current implementation was not
formally derived from any one paper.

## Scope and assumptions

* One externally fenced coordinator per queue. `exclusive_owner` documents an
  external guarantee; a nonempty string does not enforce fencing. No automatic
  leader election, consensus or partition-tolerant failover is implemented.
* Multiple queues may share one immutable storage pool/run, using
  `Coordinator(..., queue_id=..., namespace=True)`. Each has a separate journal;
  the pool has one `storage.log` with durable owner sets and pack states.
  Enable GC when creating the pool; migration of existing untracked queues is
  not implemented. Queue/staging owner names are internal and applications must
  not overwrite or release them directly.
* The storage catalog serializes metadata mutations and collection with a
  cross-client advisory file lock. All writers/collectors must participate.
  The inode is never renamed/replaced. Inside the lock, I/O reopens the log;
  keeping an old handle open is not assumed to invalidate remote caches.
* Supported deployment is Linux/POSIX. A file fsync makes its preceding bytes
  durable/visible; directory fsync makes newly created entries durable. The
  storage service must actually provide those guarantees. Errors stop the
  operation; a failed sync is not evidence that no bytes persisted.

* For JuiceFS, qualify distributed locks, fsync and close-to-open behavior on
  the deployed client/service, with writeback and open-cache disabled. See the
  [JuiceFS cache contract](https://juicefs.com/docs/community/guide/cache/).
  A successful four-client workload is evidence about that run, not a storage
  service power-loss certification.
* Payloads and references are immutable. Pack paths are fresh UUIDs. There is
  no reuse of reclaimed paths and no in-place tensor overwrite. COW currently
  copies a **modified tensor**; it is not chunk/page-level COW.
* A manifest dependency graph describes reachability. Raw serialized reference
  bytes alone are not a lifetime owner. Readers must be covered by a task,
  unfinished batch, explicit `pin`, another queue or retained checkpoint.

A client can still cache attributes for a pack another client has already
deleted. After a durable tombstone, `unlink` returning `ENOENT` is an idempotent
success; it does not count as another reclaimed file or byte. Other unlink
errors remain fatal. The four-client cached-attribute probe in
`verification/gc_cross_client.py` reproduced this case on JuiceFS, including
an actual stale successful stat followed by `unlink(ENOENT)`. A deterministic
fault test covers that boundary and confirms unrelated unlink errors propagate.

## Durable order and acknowledgment

A writer registers a pack as open **before** creating/appending its bytes.
Before publishing a manifest with external dependencies it durably retains
those dependencies under an operation owner. It then writes and syncs the
payload, persists staging ownership of the returned publication's transitive
pack closure, and releases the operation owner. Rotation/close seals packs.
An uncertain or crashed writer is not declared sealed by age or timeout.

Queue acceptance proceeds as follows:

```
retain destination queue roots in storage catalog; fsync
append queue transaction; fsync
apply queue state
consume staging ownership
reply
```

The queue journal's durable transaction is the acceptance point. The catalog
and queue log are **not a distributed atomic transaction**. The ordering makes
interrupted transfer retain too much data instead of leaving an accepted
reference unprotected. Recovery replays the queue log; a later collection
prunes that queue's roots against recovered state. In-progress/staging leaks
without a trustworthy completion signal are retained conservatively.

Collection uses the transitive pack sets already persisted for these immutable
queue roots. It checks that every live root has valid catalog ownership, then
prunes dead owners and collects packs. It does not reopen every live payload
graph under the coordinator/catalog locks; GC is not a data-integrity scrub.
Missing ownership fails before pruning/unlinking. This relies on the same
retain-before-WAL ordering, including after recovery, and avoids a cold-cache
metadata scan blocking leases and continuation writes.

A leased task retains its input and published continuation versions until
completion, explicit failure or yielding acknowledges the end of that attempt's
reads. Cancellation, timeout and coordinator epoch change revoke authorization
but preserve the old reader roots. `release_task_reads([lease])` acknowledges
actual read completion even for a stale lease. `retire_worker` is valid only
after the hosting system confirms that worker stopped.
`outstanding_reads()` exposes those leases for a supervisor's explicit stopped-
job acknowledgment; it does not declare them dead by elapsed time.

Discarded tensor results must also release their publication staging roots.
`release_tensor_publications` releases entire source publications (including
sibling tensors published together), while adopted queue/checkpoint owners stay
independent. An application can release rejected output staging before ending
the task's ownership of its previously adopted continuation.

For the AI training consumer, `processed_cursor` plus sparse processed positions
acknowledge raw results, while `finished_batches` acknowledges that **all training
ranks** finished their batch reads. Pending/unacknowledged batches retain their
plan and output dependencies. Additional readers need explicit roots; a cursor
alone is not a promise to retain arbitrary historical payloads.

An explicit training consumer save may rewind the processed prefix, sparse
positions or finished batches. Before its queue WAL commit, the native core
adopts any reactivated receipt/batch roots whose independent owners were pruned.
Their entire reachable data must validate while the catalog lock is held, and
every pack must still be protected by a durable source owner (for example a
checkpoint). Missing, corrupt, reclaimed or unprotected data rejects the save
before the consumer WAL changes. Existing live owners are reused without a
payload scan. Restored replay boundaries are journaled with the consumer state.
GC does not repair missing owners.

A checkpoint is a separate owner. Save its roots before publishing the external
checkpoint pointer. Overwrites use a distinct physical-reference version in the
owner ID so the previous checkpoint cannot lose protection before replacement
is durable. Record that owner in the application's checkpoint manifest and drop
it only after explicitly discarding that checkpoint version. Replaced versions and
crashed staging writers can conservatively leak. Joint model/optimizer/queue
checkpoint finalization still requires its own end-to-end qualification.

## Collection and invariants

Collection first journals the expiration of acknowledged replay history. It
reconciles **only its queue's** roots against live tasks, issued readers,
unprocessed results, current consumers, unfinished batches and checkpoints.
Other queues and explicit application owners remain independent.

While holding the shared catalog lock:

```
candidates = sealed packs - union(all owners' transitive pack closures)
append durable tombstones for candidates; fsync
unlink candidates; fsync parent directories
```

Retain rejects tombstoned paths. If the collector dies after marking but before
unlinking, another collection retries the unlink. Any live extent holds its
entire pack. Unregistered packs are never guessed to be dead from a scan.
The offline orphan inspector refuses shared-catalog/multi-queue roots.

The core safety obligations are:

1. Every acknowledged, unreleased reference has durable payload and durable
   reachability protection, including all transitive tensor dependencies.
2. Retiring one owner cannot invalidate another queue, active reader or retained
   checkpoint. Work-lease expiration is not read completion.
3. No open or reachable pack can be tombstoned. No pack can be unlinked before
   its durable tombstone. No new owner can resurrect that path.
4. A complete corrupt log frame fails closed; it is not treated as an incomplete
   suffix. Only a valid durable prefix plus an incomplete final append may be
   recovered. Header length/sequence and body/trailer are checksummed.
5. An uncertain reply is resolved by operation identity. Recovery preserves
   accepted facts and revokes stale authorization; it does not promise each
   physical computation or optimizer update ran exactly once.

Safety is deliberately stronger than reclamation progress. A permanently lost
writer/pin can retain storage indefinitely; there is no TTL-based reclamation,
live-record compaction or bounded metadata/journal history yet.

## Executable evidence and remaining work

| Layer | Available now | What it does not establish |
| --- | --- | --- |
| Finite protocol model | `verification/ownership_model.py`: fixed-point exploration of two queues, one immutable pack, one reader, one checkpoint and one crash. Five unsafe mutations, including cursor rewind without root adoption, must produce counterexamples. | Not a TLA+/TLAPS proof, not an unbounded proof, and not verified refinement of Rust. No multi-pack dependency cycles, arbitrary retries, filesystem cache model or liveness proof. |
| Native log campaign | `verification/crash_campaign.py`: every byte prefix and one XOR mutation at every byte of an appended frame, for both WAL formats; abrupt child exit at catalog and queue WAL boundaries followed by recovery/GC. | Prefix persistence is only one storage fault model. Process exit leaves the kernel alive; this does not simulate torn device sectors, reordered block persistence or server failure. |
| Regression/integration | Rust tests, Python tests driving the native core, executable record/tensor/queue examples. | Finite examples do not establish completeness. |
| Four-client storage workload | `python -m straw.benchmark run --online-gc ...`: concurrent writers/readers and collection, then reader acknowledgment and measured reclamation. | Fresh paths are not cold-cache evidence; syscall/application payload rates are not backing-device throughput. |
| Cached-attribute and root-scale probes | `verification/gc_cross_client.py` and `verification/gc_scale.py`, on distinct filesystem clients. | A bounded run does not qualify arbitrary mount settings, service failures or unbounded histories. |

Application training/recovery tests belong to the consuming application. Their
results are useful integration evidence but are not a core correctness proof.
See [application boundaries](APPLICATIONS.md) and [verification commands](VERIFICATION.md).

Reproduce the inexpensive layers:

```bash
cargo test
cargo clippy --all-targets --features python -- -D warnings
python -m pytest
python verification/ownership_model.py --output /tmp/straw-model.json
python verification/crash_campaign.py --output /tmp/straw-crashes.json
```

The native crash campaign also accepts `--hosts <four hosts> --parent <shared mount>`;
its handoff cases run on four separate clients. The private campaign root is
removed automatically, and the caller chooses an output report outside it.

Create fresh stores and coordinators after process fork; inherited mutable
writers, mutexes and file-lock handles are outside the supported contract.

The model and native crash campaign are registered in CI. Local execution does
not mean the hosted CI matrix ran.

Before calling this production-ready, the next validation layers should be:

* Expand the model to multiple packs/dependencies, retry identities, coordinator
  epochs and checkpoint replacement; check fairness-conditioned reclamation
  progress separately from safety. A reviewed TLA+/PlusCal specification and TLC
  runs are a good way to make this model independently inspectable.
* Introduce a narrow filesystem/clock/scheduler interface into the Rust core,
  then run seeded deterministic simulations against a separate sequential
  reference model. Replay each failing seed and shrink it to a short history.
* Record real client invocations/responses and check their histories against
  the queue specification, including incomplete operations. Tools such as
  [Knossos](https://github.com/jepsen-io/knossos) check a supplied linearizable
  object model; they do not infer the desired queue contract automatically.
* In an isolated qualification environment, interrupt client and storage-service
  processes, delay or lose RPC replies, inject ENOSPC/EIO/short writes, and test
  VM/device power loss. Never power-cut a shared training cluster as a test.
* Run long-duration workloads with leak/orphan accounting, checkpoint retention
  churn and bounded recovery time. Review the protocol and filesystem assumptions
  independently before enabling automatic failover or pack compaction.

If highly available metadata or strict multi-queue transactions become a
requirement, evaluate a mature transactional/consensus metadata service rather
than quietly growing this single-owner log into a new consensus implementation.
Moving an embedded database onto JuiceFS would itself need filesystem-locking
and crash-durability qualification; it is not automatically a correctness fix.
