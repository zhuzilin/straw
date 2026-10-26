# Formal verification: TLA+ and Verus

[中文版](FORMAL_VERIFICATION_zh.md)

**Core implementation refinement is incomplete.** See the [call-chain correspondence](VERIFICATION_BOUNDARIES.md). Passing the existing models and helper proofs does not close native WAL/lock adapters, all derived indexes, coordinator composition or the GC call chain.

Environment assumptions are extracted into independently reviewable modules and
a stable-ID inventory; see [environment models](ENVIRONMENT_MODELS.md).

Each contract separates its provider, straw implementation obligations and
unproved correspondence. Use `python verification/runners/environment_manifest.py --list-layers`
to list component layers, `--layer durability` to review the persistence
environment, `--model GCReceipt` for that GC model's premises across layers, or
`--contract ENV-GC-IO` to find affected models; see the
[independent review entry point](ENVIRONMENT_MODELS.md#evidence-and-independent-review).
Component layers are distinct from the L0–L5 evidence levels below.
Passing inventory checks does not mean L3 code has proved the L0 assumptions.

Review the contracts consumed by each component: an upper layer's environment
can include straw's own lower-level implementation. A violated assumption still
requires distinguishing an incomplete model, an incompatible deployment and an
adapter bug; it does not directly establish a JuiceFS fault. See
[who must satisfy each contract](ENVIRONMENT_MODELS.md#who-must-satisfy-a-contract-between-layers)
and use the [incident review record](ENVIRONMENT_MODELS.md#incident-review-record)
to assess safety and progress impacts separately.

The control plane now uses an **Anvil-style, incremental verification boundary**:
production Rust controllers expose a `View`, each executable step is proved to
match its mathematical step, and separate model theorems establish safety and
conditional progress. This is implemented for commit ordering, filtered lease selection and GC
selection / deletion; it is not a proof of all straw Rust or complete recovery.

## Verification scope for this release

This release freezes new formal-verification work and keeps the integrated TLA+, Verus and semantic controls as required checks. Remaining implementation refinement and liveness proofs are deferred. Regression, fault-injection and recovery tests exercise those paths for this release; passing tests does not close a proof obligation.

| Incomplete proof boundary | Runtime checks for this release |
|---|---|
| Locked catalog retain/reactivation, closure caches and complete callers | `test_online_gc.py`, `test_consumer_rewind.py`, `test_dependency_traversal.py` |
| Queue/catalog handoff, staging release and complete recovery composition | `test_durable_handoff.py`, `test_commit_consumption.py`, `test_staging_consumption.py`, `test_request_replay.py` |
| Complete extent authentication and independently visible regions | `test_extent_visibility.py`, `test_extent_bounds.py`, `test_tensor_preflight.py` and native Reader regressions |
| Partial native replay, cache quarantine and public entry-point composition | `test_wal_recovery.py`, `test_metadata_recovery.py`, `test_journal_contract.py` and native Journal regressions |
| Complete crashed-writer publication/ownership/GC composition | `test_writer_retirement.py`, `test_pending_publication.py`, `test_incomplete_publication.py` |
| Sampling continuation, training checkpoints and GC composition | `test_resume_workflow.py`, `test_task_recovery.py`, `test_queue_gc_roots.py` |

Real scheduling, eventual lock/I/O progress, complete cross-layer safety and conditional liveness remain unproved end to end. Filesystem, external fencing and application owner-release assumptions remain explicit. Tests cover exercised executions; performance evidence separately identifies the tested build and workload.

Sampling resumes only from a durable continuation. The training application selects and loads a checkpoint, restores matching model/optimizer state and data cursors, and protects any post-checkpoint data needed for replay. straw neither loads the model automatically nor restores legitimately reclaimed data.


## Actual GC mark-to-receipt chain

The production `gc_collection::Batch::run` now proves the complete lower collection chain: durable mark, exact pending snapshot, all target unlinks, shared-parent barriers, missing-parent ancestor repair and durable receipt. Success establishes `after_collection(initial)`, empty pending and durable absence for every original target. Returning errors allow exactly the initial, marked or fully collected business-state prefix. The adapter preserves an explicit immutable context while its physical and log state change.

Filesystem path/handle identity, complete ancestor enumeration and individual unlink/sync behavior remain reviewed primitives in `ENV-GC-NAMED-PRIMITIVES`; the complete Batch/Repair/receipt algorithms are checked Rust. The outer locked refresh, complete root provenance, queue retirement before prune and conditional progress remain separate work. The [fixed collection record](../verification/measurements/20261026-gc-collection.json) preserves exact inputs, positive and semantic-negative checks, and native regression evidence. It does not certify later source changes or performance.

## Queue roots and retirement integration

The actual `queue_roots` traversals select nonterminal tasks, unprocessed commits, unfinished batches, readers, consumers, producers and checkpoints. Producer continuation roots were previously omitted: a valid saved `producer_state` could survive in the queue after GC deleted its only payload. The old installed wheel reproduced the failure; a frozen snapshot containing the fix passed 310 targeted Python tests, including preservation after reopening, state replacement, repeated requests and shared dependencies. Those results identify that snapshot, not later writer-retirement changes.

`queue_gc` now owns the actual retirement/owner preparation/locked collection dispatch. A private, non-cloneable token carries the selected roots only after the retirement call succeeds. Owner construction is exact; `require` rejects absent, empty or tombstoned ownership, and `prunable` selects only obsolete owners with the supplied queue prefix. `retirement_preserves_roots` derives preservation of all seven root fields from actual `queue_metadata::outcome` for `RetentionAdvanced`. It does not assume the complete frame rule.

The JSON library boundary adds one narrow representation law: Map entry keys have unique character content. A checked unfolding lemma connects its existing map representation to the field lookup; this adds no trusted straw algorithm. `ENV-QUEUE-GC-CALLS` records exact calls/results separately from durable lower-level effects. Native commit/context, progress-cache and complete deletion/receipt refinement remain open; a successful interface call is not by itself an end-to-end GC proof. Root recursion is proved for normal return, not unlimited stack/allocation or termination. The [integrated snapshot record](../verification/measurements/20261026-writer-roots-integration.json) binds 695 full-entry Verus checks, 127 native runtime tests, 13 compiler doctests and 947 installed-wheel Python tests. Subsequent source changes require fresh evidence.

## Native WAL file boundary

The production `wal_file` now uses the checked `file_adapter::NativeFile` and `NativeBuffer` bodies. A non-copy tracked permission holds mutable bytes, cursor and durable-prefix state; the raw standard File value exposes only descriptor identity. The adapter passes actual lengths, offsets and byte slices, transfers the same descriptor through buffering, hashes the two spans in order and propagates primitive failures. File/Writer validity is explicit throughout Source/Appender and the append callers.

Ten standard I/O wrappers in `file_primitives` are the pinned, audited trusted boundary (`ENV-NATIVE-FILE-PRIMITIVES`); their OS/standard-library semantics are not claimed as verified Rust algorithms. SHA/error and callback wrappers retain separate contracts. The constructor and returning mutable methods also carry checked borrow-return relations, using [Verus final-reference semantics](https://verus-lang.github.io/verus/guide/mutable-references.html). Production Journal recovery, append and root-retirement now call [journal_io](../rust/journal_io.rs), which constructs the native adapter, executes the checked WAL/business operation and returns the original File/permission on every Result path. Success binds the returned permission to the exact cursor, durable boundary and accepted event; recovery errors retain unchanged or quarantined state. The Source length-error path also returns its original loan. The [fixed chain record](../verification/measurements/20261026-journal-io.json) includes semantic controls that redirect work to a different State or mask a recovery failure. The later owning Journal refinement below closes initial Journal open and Run admission; lock authority, native callback bindings and complete coordinator composition remain separate obligations.

The production [journal_owner](../rust/journal_owner.rs) owns the same path, optional File and State used by the public Journal. Its creation branch requires complete ancestor preparation, opens a new file, synchronizes the file and containing directory, and appends the exact `Run(identity)` frame through `journal_io`. Recovery replays that descriptor and admits it only when its first transaction equals the expected Run header. The returned contracts retain the original bytes, decoded history and exact accepted-frame relation. `append_with` preserves descriptor identity, read-only mode and confirmed sequence/offset on errors; uncertain writes retain quarantine. `close` removes the descriptor without clearing state, and subsequent appends reject before hooks or I/O. The proof permission and initialization evidence are erased from ordinary builds.

Single OpenOptions behavior and Path representations are pinned in [journal_primitives](../rust/journal_primitives.rs). Native bindings still certify individual directory preparation, file/directory sync and fault callbacks; they do not supply a successful Journal initialization or identity check. Filesystem durability, exclusive ownership supplied by the caller’s lease or writer gate, and callback noninterference remain separate environment obligations. Native regressions cover wrong identities, creation over an existing journal, read-only and closed rejection, both initialization barriers, and a durable Run frame whose reply is lost. Existing append-unwind and failed-replay tests exercise quarantine and recovery.

The [owned-chain integration record](../verification/measurements/20261026-owned-chains-integration.json) fixes one identical set of proof/runtime inputs for Journal I/O, owned Catalog transactions, retirement preparation and writer recovery. It records the full positive check and native/installed-wheel regressions, and explicitly excludes later progress-cache and typed GC callback changes.

## Actual catalog namespace adapter

`catalog_namespace::Adapter` now implements the production Resolver trait in checked Rust. Its constructor, reset and resolve methods preserve an explicit `valid` relation tying the cache to the source namespace. That premise is propagated through field decoding, raw Target recovery/application, append, mark, receipt and queue owner release. The proof uses ordinary explicit preconditions; it adds no blanket panic-free premise. `NativeKeySet` invokes the same standard bulk construction as before, with a separately audited library law and no replacement insertion loop.

The actual borrowed component factory, PathSource construction/legacy dispatch, cache initialization, ordered component resolution and missing-target reconstruction are checked. `namespace_step` binds the actual per-component algorithm to individual readlink/canonicalize/Path primitives. Remaining native obligations are initial or reused namespace validity under the real catalog lock and faithful primitive correspondence. Stable aliases and truthful absence remain environment requirements; the complete Resolver or per-component algorithm is no longer assumed.

## Explicit writer retirement

`Store::writer_id()` and `Config::retire_writer` (Python `writer_id` / `retire_writer`) identify an exact incarnation. Every mutation acquires its permanent gate and checks the durable fence. Retirement refuses a held gate, persists fencing, drains/repairs payload files, reconciles the operation registry, seals reservations and records completion. Gate and registry files live under `control/writers`, independently of deletable payload directories.

Recovery stages all complete results before releasing registered preparation owners. Intermediate publication cleanup uses the same durable consumption identity in ordinary retries and retirement, so a lost reply cannot release staging reestablished by another client. Unknown legacy owners remain protected. Recover and abandon are explicit policies; there is no timeout-based takeover or automatic retirement daemon.

The actual `writer_registry::{decode,apply_event,apply_frame,replay}` chain connects the journal's decoded raw prefix to ordered registry business history through `journal_history_to_registry`. It checks strict schemas, valid-event admission, monotonically registered operation IDs, immutable dependency/result evidence and exact settlement. `reconcile_registered` uses those actual operations to select every validation/staging call, intermediate consumption identity, preparation owner and final settlement ID. Success produces the exact two-pass action sequence; an error can execute only its prefix, including a current leaf that took effect before reporting failure. Recovered receipts preserve earlier entries, cover every recovered result and introduce no value without recorded result evidence.

Production `writer_gate_flow` checks open/lock/inode/header/fence admission and owns the resulting handle. Its `bound` check rejects a changed named inode; marker persistence checks create-or-reopen, file sync and directory sync ordering. The actual drain loop synchronizes packs, deletes only enumerated partial names in the same writer scope and finishes with a directory barrier. Store publication/cleanup helpers borrow the owning permit; Rust ownership keeps that handle alive across the calls. `writer_ownership` checks registered-owner coverage from positive dependency-closure evidence and selects exactly the unsealed packs whose resolved parent is the writer directory. The native implementation invokes these checked routines rather than a second algorithm.

Recovery authentication reuses the checked `closure::collect` traversal and `closure_node::children`, with `authenticate_all` covering every result root. The native reader authenticates one manifest or record at a time; it no longer supplies an entire recursive recovery walk. Remaining native bindings are specific: descriptor/run/writer construction, standard map/directory snapshots, OS open/lock/metadata/read/unlink/sync and Path operations, single-node Reader authentication and the named catalog stage/consume/release/append calls. These are separate contracts and correspondence obligations; the whole retirement, owner-selection or reconciliation algorithm is not supplied as a filesystem assumption. `WriterRetirement` checks finite pause/death/retry and GC interleavings. Its environment requires earlier submitted I/O to finish or be cancelled before gate handoff, and subsequent fence/registry reads to observe the persisted state. Fresh open alone does not exclude a stale negative directory-entry cache. The four-host smoke confirms particular outcomes after a synchronous payload chunk, not arbitrary remote I/O drain, lock-handoff coherence or a latency bound. Locks and registry synchronization add runtime work; performance is pending.

The read-only `publication_receipt` query uses exact writer/API/submission identity and remains available after fencing. Its checked selector includes results whose internal Done/Retired record is already durable, covering a reply lost after completion. It does not re-stage data, modify ownership or restore references already consumed and collected; the [usage guide](USAGE.md#recover-a-writer-publication-after-a-lost-reply) describes this distinction.

The [writer admission record](../verification/measurements/20261026-writer-admission.json) distinguishes finite protocol modeling, checked gate/marker/drain algorithms and native Permit wiring. Old-writer exclusion and in-flight ordering are conditional on the named stable-gate/coherence/drain contracts; complete publication-to-ownership/GC composition remains separate.

## Rust ownership boundary

Coordinator business state, limits, epoch and its owned journal/storage now expose immutable views. Journal retains read-only dereferencing for sequence/transaction inspection, but has no public mutable dereference or mutable path. External Rust callers therefore cannot clear quarantine, edit recovered business state, append directly through the coordinator's journal, replace that journal, or change the owned storage configuration through these views. Direct `usage()` also performs the verified availability check.

This is Rust visibility/borrowing evidence, distinct from Verus transition proofs: eight new compile-fail examples reject those operations and a positive example checks inspection. Existing caught-apply-unwind/GC regressions also check that direct usage is rejected. The remaining native callback/context, filesystem identity and full distributed correspondence are not discharged by encapsulation alone.

Rust callers use `state()`, `limits()`, `epoch()`, `journal()` and `store()` for reads, `Journal::path()` for its location, and an independent `Store` for publication. Python APIs and persisted formats are unchanged. The existing `recovery_seconds` diagnostic remains directly readable.

## Actual application guard and fresh recovery

[queue_application](../rust/queue_application.rs) now supplies the production commit application's guard, callbacks, business update and confirmation. `arm` poisons the actual journal state before callbacks; `apply` only borrows that state immutably and returns a privately constructed confirmation after both hooks and the complete transaction succeed. `confirm` consumes it and clears only the poison flag. The existing commit dispatcher stores this confirmation until its success branch. `permitted` is the actual entry check: poisoned or closed coordinators accept only diagnostic `state` queries. It adds no I/O and does not change diagnostic access.

`application_result` distinguishes failure before any business mutation, partial transaction failure and callback failure after complete application. `hook_result` proves the exact before/after invocation and outcome sequence under the separate `ENV-QUEUE-APPLICATION-HOOKS` interface. The lower callback may fail or have external effects; success, termination and unwind rollback are not guaranteed. The native callback-name binding and complete CommitIo/context correspondence remain reviewed/tested adapters, not proved by merely declaring that interface.

`restore` constructs business state from the verified initial state and exposes a `Restored` value only after full replay. Any return error keeps the journal poisoned; retrying the same poisoned decoded state is rejected. Physical recovery must establish a new usable journal first. `rebuilt_fold` and `journal_equivalence::restored_prefix` connect this fresh result to the accepted byte-prefix history, independently of any prior partially applied business state. Derived caches/bookkeeping, native physical identity, public mutable Rust fields and full catalog/GC composition remain separate obligations. Normal-return Verus proofs and caught-unwind native regressions are distinct evidence; the latter reuse the existing commit-apply panic regression.

A specification sensitivity check deliberately removes only the hook-result postcondition and ignores the after-hook error: the remaining business-only proof passes. Keeping the trace postcondition makes the same code mutation fail. This demonstrates why callback error propagation needs its own specification; it is not evidence of a new production defect.

The [application record](../verification/measurements/20261026-queue-application.json) contains exact proof inputs, callback/guard/recovery semantic controls and native/wheel checks. No new production defect or zero-overhead claim is made; performance remains deferred.

## Queue business histories and WAL composition

The production [queue_history](../rust/queue_history.rs) now connects initialization, event preparation, complete target admission, business dispatch, cache callbacks and final receipts. Commit uses its `transaction` loop; recovery uses `journal_state::State::apply_business`, which borrows that journal's actual decoded transaction vector. Recovery skips only the first identity frame. The actual `journal_owner::open` now checks that first frame against the requested Run identity before returning the owned Journal.

`transaction_result` covers every event in input order. Failure identifies the exact successful prefix and the failing event's partial effects; it does not roll back already executed operations. `recovery_result` extends this across arbitrary finite transaction sequences and distinguishes bookkeeping failure before a frame from application failure inside it. `frames_fold` proves the transaction sequence equals the same ordered flattened event history. `journal_equivalence::physical_prefix_business` composes the postconditions of actual WAL recovery and business application: the final business state is the fold of the **actual Values decoded from the accepted byte prefix**, excluding the header. It does not substitute an independently reparsed or merely observationally equal history; the existing numeric model is still limited to unsigned observations.

The state in this theorem contains all business JSON roots plus epoch, scheduling-order counter and sealed flag. The initial constructor proves empty tables/arrays and zero retired prefix. Cache and recovered-consumption hooks receive immutable business inputs and separately borrowed mutable state. Normal callback return cannot change this business state; no cache-content correctness premise is supplied. Scheduling/usage/deadline consistency, bookkeeping IDs, callback termination/unwind, native file/run/fencing correspondence and cross-log/GC ownership remain open. Those boundaries are distinct from the now-proved event and transaction loops.

The [history record](../verification/measurements/20261026-queue-history.json) records exact proof inputs, semantic mutations and native/wheel checks. Tests cover admission before business effects, partial body failures, receipt failure after cache refresh, frame/error indexes and mixed histories. Recovery no longer clones the entire transaction history. No new I/O or persistent-format change is introduced; this is not a throughput measurement.

## Actual queue event dispatch and worker retirement

[queue_replay](../rust/queue_replay.rs) is the business dispatcher called by `queue_history::apply`. Its `body_result` binds each event kind to the existing task/reader/receipt/metadata relation, including epoch, bounded order and sealed-state frames. Lease and failure batches preserve input order and duplicate IDs. Errors expose the complete successful prefix plus any effects of the failing item; a failed task retry can already have released its reader. `receipt_flag` identifies the metadata branches that recorded a request, and `finish` records other requests while leaving already-recorded state unchanged. The native deadline callback remains at the original success points, including within a partially completed batch. Its cache contents are not part of this business-state proof.

`queue_reader::retire` proves the actual worker predicate and exact retained map. `queue_admin::reader_retired` binds the event's worker ID to the queue's reader table. Only matching workers are removed; every peer reader value and its complete roots remain unchanged, as do other queue roots. Missing/wrong-shaped reader tables return defined errors. Missing worker fields retain the old null-index/equality behavior; API authorization is a separate obligation. The callback body is written once, with a verifier-only closure contract; native builds use stable Rust and the original in-place `Map::retain` algorithm.

`ENV-JSON-LIBRARY` adds **one external-library primitive**, `Map::retain`, bringing the current audited surface to twenty-one methods. On normal return it covers every original key, connects the callback's decision and final value to retention, and forbids new keys. It does not assume the straw predicate, event binding or peer preservation. The predicate uses immutable captures and does not modify entries. Stateful mutable captures, library internals, callback panic rollback and visitation order are outside this contract's reviewed use. The specification follows the pinned library's delegation to `BTreeMap::retain` and the verifier's [callback contract semantics](https://verus-lang.github.io/verus/guide/exec_funs_as_values.html).

The review also found missing capacity preflights in batch `yield_tasks` and `release_tasks`. Both now check the number of new orders after cached-response lookup and before retain/WAL writes. An extended native test forces the private counter to its maximum: it failed by entering a forbidden write callback before the fix and passes after it; isolated removal of each new guard also fails that assertion. This is defensive capacity coverage, not a practical valid-workload overflow reproduction. The [dispatch record](../verification/measurements/20261026-queue-replay.json) separates actual-body proofs, semantic negatives and native/wheel regressions.

The history composition above now connects preparation/target admission to the dispatcher and completes the business event/transaction fold. Derived scheduling/usage/deadline caches, root extraction, run/lock/File adapters and cross-log/GC ownership remain open. No additional filesystem operation, persistent format or payload copy is introduced. Capacity checks execute at runtime; performance testing remains deferred.

## Actual catalog transaction entry and exit

Production `Catalog::transaction` now calls the same [catalog_transaction](../rust/catalog_transaction.rs) bodies checked by Verus. Under the separate `ENV-CATALOG-TRANSACTION` contracts, `execute` and `locked` require correct pool/run binding, reject nested entry without changing the outer context, hold the original lock while opening a fresh handle, check device/inode before installation, and recover before enabling the callback. The callback executes at most once. Successful return requires its final context to remain bound and ready; returned errors clear a newly entered active guard. `finish` proves exact error precedence: an operation error survives a simultaneous unlock error, while an unlock error prevents an otherwise successful return.

The model deliberately permits a callback to leave the catalog unready or replace it with an inactive context; final validation must reject that state. Legal callbacks that leave an active context must preserve the transaction identity and recovery provenance. This is a separate native/caller correspondence obligation, not a guarantee about arbitrary Rust callbacks. Lock/open/metadata primitives, native refresh/context adapters, complete durable history and contention progress remain unproved. A failed acquire or unlock can retain effects; there is no unconditional release guarantee on such errors. Result-path proofs do not prove unwinding. Native tests separately check lock exclusion/release, persistence before a callback error, and that a caught callback panic requires reopening before reuse.

The [transaction record](../verification/measurements/20261025-catalog-transaction.json) binds exact inputs, semantic negatives and native/wheel results. The extraction retains the existing filesystem call order and error messages, using stack wrappers and one callback slot; no heap allocation, filesystem barrier or payload copy is added. There is no new performance claim.

Native boundary review reproduced two out-of-lock write paths. After a caught callback panic, direct `Catalog::pack` could still see an active/ready context; the callback unwind guard now invalidates the log. A Rust callback could also use `mem::replace` to take the active Catalog away and keep writing after its lock was released, even while another descriptor held that lock. Callbacks now receive a private borrowed `LockedCatalog`, with read-only Deref and explicit forwarding of operations. Rust lifetime/type checks reject ownership extraction, mutable-dereference extraction and escaping the callback borrow. These are compiler/regression checks, separate from the Verus dispatcher proof.

**Rust API migration:** `Catalog::transaction` and `Config::catalog` callbacks now take `&mut LockedCatalog<'_>`. Closures with inferred argument types and normal operations keep their call syntax; explicitly typed `&mut Catalog` callbacks must change their annotation. Python APIs and persisted formats are unchanged. The native callback/context mapping and complete durable-history proof remain open.

## Actual replay and append descriptor dispatch

Production `wal_file::Source` and `Appender` now use the same [wal_source](../rust/wal_source.rs) and [wal_sink](../rust/wal_sink.rs) bodies checked by Verus. `Source::from_file` establishes the device validity predicate; returning recovery paths preserve it through actual WAL, catalog Target and queue recovery. The predicate binds the cached length whenever the source is available. Reads, truncation and seeks arm uncertainty before effects, keep failed attempts unavailable, and restore the owned handle before returning a seek error. Buffer consumption preserves the file identity, bytes and durability while allowing the exposed physical cursor to differ from the buffered logical cursor. Final seek reestablishes the requested position; a synchronization callback is consumed once.

`Appender` establishes the Writer contract from exact metadata/position observations, the supplied write spans, queue/catalog synchronization and existing fault-hook order. Both dispatchers keep the same abstract file identity across every returned result. `ENV-FILE-PRIMITIVES` separately records File/BufReader/Take, SHA and callback semantics; thin native wrappers, snapshot freshness, successful multi-read consistency, actual lock/open/run binding and physical I/O correspondence remain lower obligations. No straw body is admitted. Error-return proofs do not establish arbitrary Rust unwind semantics; a native caught-unwind regression separately checks that a barrier panic after a file effect leaves the source unavailable.

This review corrected an overstrong read contract: an empty read can succeed past EOF without moving the cursor. `read_matches` now admits that behavior while retaining exact bounds/content for every nonempty read; `empty_read_matches_any_cursor` prevents the specification from excluding it again. Actual buffered and plain-file tests exercise this case. It is a specification repair, not evidence of production data corruption. The [Rust Read interface](https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact) is an external library boundary.

The [file-adapter record](../verification/measurements/20261025-file-adapters.json) binds exact inputs, positive proofs, semantic controls and native/wheel regressions. Dispatch extraction preserves payload copies, buffer-capacity policy, filesystem calls and existing fault points. It adds defensive rejection of direct buffered truncate/sync calls that violate the existing Device preconditions, and arms read uncertainty before allocation. No performance campaign or zero-overhead conclusion is included.

## Actual dependency traversal and closure cache

[closure::collect](../rust/closure.rs), called by production `Catalog::closure_inner`, now proves that a successful traversal returns exactly all reachable physical packs of a finite immutable graph. The actual heap frontier handles repeated edges, cycles and shared packs. Cached subgraphs contribute their complete reachable pack sets; only a complete requested root is inserted, and cache eviction preserves validity. A read error leaves the closure cache unchanged. The empty-cache base is proved separately. The loop's decreasing set counts unvisited nodes, so traversal terminates if its per-node calls return; this supplies no I/O deadline or resource guarantee.

Actual `closure::resolve` composes this result with the tombstone scan: successful output is disjoint from the deletion set, and a `Reclaimed` error requires an actual reachable tombstone. Both cold and cached closures are checked. This establishes the set passed back to retain callers under a valid graph, correctly bound root and valid initial cache. It does not prove those native caller premises or catalog locking/append placement.

The actual [closure_node](../rust/closure_node.rs), used by `ClosureReader`, now proves field selection and manifest successor construction. Manifest field presence takes priority over a direct segment, including a null manifest. Both successor arrays are checked before hashing; successful output contains every record followed by every dependency, preserving duplicates, cloned content and each digest/reference pairing. Hash errors return no partial list. `complete_keys` and `graph_binding` connect that list to the traversal interface under explicit graph-domain and key-context premises. The [node record](../verification/measurements/20261025-closure-node.json) records the exact proof inputs, eight semantic negatives and runtime checks.

`ENV-CLOSURE-SOURCE` still isolates authenticated manifest/index reads, native serialization/SHA correspondence, canonical pack mapping and graph/cache context. Those native premises and the complete `ClosureReader` implementation correspondence remain unproved. No new external-library primitive or admitted straw function was added. Extraction materializes a temporary child vector per expanded manifest and clones the supplied root before a cache hit; field selection now uses the existing verified object-iteration helper. These are executable allocation/lookup changes. Payload I/O, synchronization and child order are preserved, but zero overhead is not claimed.

The [closure record](../verification/measurements/20261025-dependency-closure.json) binds the exact inputs and results: full positive Verus verification, thirteen semantic negatives, native cycle/shared-pack/cache/failure tests, and real-file retry/retain/GC regressions. Negatives require semantic rejection for missing successors/packs/cached descendants, partial cache insertion, wrong identity binding, wrong cached roots and incorrect tombstone acceptance/rejection. Full node-source refinement and complete ownership/GC progress remain open; no new performance campaign was run.

## Actual GC discovery, durable mark and pending snapshot

Production `Catalog::collect_with_stat` calls [gc_mark::mark](../rust/gc_mark.rs). Verus now connects actual owner-root enumeration, complete sealed/unowned/non-tombstoned candidate selection, construction of the resolved-path JSON event, catalog serialization/append and the resulting pending snapshot. Success preserves owners, packs, edges and consumed handoffs; adds exactly the candidates to permanent tombstones; and returns every old pending deletion plus every new candidate. A nonempty candidate set produces one confirmed durable catalog frame before the deletion batch can start. No candidates means no mark I/O. Mark errors return before unlink; uncertain write errors leave the log unavailable until recovery.

The mark uses identities already normalized by catalog replay. It emits `path_encoding=1` and retains the existing resolver's lexical validation. This removes redundant normalization of catalog keys, changes no frame format and adds no filesystem barrier. Candidate array ordering follows the HashSet snapshot and is not a protocol guarantee. Existing alias/legacy-log regressions cover this path. This is a verification integration, not a newly discovered production defect or a measured performance improvement.

At the GC mark checkpoint the library whitelist had seventeen primitives, adding only empty `Map::new` and exact unsigned `Number::from(u64)` contracts. No straw function body is trusted. The input log must already be valid, ready and interpreted; Writer, Resolver, JSON and String contracts remain explicit. Complete application roots/closures, the native locked recovery/descriptor binding, `BatchIo`'s target/path/parent/receipt conformance and eventual discovery/retry are still unproved. The returned snapshot supplies the actual batch call, but this does not establish the whole outer native GC refinement.

The [GC mark record](../verification/measurements/20261025-gc-mark.json) binds the exact inputs, full positive proof, eleven semantic negative controls and native/wheel regressions. Negatives cover skipped/live candidates, wrong JSON fields/encoding, ignored append failure, lost old pending deletions and revisiting completed receipts. New integration cases cover all six mark-write/sync fault boundaries and recovery with old pending, new garbage and a protected reader. No performance campaign was run.

Production `BatchIo::receipt` now calls [gc_receipt::record](../rust/gc_receipt.rs), connecting the supplied canonical target set to resolved-path JSON, the durable catalog append and the exact reclaim fold. Success removes precisely those entries from pending deletions and pack records, preserving permanent tombstones and all owners, staging edges and consumed handoffs. The proof admits duplicate names and any already-marked subset; it does not clear unrelated pending work. The completed-receipt projection is `deleted − pending`: `mark_preserves_completed` proves that marking creates no completed receipt, and `receipt_completes_exactly` proves that this append adds exactly its targets to that projection. Empty input performs no append. Uncertain writes leave the log unavailable and its confirmed history unchanged, even though an unconfirmed complete frame may survive recovery.

The receipt now uses the canonical keys already selected from pending deletions, removing a redundant name-normalization/set round trip after physical deletion. The format and barrier order stay the same; target array order is not a protocol guarantee. Physical durable absence is still a caller obligation: native target/path/parent binding, directory repair/barriers, outer recovered-lock context and thin Writer/Resolver conformance remain open. This is an implementation-composition proof, not a new physical-deletion theorem or a newly demonstrated production bug. The [receipt record](../verification/measurements/20261025-gc-receipt.json) binds exact inputs, semantic negatives and existing native/wheel failure-recovery regressions; no performance campaign was run.

## Multiple queues, requests and shared ownership

[CrossLogComposition](../verification/protocol/tla/CrossLogComposition.tla) and [cross_log](../verification/protocol/verus/cross_log.rs) combine queue-scoped request IDs, durable acceptance, lost apply/reply, repeated recovery, retirement/prune, shared dependencies, independent holders, staging generations, consumed-handoff receipts and GC. Queue and catalog effects remain separate actions. A queue with unapplied acceptance cannot expose its cached state; recovery restores the recorded receipt. Retiring one request cannot drop another active request's closure, including requests in another queue.

Verus proves preservation of the combined invariant by every enabled transition, induction over arbitrary finite histories, immutable accepted receipts, and protection of every active request's dependency. Cached retries work after retirement and physical GC without requiring old payloads; conflicting digests are rejected. Replaying an already consumed handoff leaves the entire state unchanged, including a later staging generation. Tombstones remain after durable deletion receipts. A concrete proved history witnesses two queue-scoped equal IDs, lost apply, recovery, restaging, repeated cleanup and one queue's retirement. This guards against vacuous specifications. `retained_closure_is_complete` also requires successful retain to include every dependency, rather than merely blocking later acceptance forever.

The existing actual `catalog_state::eligible` predicate is connected conditionally: `active_request_excludes_actual_candidate` rules out a live dependency **when the caller establishes that the actual locked catalog roots correspond to the model's protected objects**. That correspondence is not assumed proved. Complete publication recipes, owner-namespace separation, native durable-effect/lock/File refinement and accurate snapshots remain straw obligations, listed in `ENV-CROSS-COMPOSITION`. [The separate environment](../verification/protocol/tla/EnvironmentCrossComposition.tla) allows restoration of unsynced deletions but preserves existing durable names. This abstraction does not expand WAL fragments; `DurableHandoff` does that separately for one handoff, and its refinement into the new composition has not been proved. Retirement is monotone in this model; rewind/reactivation, stale lease/epoch admission, training checkpoint/cursor matching, writer restart and GC liveness remain separate unfinished work. TLA+ and Verus check related finite and unbounded models; no mechanical TLA-to-Verus refinement is claimed.

The [composition record](../verification/measurements/20261025-cross-log-composition.json) binds 378 complete positive Verus obligations and fourteen semantic negatives, plus three finite configurations (9,944 / 64,268 / 492 states) and thirteen specified invariant counterexamples. Seventy installed-wheel integration cases pass, including a new combined shared-dependency/restaging/lost-reply/two-queue/repeated-recovery case. All production and build inputs match the previously tested wheel; no production execution body changed, and no fresh full Python/native or performance run is claimed. This closes the model-level multi-request/queue composition obligation, not the complete distributed implementation proof.

## Queue event encoding and recovered transactions

Production `Journal::append` now calls [journal_append](../rust/journal_append.rs), which validates the actual event list, serializes that same Value and supplies those bytes to the verified `journal_state::State::append_to`. Success binds the input to one complete durable queue frame, the returned prior sequence and the unchanged preceding byte prefix. Unavailable, invalid-event and serialization errors leave state and the writer unchanged; uncertain write errors preserve the confirmed boundary and keep quarantine armed. The native wrapper retains closed-file and error-class mapping. This extraction keeps the existing serialization and I/O operations.

[journal_equivalence](../verification/wal/verus/journal_equivalence.rs) composes the accepted-frame predicate with the actual frame decoder contract: a successful read of that frame yields its exact payload and boundary, and successful Value parsing preserves the original finite-depth observations. `encoded_history_observations` and `repeated_recovery_observations` establish the corresponding result for arbitrary finite histories with the same raw sequence. These are observations of transactions, not a proof of the complete Coordinator business fold. Numbers expose only `as_u64`; parser acceptance, native descriptor/result mapping, run-identity comparison and physical retained-prefix selection remain separate. No new trusted law was introduced.

The [queue WAL record](../verification/measurements/20261025-journal-append.json) binds 362 complete positive obligations, twelve new and twenty-one affected semantic negatives, 73 native tests and 909 installed-wheel Python tests. One initially invalid negative is recorded separately and replaced by a semantically rejected weakening; both positive runs use identical source inputs. Both Clippy configurations and formatting pass; no performance/TLC campaign was added.

Production `queue_history::apply` now calls [queue_event::prepare](../rust/queue_event.rs) before changing scheduling indexes. Verus checks exact ordered target selection for Submitted, Leased, Epoch, Expired, Failed, Cancelled, Yielded, TaskProgress and Completed: no IDs are omitted, reordered or deduplicated; invalid affected fields are rejected, and well-shaped lists cannot be rejected arbitrarily. Events without task targets return an empty list; this function does not validate their remaining business fields or authorize unknown event types.

[queue_event_equivalence](../verification/protocol/verus/queue_event_equivalence.rs) connects successful independent JSON interpretations to identical target lists. `recovered_frame_targets` and `recovered_history_targets` compose that result with actual queue encoding and WAL replay, preserving every transaction/event position. These are target-selection theorems; complete task dispatch, cache maintenance, stale attempt/epoch admission and full business replay remain open; individual task-record bodies are covered below. The [queue event record](../verification/measurements/20261025-queue-event.json) records actual-body and semantic-control results separately from native regressions.

The only added trusted library primitive is borrowed `str` indexing, the same lookup beneath serde_json Value indexing: absent keys and nonobjects return None, without allocating a lookup key or scanning the object. Its actual pinned implementation calls Map::get; that stage audited this exact method among eighteen primitives. The output still contains one ordered ID vector and copied IDs; it now reserves the known list length. This is not a measured zero-cost claim.

## Actual task-record replay updates

Production `queue_history::apply` calls [queue_task](../rust/queue_task.rs) for task creation, leasing, retry, terminal transitions, failure evidence and continuation updates. The actual function bodies prove exact changed fields and preservation of every unrelated field. Retry increments the failure count with checked arithmetic, clears the lease and selects pending/failed using the configured limit. Nonretryable failure preserves its existing lease fields. Progress updates input and any supplied priority, scheduling key or metadata; yielding additionally clears the lease, returns to pending and records the supplied queue order. Invalid task/spec shapes and exhausted counters return errors without changing that task record. This last property is local: surrounding reader/cache/epoch changes are not claimed atomic.

A native recovery regression found that checksum-valid WAL events targeting a never-submitted task could manufacture a partial task and continue recovery. The native caller now checks every affected target before business updates; nine malformed-history fixtures reproduce acceptance in the old wheel and require `UnsafeRecovery` with no new recovery epoch in the fixed wheel. This is a malformed-business-history defect, not evidence that a valid API sequence emitted those events. Counter exhaustion is defensive checked handling, not a separately demonstrated reachable API defect.

The actual [queue_state](../rust/queue_state.rs) now binds these leaf transitions to the selected task inside the queue's `tasks` table. Its dispatcher forwards the exact lease/event, terminal state, yield flag and pending order. `edit` proves the complete nested selection and frame: missing table/target is rejected unchanged, leaf errors leave the whole passed queue state unchanged, and success changes only that task while preserving every other root field. `submit` installs the exact new record under the spec's task ID; duplicate-submission admission remains an outer obligation; the ordered allocation kernel is now covered by queue_admin below. The obsolete native identity helper was removed.

Production epoch revocation calls the verified `edit_many` loop. Its trace contains every supplied ID in order, including repetitions. Success equals the whole trace; failure identifies the exact completed prefix and the first failing identity/target/transition, with no later task applied. `history_frame` proves that any such successful finite batch preserves all non-task root fields (including readers/checkpoints) and every unselected task. This local theorem concerns uniform-change task batches. queue_history above now supplies full event-kind/argument selection and the mixed-event fold; cache semantics and native recovery/quarantine conformance remain open. queue_admin below supplies epoch assignment and bounded order allocation. The [queue state record](../verification/measurements/20261025-queue-state.json) records semantic controls and native/wheel regressions. No library primitive, whole-operation assumption or filesystem operation was added; performance has not been remeasured.

The actual [queue_transition](../rust/queue_transition.rs) now composes task, reader and receipt effects for Leased, Failed/Expired, Cancelled, TaskProgress/Yielded and Completed. Its relations include every completed effect before a later error: lease installation precedes reader registration; a reported failure releases the current reader before retrying the task; expiration and cancellation retain readers; progress extends reader roots before changing the task, while yield releases its reader first. Completion releases the current reader, marks the task, installs the submission receipt and then appends to commits. An error in that last step preserves the preceding effects, rather than claiming whole-event rollback. Returned task identities are borrowed from the validated event. The production Coordinator calls these bodies; no new I/O or persisted format is introduced. Completion now clears its derived deadline only after receipt application succeeds; a receipt failure leaves the already-quarantined cache unreconciled, so native quarantine conformance remains a separate obligation.

[queue_transition_roots](../verification/protocol/verus/queue_transition_roots.rs) derives root preservation for an existing valid reader from the actual successful progress contract: every old root remains at its previous position, a non-null new input is protected, and peer reader entries are unchanged. The derivation explicitly uses `ENV-STRING-KEY`'s existing `string_content_contract` to connect the cloned lease's attempt string to the same logical map key. This is an additional stated premise of that theorem, not a proved library identity law. This root theorem does not establish admission/freshness, scheduling-cache consistency, root extraction or catalog ownership. The subsequent queue_admin, queue_replay and queue_history proofs now supply order allocation, reader retirement and the mixed-event business fold; complete native recovery and GC remain open. The [compound replay record](../verification/measurements/20261026-queue-transition.json) separates actual-body proofs, root derivations, semantic controls and regression evidence; no new valid-API bug or performance result is claimed.

The actual [queue_admin](../rust/queue_admin.rs) adds the remaining submission/order, epoch, producer, ReaderDone and request-table bindings. Submission replay assigns consecutive orders to every supplied task in order, including repeated IDs in malformed histories, and reports the exact completed prefix on error. Producer continuation changes only after the task batch succeeds; a producer-table error leaves that batch and its counter advances applied. Epoch assignment precedes ordered revocation, including the case where the revocation array itself is invalid. ReaderDone and request receipts are bound to the correct queue root while other roots remain unchanged. The request relation uses the existing receipt ledger/content abstraction, not exact JSON allocation identity.

Scheduling orders now use checked increments. Exhaustion rejects the current submit/yield before modifying its task or reader; earlier batch members remain applied. Ordinary progress and empty submission batches need no new order. `has_capacity` checks the complete requested count, and `capacity_rules_out_order_error` proves that a batch fitting that bound cannot fail from order exhaustion. Native submit/yield admission checks capacity after cached-response lookup and before retain/WAL writes. A forced boundary regression checks unchanged state/log sequence, no quarantine and cached retry success; two isolated native mutations removing the guards reach the forbidden write callback and fail the test. This is defensive boundary coverage, not evidence that a practical valid workload exhausted the counter. The [administrative replay record](../verification/measurements/20261026-queue-admin.json) records the exact proof and native mutation inputs. The dispatcher and ReaderRetired stage above now covers event-kind/parameter selection and sealed state; full native admission, mixed history, scheduling indexes and ownership remain open. No new trusted library method, I/O operation or persistent format is introduced; the arithmetic checks execute at runtime and have not been benchmarked.

`ENV-JSON-LIBRARY` explicitly trusts the additional borrowed `str::index_into_mut` primitive: only the selected object field can change; a missing field leaves the value unchanged. The pinned serde implementation delegates to Map::get_mut. At that stage the audited surface had nineteen library primitives, with no admitted straw body. The [task replay record](../verification/measurements/20261025-queue-task.json) separates actual-body proofs, semantic controls and native regressions. Full event dispatch, reader/submission roots, scheduling caches, pending-order allocation, stale lease/epoch admission and complete Coordinator history refinement remain open. These field-update proofs do not establish automatic training checkpoint selection or full cross-log/GC safety. The change adds no filesystem operation; runtime performance has not been remeasured.

## Reader protection, completion receipts and producer continuation

Production replay now calls [queue_reader](../rust/queue_reader.rs) to register a lease/input, extend its read roots, remove one attempt and execute an ordered `ReaderDone` list. Extension preserves every previous root and the existing lease, includes the new input and deduplicates with native Value equality. Other attempt entries remain unchanged. The actual release loop admits repeated IDs and identifies the exact applied prefix when a later lease lacks its attempt ID. Invalid record shapes return a defined local error. Registration replaces its selected attempt, matching existing behavior; fresh attempt identity remains an admission obligation. Cancellation, expiration and epoch recovery continue to retain readers. The worker-selection predicate and the enclosing `ReaderRetired` filter are now proved under the retain boundary described above.

[queue_receipt](../rust/queue_receipt.rs) proves the actual submission/commit sequence and producer continuation update. Completion stores the matching receipt and lease before appending the receipt to the ordered commit list, preserving all prior commits and other fields. If the commit list is malformed, the exact submission prefix is described; no atomic rollback is claimed. Missing submission identity is rejected before these edits. Producer updates change only their selected producer entry. Whole-event task/cache/reader updates, request admission, quarantine and full historical replay composition remain separate caller obligations.

`ENV-JSON-LIBRARY` adds only native `Value::eq`, bringing that stage’s audited method surface to twenty; the retain stage above adds the twenty-first. True equality preserves all finite-depth observations; it does not imply identical serialized bytes (negative and positive floating zero can compare equal). Numbers still expose only `as_u64`. The runner rejects `preserve_order`, retaining the sorted-map configuration required by ordered observations, and checks `float_roundtrip`. No straw body or whole reader operation is trusted. The [reader/receipt record](../verification/measurements/20261025-queue-reader.json) binds actual-body proofs, semantic negatives and installed-wheel regressions. Six new workflows use independent old/new continuation inputs and a peer reader, covering cancel/expire/recover followed by explicit read completion or worker retirement; only acknowledged roots are reclaimed. These tests complement the local proofs; complete root extraction, fencing and cross-log/GC refinement remain open. No filesystem operation or performance campaign was added.

## Queue consumer, batch, checkpoint and retention replay

The six metadata branches of production `queue_history::apply` call [queue_metadata](../rust/queue_metadata.rs). Verus checks their actual event decoding and ordered updates, including the optional matching request receipt as the last edit. The specification requires acceptance of recognized events with correctly typed identifiers; it cannot pass by rejecting every input. [json_update](../rust/json_update.rs) proves exact nested-leaf insertion/removal, preservation of unrelated fields and unchanged state on an invalid path. Missing intermediate objects are never created implicitly.

A multi-edit event is not claimed to be atomic in memory. On failure, `outcome` identifies the exact successful prefix and the invalid next path; the receipt is reached only after all business edits. Native commit quarantine and constructor failure must prevent reuse of partial state. Complete caller conformance, the other task/lease/epoch/reader events, derived scheduler caches, raw queue serialization equivalence and cross-log ownership/GC composition remain open. These six branches do not establish automatic model-checkpoint recovery or protection of all training replay data.

The current external library surface has twenty-one primitive specifications, including the borrowed str read/mutation indexing described above. The new Map `get_mut` and `remove` frame laws, plus Clone preservation of finite-depth observations, are explicitly trusted in `ENV-JSON-LIBRARY`; no straw implementation body is admitted. Numeric observations still expose only `as_u64`; full numeric round-trip behavior has separate runtime regressions.

The recovery fixture uses correctly framed but malformed business events: `BatchReady` without an object-valued planned batch. The prior wheel accepts absent/null batches or panics on a boolean; the new code returns `UnsafeRecovery` without appending a recovery epoch. This is malformed-history hardening, not a demonstrated history reachable through valid API commits. The extraction builds small edit/path vectors and therefore changes executable allocation work; no zero-overhead or throughput claim is made. Performance work is deferred until the outstanding correctness obligations are complete.

The [queue metadata record](../verification/measurements/20261025-queue-metadata.json) binds the complete positive proof (357 obligations), 17 new and 13 affected semantic negatives, 73 native tests and 909 installed-wheel Python tests. Both Clippy configurations, formatting and the environment inventory pass. No new TLC or performance campaign is included.

## Catalog append and replay business equivalence

[catalog_json_equivalence](../verification/wal/verus/catalog_json_equivalence.rs) proves that allocation-independent JSON observations preserve every consumed wire field. [catalog_equivalence](../verification/wal/verus/catalog_equivalence.rs) connects those fields to the actual `catalog_state::run` and `LogState` history: staging updates may appear in different HashMap iteration orders, but update distinct keys and yield the same map. Owner/pack/mark/reclaim/receipt actions retain their required order.

`raw_history_state_equal` proves equal six-index states for any finite pair of interpreted histories with the same raw payload sequence and fixed run/namespace. `completed_states_equal` applies that induction to actual completed log states, also establishing equal cursor and sequence. It relates the histories produced by the proved append and recovery bodies; it does not assume identical parsed allocations or identical operation ordering. It requires a completed state: partial replay remains quarantined by the existing log invariant.

Two external premises are explicit: `ENV-JSON-VALUE-CONSISTENCY` supplies deterministic finite-depth Value observations on successful parsing, and `ENV-STRING-KEY` supplies identity of opaque logical String keys with equal character content. The former exposes only `as_u64` for numbers, not a complete floating-point model. Neither premise asserts parser acceptance or deterministic custom Deserialize behavior. No new whole straw adapter is trusted. Native namespace/File/locks, the physical choice of surviving prefix, complete Coordinator replay and full GC composition remain open.

The native `append_and_fresh_replay_agree_on_every_business_index` regression compares all six indexes and confirmed boundaries after each of four frames, including a shared staging graph, Unicode identities, duplicate set members, release receipts and ordered mark/reclaim. This change adds erased proofs and a test; production execution bodies are unchanged. That checkpoint passes 348 obligations and fourteen selected semantic negatives; all 70 native tests, both Clippy configurations and formatting pass. No new wheel/Python, TLC or performance run was needed. Exact inputs and results are in the [equivalence record](../verification/measurements/20261025-catalog-equivalence.json).

## Catalog append input and serialization

Production `Catalog::append_frame` now calls [catalog_append](../rust/catalog_append.rs). Verus checks the same executable `prepare` and `append` bodies: bind the supplied run without changing other input fields, decode those fields, serialize that exact value, pass its bytes and operations to the shared writer, and preserve the interpreted history after success or failure. `LogState::new` establishes the empty history base. Successful confirmation extends history by one frame and agrees with the byte cursor and durable boundary; decode errors retain their input-relative justification and cannot become writes.

`ENV-JSON-ENCODE` is a separately audited **external-library** contract. A generic serializer relation is specialized to `serde_json::Value` by one named library law; it does not presume reversible custom serde implementations. The runner checks twenty-one library method specifications, four opaque types, the transparent Value enum two named Value library laws (encoding and observation consistency), and one Map key-uniqueness law. It also checks that the compiled serde_json artifact actually enables `float_roundtrip`. Parser/serializer internals, acceptance/depth/resource limits and compiler correctness remain outside the proof. The conditional catalog business-equivalence theorem above now connects independently allocated parsed values to identical completed indexes. Native namespace/set/File/lock adapters, pre-append path normalization, full coordinator replay and GC composition remain open.

Reviewing this boundary exposed an actual numerical bug: the previous parser changed some finite floats when reading serialized JSON. Real regressions reproduce a run-identity rejection with an unchanged lease-duration configuration, changed producer continuation after recovery, and changed canonical event bytes in native journal replay. Enabling precise floating-point parsing fixes these cases, including repeated recovery, same-ID request replay, signed zero and numeric boundary values. Already rounded values in old data cannot be reconstructed; an old request hash based on a rounded value can conflict with the original unrounded input after upgrade. No performance conclusion is inferred from this parser correction.


The safety/progress premise audit checks actual `execute_commit`, `wal_recovery::recover` and `gc_batch::collect`, plus `catalog_history_safe` and `gc_liveness::history_valid`. Their safety contracts require current-state/effect invariants, with no eventual-stability or scheduling premise. The separate `eventually_finishes_from_environment` and `eventually_reclaims` theorems explicitly require stability and effect/caller fairness. This separates existing conditional theorems; it does not establish native caller histories or complete system progress.


The [append/JSON record](../verification/measurements/20261025-catalog-append.json) binds 323 complete positive Verus obligations, twelve new append/library semantic negatives and thirteen affected receipt controls, 69 native tests and 906 installed-wheel Python tests. Both Clippy configurations and formatting pass. No performance or new TLC campaign is included.


## Request replay after retirement

The actual `request::{lookup,response,copy_field,remember}` bodies used by `Coordinator` are Verus checked. Lookup distinguishes absence from a cached null response, rejects conflicting digests and returns the stored response content; replay updates exactly one receipt key. The library contract preserves canonical JSON content and digest text across cloning, without equating allocation/layout. `request_history` proves idempotence, unrelated-key preservation, admitted-history receipt preservation and chunked replay equivalence for arbitrary finite histories.

`RequestReplay` separately checks two request IDs, conflicting content, lost replies, repeated recovery and removal of accepted data. Its lower append/sync/apply is atomic; it does not prove that native coordinator calls establish admission, key/hash identity, authorization, ordered business replay or cross-log ownership invariants. Those remain implementation obligations. `ENV-REQUEST-REPLAY` makes explicit that legitimate GC can remove old inputs while receipts survive.

Regression tests reproduce eight old-version failures across consumer-state save, batch plan, batch ready and checkpoint registration, both live and reopened. The fixed methods consult receipts before dereferencing retired data, preserve content conflicts and leave state, WAL, catalog and physical deletion unchanged on replay. A repeated batch-plan request retains its existing API behavior of returning the current batch state. These tests do not establish the full coordinator proof.


The [request replay record](../verification/measurements/20261025-request-replay.json) binds 319 full positive Verus obligations, 14 semantic negatives, five TLC cases, 68 native tests and 904 installed-wheel Python tests. Both Clippy configurations pass. Older full negative/model campaigns were not rerun; no new performance claim is made.


## Core verification: distributed protocol, WAL and GC

The core review covers these three components and their boundaries. The table
states the target properties. The finite composition model, unbounded retry
proof and production Rust checks are recorded separately below. Complete
adapter refinement remains unproved; combining the results does not establish
verification of every implementation path.

| Core | Model / proof to add | Required properties |
|---|---|---|
| Distributed commit and recovery | Compose payload durability, catalog retain, queue WAL acceptance, staging consumption and reply; track current and durable state for each log independently, with crashes, lost replies and retries at each boundary | Every accepted task retains its full dependency closure through durable ownership; retries of one request ID apply once, conflicting contents are rejected, and stale attempts / epochs cannot modify current state |
| WAL and replay | Separate header, body, trailer, sync, valid torn tails, complete corrupt frames, recovery truncation and later appends; failed sync may already have effects | Confirmed prefixes survive; complete corruption is rejected; only a valid incomplete tail is truncated; append position and sequence match the recovered prefix; recovered state must be synced before authorizing operations that depend on it |
| Cross-client GC | Compose queue, reader and checkpoint ownership changes with durable tombstones, unlink, directory sync, receipts and restart; split catalog operations into their persistence stages | Live ownership protects the full closure; tombstones fence late retain; errors do not imply rollback; receipts imply durable deletion; cancellation / lease expiry cannot release a still-active reader |
| Conditional progress | Connect eventual environment recovery, scheduler fairness and actual retry transitions separately for commit, recovery and GC | Under stated conditions, operations finish or return their specified errors; eventual next-stage success cannot itself serve as the environmental progress proof |

The WAL prefix model supplies each log's recovery boundary; `DurableHandoff`
splits cross-log operations into persistence stages. Separate generic-controller
and GC Verus proofs handle retries after failure. Their environment contracts
remain explicit, as does the unproved mapping from I/O, decoding and recovery
adapters into the models.

Each result must state what the environment permits, what straw must implement,
what the finite model checked, which production Rust functions Verus proved,
and which I/O, locking and encoding adapters remain unproved. Negative controls
must hit the intended safety or liveness counterexample. Implementation
regressions must exercise the corresponding failure windows and operations
after recovery. Passing L1 / L2 models without adapter refinement does not
establish verification of the entire distributed system.

The first production step shares the append controller and frame-boundary
decision in `rust/wal.rs` between the queue and catalog, and verifies those same
function bodies. The controller proof does not establish that callers write
the specified bytes, fsync the correct descriptor, or faithfully decode JSON
and checksums. Composition models and fault regressions are still required;
complete adapter refinement remains outstanding.

This first step passed 128 positive Verus checks, eight new WAL semantic
negatives and the existing WAL-before-retain negative, plus 40 native tests and
111 focused Python WAL / commit / GC regressions. The new regression enumerates
every byte prefix of one frame, appends after recovery, then reopens and checks
the transactions. It models file prefixes, not device power loss. The first check did not include the remaining
Verus negatives or full Python suite; the later complete rerun is recorded next.
The later finite cross-log model is described below; full implementation refinement remains unproved.

The previous feature-directory migration passed fresh checks of 846 Python tests,
40 native tests, builds of all seven Rust examples, 383 TLC cases (157 positive,
197 algorithm negatives and 29 environment negatives, including 20 genuine
liveness counterexamples), and the full Verus campaign (128 positive checks and
113 semantic negatives). The 30 `--area wal` cases also pass independently.
That inventory contained 43 contracts, 43 consumers and 19 component layers; the current inventory has 65 contracts, 49 consumers and 31 layers.
See the [verification index](../verification/README.md) for the layout and commands.
These checks cover their actual inputs; missing cross-log refinement and complete
environmental progress derivations remain separate work.

Here, the distributed protocol is the transfer of data and ownership among
clients through shared storage. Catalog lock serialization and stopping / fencing
the old coordinator before takeover remain independently reviewable environment
contracts. This is not a claim that straw implements consensus or automatic
leader election. Counterexamples should expose the loss of those contracts
instead of hiding external guarantees inside an atomic operation.

Safety must not depend on faults eventually stopping. Progress needs that
assumption and caller retry obligations separately. A timeout does not make
automatic reclamation of a dead writer safe without external fencing. JuiceFS
internals, the lock service and power-loss durability remain external contracts;
p99 and 100 GB throughput measurements are deployment evidence, not correctness
or deadline theorems.

## Actual GC batch deletion dispatch

`Catalog::collect_with_stat` calls production [gc_batch::collect](../rust/gc_batch.rs), whose actual body is checked by Verus. Each target index has a fixed parent directory. The loop checks marks, unlinks every target, collects parents including those of unlink NotFound responses, completes every distinct parent barrier, and only then records receipts. For arbitrary finite batches, success means every target is durably absent and receipted. Errors preserve the invariant that receipts cover only durable absence, including errors with unlink, sync or receipt side effects.

This composes the real loops beyond individual controller stages: shared parents do not omit targets, and skipping a barrier cannot authorize an early receipt. A completely marked batch cannot spuriously return an internal protocol error. Accounting uses proved checked addition; overflow returns an error with retry state preserved. Exact accounting refinement and an observed deployment overflow are not claimed.

The native adapter now uses [gc_io](../rust/gc_io.rs) for actual error classification, unlink and parent-sync dispatch. Only NotFound becomes absence; stat is advisory and never suppresses unlink. Existing parents are synchronized, while missing parents require successful repair. [directory_io::sync_all](../rust/directory_io.rs), called by production mkdir, proves complete ordered ancestor barriers and the exact attempted prefix on failure, preserving earlier durable effects. Both use the existing fault points and filesystem operations; no extra allocation or syscall is introduced by this extraction. This is not a measured performance guarantee.

`ENV-GC-PRIMITIVES` and `ENV-DIRECTORY` expose the narrower contracts below `ENV-GC-BATCH-EFFECTS`. Native target/mark/path/handle binding, complete canonical ancestor enumeration and the aggregate repair-to-durable-absence relation remain open, along with complete roots, native catalog context and retry progress. Successful hooks must preserve modeled state; arbitrary user callback mutation is outside this contract. The [GC I/O record](../verification/measurements/20261025-gc-io.json) distinguishes actual-body proofs, semantic controls and native regressions. It does not claim complete GC refinement or crashed-writer fencing.

The [batch deletion record](../verification/measurements/20261025-gc-batch.json) binds 280 full positive Verus obligations and 19 new semantic negatives. Controls omit or misidentify targets and parents, skip NotFound barriers, skip synchronization or receipts, write receipts early, swallow errors, spuriously reject, use unchecked arithmetic, or weaken primitive response/durability contracts. Real multi-writer regressions cover partial unlinks, shared parents, missing-parent repair and lost receipt responses. Existing finite GC models remain separate evidence; this checkpoint does not claim a fresh TLC campaign. The full installed-wheel suite passes 896 tests, alongside 63 native tests, both Clippy configurations, examples and protocol/WAL campaigns. The [collection-cost comparison](FILESYSTEM.md#gc-batch-dispatch-comparison-2026-10-25) retains the observed slowdown.

## Actual commit execution and interrupted application

`Coordinator::commit` now calls [queue_commit::persist_and_apply](../rust/queue_commit.rs), which directly borrows the owned Journal, business state, epoch, ordering, seal state, pending handoffs and derived caches. It calls the checked Journal append, records the handoff only after durable acceptance, arms the actual Journal state, applies the complete business transaction and consumes the private confirmation before clearing quarantine. Its contract preserves the same File permission and exposes the exact accepted WAL bytes and partial business result on every returned error. The older `protocol::execute_commit`/`ENV-COMMIT-EFFECTS` proof is a separate abstract dispatcher, compiled only for verification; the native `CommitIo` implementation has been removed. Catalog retention evidence and final staging-consumption composition remain separate proof obligations; runtime regression tests cover these paths.

[queue_handoff](../rust/queue_handoff.rs) supplies the production handoff creation and recovery paths. The actual parser requires a unique final marker, binds its identity to queue namespace, sequence and all preceding business events, traverses those events for the complete reference set and appends only that recovered intent to pending cleanup. Legacy frames produce no cleanup intent. Public catalog APIs reject the reserved `queue:` and `publication:` namespaces; scoped internal queue retention and typed GC remain available. This admission guard prevents public replacement or release of internal roots, but does not by itself prove historical owner-to-reference correspondence. Canonical JSON identity uses only the independently reviewed Value serialization roundtrip and constructor contracts; neither Value equality nor shallow schema observations imply equal hash input.

A concrete Rust defect was reproduced: after durable acceptance, a panic before memory application could be caught by the caller while `journal.poisoned` remained false. A subsequent GC call consumed staging, saw no live task in memory, pruned its queue owner and deleted the accepted input. The dispatcher now arms quarantine before application or callbacks and clears it only after successful application. The native regression checks both callback cutpoints, refusal to continue, replay of the original request and data protection after recovery. The preceding exact source reproduces one reclaimed accepted file; the fixed source rejects GC with `UnsafeRecovery`.

`CommitApplication` separately models returned errors, caught unwinds, the later API admission barrier, recovery, staging cleanup and retirement-before-prune. Six algorithm mutations produce `AcceptedDataPresent` counterexamples. Verus proves normal Result executions and requires apply to preserve the guard; it does not itself prove Rust unwinding. The model and native unwind regression provide distinct evidence for that boundary.

The [commit/recovery checkpoint](../verification/measurements/20261025-commit-recovery.json) records 272 positive Verus obligations, 14 targeted semantic negatives, 16 application-interruption model cases, 63 native tests and 888 installed-wheel Python tests. Rust/Python formatting, both default/Python Clippy and examples/protocol/WAL campaigns pass. Older complete negative/TLC campaigns were not rerun.

## Actual catalog recovery and durable confirmation

`Catalog::refresh_with_sync` now calls the production `catalog_target::Apply::recover`: check the confirmed floor, reset uncertain or partially applied index history when rebuilding, run shared byte recovery, then confirm its actual boundary. Verus checks this call composition. Success aligns typed-history indexes, frame-count sequence, cursor, truncated file length and durable floor, with no unfinished apply/append. Errors preserve the previous confirmation floor. Empty incremental refresh retains its existing cache behavior.

The Target interface separately specifies complete application on success and preservation of application context (the catalog floor). The actual catalog Target proves both; the shared byte driver carries them into the caller. Auxiliary theorems connect arbitrary payload histories, frame lengths and replay suffix boundaries. The confirmation call also proves that it succeeds, rejecting wrong arguments that could otherwise introduce a spurious error without violating success-only postconditions.

This implementation proof depends on `ENV-REPLAY-IO`. Native Source descriptor/buffer/result correspondence and namespace/cache conformance still need refinement. The later raw-history section composes the actual decoder and run check under explicit parser/namespace/set contracts. It does not complete outer locking/fresh-open, Journal business recovery or the full cross-log/GC composition, and does not promise successful recovery of every business input.

The [recovery composition record](../verification/measurements/20261025-catalog-recovery.json) binds 276 positive Verus obligations, 14 new and 57 affected existing semantic negatives, 63 native tests and 888 installed-wheel tests. Both Clippy configurations, formatting, examples and protocol/WAL campaigns pass. This is not a fresh TLC or full remaining-negative campaign. The 32-trial recovery-cost comparison is in [FILESYSTEM](FILESYSTEM.md). Semantic controls skip rebuild, use wrong cursor/sequence/log kind, ignore the confirmed floor, recover read-only, skip/misbind confirmation, confirm after a failed barrier, or weaken completion/context/durability contracts. Controls verify the actual caller or shared driver as appropriate; resource exhaustion is not accepted as a semantic failure.

## Sampling and training resume boundaries

`tests/test_resume_workflow.py` starts a separate sampling process and exits it without close/destructors. A replacement recovers the last committed `save_task_progress`/`yield_task` reference; publishing a continuation alone does not change the task. Stale attempts are rejected, accepted completion remains idempotent, and interrupted active leases use the existing retry budget. Interpreting continuation tokens/RNG and choosing a committed external model checkpoint are application responsibilities.

The training regression uses online GC and results created *after* an immutable checkpoint. A later processing cursor can outpace model checkpoint state. Without a separate replay owner, GC reclaims processed results and restoring the older consumer position is rejected. With that owner, restoring the checkpoint's matching consumer state re-establishes queue ownership before the extra owner is released; replay matches the uninterrupted mock optimizer. This is evidence for the supported explicit protocol, not an automatic latest-checkpoint feature or a proof of a real trainer.

`TrainingResume` checks the application protocol for two, three and four positions, with counterexamples for using the latest queue cursor, omitting post-checkpoint protection and committing mismatched model/data positions. `ENV-TRAINING-CHECKPOINT` states who must implement that protocol. The model uses ordered positions and one input per optimizer step; distributed optimizer/RNG state, sparse batches and external manifest publication remain outside it.

## Actual WAL byte framing: partial implementation proof

[`rust/framing.rs`](../rust/framing.rs) now supplies both queue and catalog
encoding and replay checks. Verus checks those same bodies: little-endian u64
round trips, the 24-byte prefix and 56/40-byte header/trailer layouts, magic,
sequence and declared payload length, digest comparisons, and the permitted
partial-header magic prefix. `encoded_header_is_accepted` composes encoder
postconditions with the decoder predicate. The proof rejects ten semantic
mutations that remove a check or corrupt field encoding/decoding.

This is a byte-framing proof, **not complete WAL recovery**. Callers still must
establish that Device supplies actual bytes and targets the correct log, and
map decoded JSON into business state. SHA/serde primitives remain trusted;
the following section records progress on the shared byte driver and its
remaining adapters. Native tests
check compatibility with the existing wire format and corrupt every header and
trailer byte; those executions supplement the proof.


## Catalog JSON wire decoding

The catalog is the shared ownership ledger persisted in `storage.log`. Queue WAL records task state; the catalog records which queue, reader, checkpoint or publication still protects each data pack, plus sealing, deletion and handoff receipts. Its state is therefore consumed by both cross-log commit and GC. Proving it alone does not prove either complete protocol.

The actual [catalog_json.rs](../rust/catalog_json.rs) now decodes the fields consumed by catalog replay. Verus proves that decoding succeeds exactly for its wire schema and that every returned string, map key, list position, Boolean and field-presence flag corresponds to the input. Missing optional fields produce empty updates; present fields with the wrong type are rejected. This fixed replay silently treating malformed `owners`, `packs` and `deleted` fields as empty updates. Unknown fields remain ignored for compatibility; semantic validation of paths, supported encoding versions, graph identities and receipts remains separate.

[`ENV-JSON-LIBRARY`](../verification/common/verus/serde_json.rs) trusts eleven primitive serde_json access/clone/map specifications, one successful-deserialization relation, one serialization relation, four opaque library types, the transparent actual Value enum and two Value-only encoding/observation laws. The runner builds the actual locked serde_json with Rust 1.99.1, records library/input hashes and audits this separate allowlist. All straw decoding bodies remain verified with no omitted bodies or assumed business result. The later raw-history section uses the separately trusted successful parsing relation to connect bytes to `Value`; native path resolution remains open. Conditional operation construction and raw Target/history refinement are described below; full business replay is not yet proved.

The actual [catalog_event.rs](../rust/catalog_event.rs) now checks the next semantic-validation stage: a handoff ID is exactly 64 lowercase hexadecimal UTF-8 bytes; a staged key has the exact `staged:` prefix and such an ID. A present receipt field requires an owners field containing only releases. Every added graph node and child must have a valid staged name, and the update must satisfy the proved graph-compatibility predicate. Each executable predicate proves both acceptance and rejection against these definitions. Missing receipt fields retain the legacy behavior. These bodies are called by native replay/append and handoff processing; that checkpoint did not cover path resolution or operation construction. Their subsequent conditional composition is described below. The [catalog semantic-check evidence](../verification/measurements/20261025-catalog-event.json) records this narrower scope separately.

This stage passes 252 positive Verus obligations, twelve targeted semantic negatives, 60 native tests and formatting, plus 202 installed-wheel WAL/receipt/graph/catalog/handoff/online-GC tests. The preceding full 883-test run belongs to the separate catalog Target checkpoint. Correction: the archived default/Python Clippy logs failed two style checks; the earlier summary incorrectly reported them as passing. The original logs are preserved and the public checkpoint records the correction.

The [catalog JSON checkpoint](../verification/measurements/20261025-catalog-json.json) passes 196 positive obligations, eight algorithm and two library-environment semantic negatives, 52 native tests and 883 installed-wheel Python tests. All 15 new malformed-field cases reproduce on the preceding exact wheel. This is targeted JSON evidence, not a fresh full Verus negative or TLC campaign.

## Catalog fields to ordered operations

The actual [catalog_decode.rs](../rust/catalog_decode.rs) composes wire decoding, receipt/graph admission and operation construction used by native replay and append. Its success contract binds every owner path, release, sealing flag, tombstone, reclamation, dependency edge and handoff receipt to the decoded input. Owner aliases are canonicalized and deduplicated under a fixed namespace; marks precede reclamations and receipts follow all updates. The consuming graph iterator preserves every entry without imposing a hash iteration order. A failure returns no partially constructed operation sequence. Schema, receipt, graph and encoding failures have explicit conditions; path failures remain dependent on the resolver.

Three boundaries remain independently reviewable: `ENV-CATALOG-RESOLVER` requires truthful successful path identities without changing the abstract namespace; `ENV-SET-BUILDER` now checks the actual `NativeKeySet` adapter against one narrowly audited standard `BTreeSet::from_iter` law; `ENV-COLLECTION-ITERATOR` trusts four standard HashMap/HashSet iterator primitives and two opaque types. The runner audits those library specifications separately. The straw decoding/iteration bodies are verified; the complete decoder is not assumed correct. The later raw-history section now composes parsing/run checks and Target business meaning. Actual append serialization/history binding is described above; native path/cache conformance and the full locked transaction remain open.

The [operation-construction checkpoint](../verification/measurements/20261025-catalog-decode.json) records exact proof inputs and scoped negative controls. Production loops move decoded keys/values and retain bulk set collection; ghost histories and proof counters are erased. Runtime performance is measured separately and does not follow from these proofs.

The construction checkpoint passes 270 positive Verus obligations, 19 targeted semantic negatives, 62 native tests, 883 installed-wheel Python tests, examples and protocol/WAL campaigns, Rust/Python formatting and default/Python Clippy. Its [64-trial replay comparison](FILESYSTEM.md#catalog-operation-construction-comparison-2026-10-25) is separate performance evidence. Older negative cases and TLC were not rerun in this stage.

## Catalog admission before WAL effects

`Catalog::append_frame` now decodes and checks the entire operation sequence before marking uncertainty or issuing write/sync. After sync it applies those same prepared operations, without repeating JSON decoding or namespace lookups. Non-object input is rejected before writer-side JSON mutation. This closes an internal writer-boundary defect: an invalid event could be fully synchronized before apply rejected it, leaving a durable frame that future replay could not accept. The native regression calls that internal boundary directly; this is not evidence that a public API generated such events.

The actual `Indexes::validate` proves success exactly when ordered `apply` can accept the sequence; `apply` now proves the same biconditional and retains its exact failed-prefix contract. Validation has no mutable access to the indexes and stores only marks added earlier in this event. Existing tombstones and preceding marks authorize receipts; a later mark cannot authorize an earlier receipt. This is the existing typed-index admission predicate, not a new proof of complete logical ownership or GC durability.

`CatalogTransaction` now distinguishes valid events from permanently invalid input. Rejected input produces no WAL effects, leaves the transaction usable and reaches its error terminal under the model's fairness premise; skipping preflight violates that property. Its sixteen cases include the previous failure windows and guard/environment negatives. The concrete decoder-to-model binding and full lock/I/O call chain remain refinement obligations.

The [catalog admission checkpoint](../verification/measurements/20261025-catalog-admission.json) records 199 positive Verus obligations, six targeted semantic negatives, sixteen catalog transaction cases, 54 native tests and 883 installed-wheel Python checks. The model explicitly records invalid-input error outcomes; reporting success for rejected input has its own negative control.

## Actual WAL append and catalog confirmation

Both journals now call [wal_append.rs](../rust/wal_append.rs) for the actual header/payload/trailer/sync loop. Its Verus contract checks the descriptor's observed length and cursor against the expected append boundary, rejects sequence/frame overflow before writes, preserves the prior prefix on success and binds every written byte and digest span to the supplied payload. The successful appended frame satisfies the same `complete_frame` predicate used by the verified recovery driver. Only successful synchronization completes the loop; error replies may follow partial effects.

The actual `LogState::append_to` composes admission, that byte writer and the stored-plan application. Success establishes agreement between the indexes' typed history, cursor/sequence, confirmed floor and Writer's durable bytes. A write failure leaves the plan pending and the state unavailable. This closes the ordering/confirmation proof at the Writer interface, including early confirmation and swallowed-error negatives.

[`ENV-APPEND-IO`](../verification/environment_contracts.json) separately records the external byte/position/sync/SHA guarantees and straw's native Appender obligations. The concrete adapter borrows the original File and preserves the existing fault-hook locations; it still needs a conformance proof for primitive results and descriptor identity. JSON serialization and raw-payload-to-operation binding, locking and the complete Journal business state remain separate obligations. A native cursor/length check may fail closed on an inconsistent observation; it is not a bound on filesystem visibility latency.

The [WAL append checkpoint](../verification/measurements/20261025-wal-append.json) records 225 positive Verus obligations, fourteen targeted semantic negatives (eleven algorithm and three Writer environment cases), 60 native tests and 883 installed-wheel Python tests, with clippy, examples and protocol/WAL campaigns. The hash-span negative is checked against the actual separate encode body; resource-limited prototype runs are excluded. This is not a fresh full older-negative or TLC campaign.

## Joint catalog state and typed frame history

The production [catalog_log.rs](../rust/catalog_log.rs) now owns the six persistent indexes, cursor, sequence, confirmed floor, rebuild flag, partial-apply state and pending append operations. Catalog callers can read the indexes but cannot mutate them independently. Verus checks the same state transitions used by refresh and append, including their exact success conditions; an implementation that always rejects valid work does not satisfy these contracts.

Erased ghost records track arbitrary finite typed frame histories within the checked machine bounds, without allocating history in production. For every complete state, the indexes equal the ordered fold of that history, sequence equals its frame count and cursor equals its total framed byte length. A failed apply records the exact applied operation prefix, leaves cursor/sequence unchanged and prevents confirmation, further replay and append. Rebuild clears tentative history, indexes and pending plans while preserving the confirmed floor. A successful recovery confirmation must match both the actual replay cursor and sequence. Append stores its admitted operations and applies that plan exactly once.

The native replay Source now exclusively borrows the original mutable File. Buffer removal, truncation, synchronization and final positioning use that descriptor; the existing replacement-name, failed-barrier and rollback regressions exercise this path. Rust borrowing and these executions do not establish the complete Device View or successful-read snapshot semantics.

This section proves actual catalog state and typed history. The subsequent raw-history section adds conditional raw-payload/business correspondence; native physical durability remains separate. append_to now establishes append barrier/confirmation ordering under Writer; replay barrier/confirmation placement is now composed by Apply::recover. Actual Source/Appender dispatch is now proved under the lower file/buffer/hook contracts described above; native thin-wrapper and outer-context conformance remains open. Path conversion, closure caches, full locking, Journal business state and higher-level protocol/GC composition remain open. `ENV-REPLAY-APPLY` and `ENV-VERUS-CATALOG` retain their unproved-adapter status.

The [catalog-state checkpoint](../verification/measurements/20261025-catalog-log.json) records 220 positive Verus obligations, fourteen targeted semantic negatives, 58 native tests and 883 installed-wheel Python tests, plus default/Python clippy, examples and protocol/WAL campaigns. It is not a fresh full older-negative or TLC campaign.

## Catalog raw history and replay application

The actual [catalog_target.rs](../rust/catalog_target.rs) replaces the catalog replay closure. Its `View` is a projection of the actual `LogState` history, not a separately supplied record list. Records and pending append plans now bind the exact payload bytes in erased ghost fields; only their lengths were tracked previously. This adds no runtime payload copy or retained byte history.

Verus checks that a successful application extends that history by exactly the supplied bytes and preserves the joint log/index invariant. It calls the decoder with the current staging graph and applies the returned operations through the verified `replay_record`; decode errors, invalid operations and incomplete state remain errors. The shared recovery loop requires and preserves Target validity, including error returns. Existing partial-apply/rebuild and confirmed-floor properties remain in force.

The initial Target checkpoint proved raw-history preservation without decoder business meaning. The actual [catalog_frame.rs](../rust/catalog_frame.rs) now replaces `EventDecoder`: successful decoding binds the exact parsed payload, matching run, wire schema, semantic checks, resolved identities and constructed operations. The `Decoder` interface carries this postcondition, established by that production body under the separate serde, Resolver and KeySet contracts.

`catalog_target::interpreted` relates every stored raw frame to its operations and preceding staging graph. `interpreted_at` covers arbitrary finite histories; `business_fold` proves that complete indexes equal their ordered business fold. Recovery preserves both the confirmed floor and the run/namespace interpretation. Failed partial apply keeps the previous complete history, and rebuild clears it before replay. The concrete decoder clears closure caches on reset; namespace reset must preserve the same interpretation.

Initial or incremental cached history must satisfy this interpretation when `Apply` is constructed. The native creation/append caller, JSON serialization and path normalization still must establish that precondition. Native namespace/initial-cache conformance and Source/Appender, complete outer locking, Coordinator business replay and cross-log/GC composition remain open. Successful serde parsing is a trusted relation; there is no valid-input parser acceptance theorem. Reproduce the complete positive proof and targeted controls with `check_verus.py --case catalog_frame_skips_run --case catalog_frame_wrong_bytes --case catalog_frame_drops_operations --case environment_catalog_decode_meaning_unbound`. No performance campaign accompanies this extension.

The [raw-decoder validation](../verification/measurements/20261025-catalog-frame.json) records 307 full positive Verus obligations and 14 semantic negatives, 66 native tests and 181 selected installed-wheel regressions. Both Clippy configurations, formatting and the environment inventory pass. Seven writer-lifetime TLC cases separately cover completion after death. An additional current-source audit passes 15 environment negatives (one overlaps the 14 above) and 47 receipt/consumption fault regressions, checking uncertain effects and truthful success boundaries. No new performance result is claimed.

The [catalog Target checkpoint](../verification/measurements/20261025-catalog-target.json) records exact source inputs and checks. Its controls cover skipped application, incorrect payloads, swallowed decoder/apply errors, altered recorded/plan bytes, lost Target validity and existing joint-state/append failure boundaries.

The recorded run passes 241 positive Verus obligations, 26 targeted semantic negatives, 60 native tests and 883 installed-wheel Python tests, plus formatting, default/Python clippy, examples and protocol/WAL campaigns. Older complete negative/TLC campaigns are not claimed for these changed inputs.

## Shared byte replay, catalog indexes and GC snapshots

Both logs call the production [wal_recovery.rs](../rust/wal_recovery.rs) driver. `read_frame` binds SHA inputs to the 24-byte prefix and the complete 56-byte header plus payload. `recover` proves ordered accepted records, permitted incomplete tails, sequence/boundary consistency and the confirmed floor; `finish` checks buffer discard, truncate, sync and seek ordering. These theorems use the explicit Device/Target interfaces in [wal_device.rs](../rust/wal_device.rs). SHA primitives remain trusted; the concrete [Source](../rust/wal_file.rs) descriptor/buffer View and JSON callbacks still need refinement.

The same driver now constrains every format-error branch. `format_result` and `malformed_at` distinguish malformed partial magic, header fields/digest, impossible lengths and trailer fields/digest; `recovery_malformed` additionally ties cursor, exhausted sequence and confirmed-floor errors to the accepted prefix. Device errors remain separate and may roll back storage. A malformed result preserves bytes and durability, so a rejected complete frame cannot be silently truncated. Target application failure may leave partial business effects and remains a separate recovery obligation.

`complete_decodes` and `incomplete_decodes` rule out spurious format rejection under successful Device operations. For arbitrary finite logs within u64/usize bounds, `legal_log_has_no_format_error` proves that a valid record sequence plus a permitted incomplete tail cannot produce a format error when the confirmed floor is covered. `legal_log_recovers_exactly` proves that successful replay yields exactly that ordered payload sequence; uniqueness and prefix lemmas connect repeated frame parsing to the whole log. These are byte-record theorems, not a proof that JSON payloads implement the intended queue/catalog operations.

A proof-gap regression changes the previous recovery implementation to always report an invalid cursor: all 225 old positive obligations still pass. The strengthened contracts reject the same change. This is evidence of an inadequate old specification, not an always-reject defect in the original runtime. The [replay completeness evidence](../verification/measurements/20261025-replay-completeness.json) records exact inputs and the focused algorithm/environment controls; tool errors and resource limits are excluded from successful negative results.

This checkpoint passes 238 positive Verus obligations, 29 targeted semantic negatives, 60 native tests, formatting and default/Python clippy. It does not claim a fresh installed-wheel Python or TLC run.


[catalog_state.rs](../rust/catalog_state.rs) is the actual index implementation used by normal catalog apply and recovery. Its View covers all six persistent indexes: owners, packs, permanent tombstones, pending deletions, staging edges and consumed handoffs. `apply_one` refines each typed operation; `apply` proves the ordered fold for arbitrary finite operation sequences, including the exact applied prefix on failure and no extra effect from a rejected reclaim. Repeated tombstones cannot reschedule deletion after a receipt. JSON decoding builds operations before applying them; rebuild still discards unconfirmed changes across frames.

`Indexes::roots` enumerates every owner set without filtering owner names. `Indexes::candidates` proves that the actual HashMap snapshot produces exactly sealed, unowned, untombstoned packs. Both executable loops have termination proofs. Collection and reactivation call these functions. The higher-level obligation remains to establish complete owners for every live queue, reader, checkpoint and publication and to connect immutable dependencies to these paths.

The call-chain review reproduced two defects: catalog reuse after an uncertain append, and changing pool/run configuration while holding another pool's lock. Writes now arm rebuild first; entry guards require an active transaction and the bound pool/run; nested transactions are rejected; fresh open checks device/inode identity. `identities::same_pool` and `same_log` verify the actual comparisons. Complete lock/open/guard placement remains a refinement obligation. External actors must not replace the shared log inode during active transactions; these checks do not provide distributed fencing.

The [catalog-index checkpoint](../verification/measurements/20261025-catalog-indexes.json) passed 182 Verus positive obligations and 17 targeted semantic negative controls for omitted index effects, swallowed apply errors, missing owners/roots, false owned/tombstone snapshots and ignored pool/run/device/inode bindings. All 50 native tests passed. Earlier helper/TLC runs are not automatically results for a changed suite. This proves the byte driver and actual index/snapshot code; **complete JSON business replay, Source, transaction call chains, closure caches, coordinator and physical GC composition remain open**.

Failed read/seek/truncate/sync may now change bytes and durability arbitrarily in the Device interface; the driver propagates failure without authorizing recovered state. Source invalidates its cached snapshot before fallible effects and refuses reuse after an error. A separate native regression catches a Rust append panic, checks that Journal remains poisoned, and reopens both an incomplete and a fully synced uncertain tail. These are adapter regressions, not a proof of complete Source/Journal conformance.

The actual staging-graph validator in `consumption.rs` now checks every added node, every child definition and immutable child-list equality. `same_children` compares the complete ordered string contents; `compatible` checks the whole HashMap update before replay applies its edges. Verus verifies these bodies with eight targeted negative controls (five graph checks and three existing child-coverage controls). The verifier now explicitly selects Rust 2024, matching Cargo. JSON/key syntax, authenticated-manifest binding and complete graph/history refinement remain separate obligations.

The [subsequent WAL-error/graph checkpoint](../verification/measurements/20261025-wal-errors-staging-graph.json) passed 186 positive obligations, eight graph negatives, 52 native tests and 868 installed-wheel Python tests. The earlier 20 replay-error negatives are preserved as a separate execution; they are not counted as a fresh full negative suite.

## WAL persistence and recovery prefixes

[`wal/tla/WalRecovery.tla`](../verification/wal/tla/WalRecovery.tla) represents
header, payload and trailer as three units per frame, separately tracking the
current prefix, synced prefix, client acknowledgements, replay boundary and
next write position. [`EnvironmentWal.tla`](../verification/wal/tla/EnvironmentWal.tla)
permits a crash to retain any prefix between the synced and current contents,
including an incomplete frame. Write/sync errors do not imply rollback. Safety
does not require faults to stop; termination separately assumes eventual
stability and fair recovery/retry scheduling.

Four positives cover one to three frames and complete corruption. Eight safety
negatives cover early acknowledgement, omitted payload, reusing an uncertain
append, retaining a torn tail, accepting complete corruption, truncating complete
corruption, skipping the recovery barrier and seeking to the wrong boundary.
A stalled recovery must produce a genuine liveness counterexample. Three
environment-only negatives permit loss of a confirmed prefix, false sync success
and false authentication success. All 16 cases meet their expected outcomes.

The model checks the boundary for appending after recovery. Its three units are
not a byte-parser proof or a proof of business-request idempotence. Actual
byte-prefix/corruption evidence comes from `wal/checks/crash_campaign.py` and
`tests/test_wal_recovery.py`. Checksums, JSON, descriptor correspondence and
cross-log refinement remain separate boundaries. `DurableHandoff` below adds finite cross-log composition checks; it does not
establish byte-level or complete Rust adapter refinement.

## Composing independent logs, handoff and GC

[`DurableHandoff.tla`](../verification/protocol/tla/DurableHandoff.tla) starts with
a sealed, durably published input closure and a staging owner. Catalog and queue
logs have separate current/durable states. Each append has header, body, trailer,
sync and reply steps. A complete uncertain frame can survive independently in
either log. Recovery must establish a new durability barrier before using its
state to skip retain or authorize cleanup. Current and durable filename presence
are tracked separately.

The model combines acceptance, staging consumption with its receipt, reply,
durable retirement, prune, mark, unlink, directory sync, reclamation receipt and
crashes. Reader, checkpoint and peer ownership can interleave. Seven positives
cover one/two packs, each external owner, all three together and a permanent reader; the all-owner case checks
257,226 states. Thirteen algorithm safety negatives break cross-log ordering,
dependency coverage, recovery/deletion barriers or late-retain fencing; one
liveness negative stops retry. Six environment negatives lose confirmed
catalog/queue prefixes, lie about directory sync, remove work fairness or
eventual stabilization, or prevent an application holder from finishing. All 27 cases have their required outcomes.

Application completion is independent of filesystem recovery and scheduling. `PermanentActors` can retain data forever; `ReadersProtected` still holds. Global reclamation is required only when application owners finish. The per-pack property separately requires reclamation once a pack becomes permanently unowned. The permanent-reader positive explores 4,990 states; requiring unconditional reclamation when that premise fails produces a genuine temporal counterexample.

The 22 fault windows in `tests/test_durable_handoff.py` cover retain, queue
acceptance, Apply and committed cleanup. Two queues transfer an input with a
separate dependency pack; reader/checkpoint owners protect it until explicitly
released. After final reclamation, recovery and same-request replay must preserve
one Submitted transaction, reject conflicting contents, avoid reopening collected
manifests and create no new owner. These regressions pass. They exercise error
returns and reopening; the separate crash campaign covers abrupt process exit.

`EnvironmentCrossLog.tla` isolates complete-frame recovery projections and barrier
responses. The three-unit/byte/snapshot correspondence is not mechanically
refined; real locks, checksums, JSON, writer restart and arbitrary request histories
remain boundaries. This model has one handoff and finitely many owner lifecycles.
Later restaging of the same reference is checked separately by `CommitCleanup`,
and stale attempts/conflicting completion by `Completion`. Finite composition
checks do not prove arbitrary queue/graph sizes or unbounded workflows; the
unbounded ownership and retry theorems retain their own premises.

## Verification layers and current coverage

These layers distinguish kinds of evidence, not a score or a claim of full coverage.

| Layer | Verified / checked now | Remaining boundary |
|---|---|---|
| L0: contract and environment | Explicit durability, visibility, ownership, fencing and failure contracts in this document and [FILESYSTEM](FILESYSTEM.md) | Filesystem, OS, hardware, locks, hashes and external fencing are assumptions; a failed operation may have taken effect |
| L1: finite protocol exploration | TLC checks published reads, ownership transfer / reader / checkpoint / rewind / GC, small-word arithmetic, usage caches, directory creation/retry, GC barriers, lease selection and completion/recovery; unsafe variants must yield named counterexamples | Only configured state spaces; no mechanical refinement from these `.tla` files to Rust |
| L2: unbounded mathematical proofs | Verus proves catalog ownership safety for arbitrary owner/pack sets and arbitrary-length histories of modeled retain/release/mark/unlink actions; controller bounds/order, conditional completion, and GC retry liveness from stable I/O plus weak fairness | Serialized durable catalog actions and complete closures are supplied; real I/O/lock/retry-adapter conformance and whole-system progress remain unproved |
| L3: production Rust conformance | The actual `wal.rs` append controller/frame-boundary decision, `typed.rs` layout equality loop, `access.rs` generic token decision, `consumption.rs` child-coverage loop, `incomplete.rs` cleanup controller/PID comparison, `identities.rs` cache/encoding functions, `pending.rs` description/sync-state functions, `finalization.rs` stage/seal controller and generic handoff admission/acknowledgement, `holding.rs` union loop, `directory.rs` parent-response classifier and barrier controller, `discovery.rs` decisions, `bounds.rs` functions, `protocol.rs` commit/delete/receipt steps and actual `execute_commit` effect dispatch,  candidate-selection and lease-filter deduplication loops, `reclamation.rs` index mutations, `stream.rs` length transitions, `replay.rs` durable-prefix guards/read-ahead capacity and generic `traversal.rs` frontier operations are proved against their contracts / `View` | String HashSet/HashMap key and BTreeSet ordering laws are explicit premises; native CommitIo/GC effect implementations, complete dependency traversal, JSON bytes/path/business adaptation, concrete WAL Device/Target adapters and transaction call chains are not Verus-proved; actual index and HashMap snapshot loops are now checked separately |
| L4: implementation integration | Native/Python tests, byte-prefix/corruption campaigns, commit crash windows, late-retain fencing and failed GC directory-sync retry | Tested executions, not exhaustive Rust verification or power-loss certification |
| L5: deployment and performance | Four-client visibility, concurrent GC, raw/queue bandwidth and real 100 GB native publication/read measurements; artifacts described in [FILESYSTEM](FILESYSTEM.md) | No universal latency bound, cold-cache guarantee, bandwidth SLA, or TB-scale physical-I/O claim |

The critical distinction is between L2 and L3: catalog safety is proved for its
model, while candidate selection and the effect controllers have actual Rust
conformance proofs. The surrounding catalog and queue adapters still require
review and fault tests. The checked-in Verus entry point is
[`verification/common/verus/verus.rs`](../verification/common/verus/verus.rs); the model theorems are in
[`verification/protocol/verus/control_plane.rs`](../verification/protocol/verus/control_plane.rs).

The earlier [tool-based directory reorganization](../verification/measurements/20261025-verification-layout.json) reran the complete checks from its then-current paths: 823 Python tests, 40 native tests, all seven Rust examples built, 349 TLC cases, and the full Verus campaign (119 positive checks and 105 expected semantic negatives). The inventory remains at 42 contracts, 41 consumers and 19 component layers. Production Rust/Python and the native binary are unchanged; the Verus entry point changes relative source paths. These are fresh checks, separate from the earlier reused evidence below.

The preceding [offline-alias fix record](../verification/measurements/20261025-offline-alias.json) passes 823 Python tests, 349 TLC cases (25 environment controls, 18 true temporal counterexamples) and 119 positive Verus checks. The inventory contains 42 contracts, 41 consumers and 19 component layers. The unchanged 40 native tests and 105 Verus semantic negatives are reused after exact-input checks. Python trace correctness remains an adapter obligation.

The preceding [offline-cleanup fix record](../verification/measurements/20261024-offline-guard.json) rebuilds the wheel and passes 804 Python tests, 332 TLC cases (24 environment controls, 17 true temporal counterexamples) and 119 positive Verus checks. The inventory contains 41 contracts, 40 consumers and 19 component layers. Rust/native binary inputs are unchanged: 40 native tests and 105 semantic negatives are reused after source checks. Python guard execution evidence and unproved complete refinement are recorded separately.

The preceding [name/data retry record](../verification/measurements/20261024-read-lookup.json) reruns 316 TLC cases (23 environment controls and 16 true temporal counterexamples) and 119 positive Verus checks. The inventory contains 40 contracts, 39 consumers and 19 component layers. Production source is unchanged: 782 Python/40 native tests and 105 semantic Verus negatives are reused from exact-source evidence; new four-host measurements are archived separately.

The preceding [descriptor-reuse validation](../verification/measurements/20261024-single-open.json) ran 782 Python / 40 native tests, 302 TLC cases (22 environment controls, 15 true temporal negatives), 119 Verus checks and 105 expected semantic rejections. The inventory retains 39 contracts and 19 component layers, with 38 consumers. Actual frame-position arithmetic and unproved complete descriptor-adapter correspondence are recorded separately.

The preceding [typed-preflight record](../verification/measurements/20261024-typed-preflight.json) passes 782 Python / 38 native tests, 291 TLC cases (21 environment controls, 14 true temporal negatives), 118 Verus checks and 101 expected semantic rejections. The inventory has 39 contracts, 37 consumers and 19 component layers. Four-host 8,400-call and shared-record interleaved campaigns complete normal GC; actual Rust matching and unproved JSON/Python/I/O adapter boundaries are recorded separately.

The subsequent [valid small-slice attribution](../verification/measurements/20261024-read-latency.json) adds measurement probes and `ENV-PERFORMANCE` evidence links; production code, environment-contract semantics and proof inputs are unchanged. It reruns 48 relevant read TLC cases and 118 positive Verus checks, separately recording 14,400 new reads and normal GC. Full integration suites and Verus negatives are not rerun. Libc timing adds neither a latency guarantee nor a complete I/O refinement proof.

The preceding [fixed tensor-granularity record](../verification/measurements/20261024-tensor-granularity.json) freshly passes 761 Python / 38 native tests, 282 TLC cases (20 environment controls, 13 true temporal negatives), 116 Verus checks and 96 expected semantic rejections. The inventory contains 38 contracts, 36 consumers and 19 component layers. Real 100 GB four-host and Python 2 GiB-threshold runs complete normal GC; actual span arithmetic is proved, while the Python producer and complete I/O/recovery adapters remain unproved.

The preceding [position-dependent range-read record](../verification/measurements/20261024-positioned-range.json) checks a real 100 GB record, four-host byte comparisons and GC. All 273 TLC cases (20 environment controls, 13 true temporal negatives), 115 Verus checks and 92 expected semantic rejections pass. The inventory has 38 contracts and 35 consumers. Production code and 20 Verus inputs are unchanged; 67 production/test inputs are checked before reusing the 754 Python / 38 native integration results. The new example is separately built, executed and linted.

The [queue/large-record workflow](../verification/measurements/20261024-queue-record-workflow.json) writes a real 100 GB record, authenticates disjoint shards on four hosts, preserves cancelled readers and reclaims only after their acknowledgements. History workloads contain 0/128/1024 handoffs with 80 recovery samples each. That run’s 256 TLC cases (nineteen environment controls, twelve true temporal negatives), 115 Verus checks and 92 expected semantic rejections pass; the inventory has 38 contracts and 34 consumers. Production Rust/Python is unchanged: prior 754 Python / 38 native integration tests are reused after source checks, with the new probe separately built, executed and linted.

The earlier [post-commit cleanup validation](../verification/measurements/20261024-commit-cleanup.json) passes 754 Python and 38 native tests, 115 Verus checks with 92 expected semantic rejections (four environment-only), and 240 TLC cases (seventeen environment-only, nine temporal negatives). The inventory has 38 contracts and 33 consumers. Four hosts reproduce staging leaks in 64 of 96 baseline cases; all 288 fixed cases finish cleanup, preserve an independent reader and require no manual data deletion. Full adapter/recovery refinement remains unproved.

The preceding complete [writer-access validation](../verification/measurements/20261024-writer-access.json) passes 717 Python and 35 native tests, 112 Verus checks with 88 expected semantic rejections (four environment-only), and 224 TLC cases (sixteen environment-only, seven temporal negatives). The inventory has 37 contracts and 32 consumers. Four hosts observe 96 blocked baseline cases and 480 successful fixed workflows; fixed GC removes 960 files without manual data cleanup. The token decision is actual verified Rust; mutex/GIL/FFI and the complete adapter remain outside that proof.

The earlier [100 GB four-host workflow](../verification/measurements/20261023-shared-record-workflow.json) passes successful publication, disjoint authenticated reads and uncertain handoff/reclamation. A full TLC rerun has 218 expected cases (fifteen environment-only, six temporal negatives); that run’s inventory had 36 contracts and 31 consumers. That workflow iteration added only a probe/model, with unchanged production Rust/Python. Exact verifier inputs, contracts and test sources are checked before reusing the preceding 693 Python/35 native tests and 111 Verus checks/85 semantic rejections; these are not claimed as new test runs. The original Verus report retains its registry hash; the current registry only adds a consumer and evidence links.

The preceding full wheel integration run, [provisional owner cleanup](../verification/measurements/20261023-provisional-holds.json), passes 693 Python and 35 native tests, 111 Verus checks with 85 expected semantic rejections (four environment-only), and 207 TLC cases (fourteen environment-only, five temporal negatives). The inventory has 36 contracts and 30 consumers. Four-host comparisons run 480 cases per binary: 480 leaked internal owners in the baseline, zero in the fix; an independent reader remains protected. That repair validation did not rerun 100 GB; the preceding paragraph reports this turn’s native large-file run on the same production sources. Earlier measurements for other binaries remain separate. Full adapters, model composition and writer restart remain unproved. The previous [staging consumption record](../verification/measurements/20261023-staging-consumption.json) retains its binary-specific proof and post-GC retry evidence.

The [writer-lifetime supplement](../verification/measurements/20261023-writer-lifetime.json)
adds a separate process environment and makes the open-reservation progress gap
executable. At that point the inventory had 33 contracts and 26 consumers. It distinguishes
a counterexample to a proposed timeout-based seal from a liveness counterexample
for the current policy after writer death; no crash-recovery algorithm or new
production Rust proof is claimed. See the [environment review](ENVIRONMENT_MODELS.md#writer-lifetime-and-eligibility-for-collection).
Four hosts exercise 16 actual pause/resume/kill cases. The full formal rerun has
171 TLC cases (eleven environment-only and three liveness negatives), plus the
unchanged 101 Verus checks and 73 rejected semantic mutants. Production Rust and
the wheel are unchanged from the integration run below; its 474 Python and 34
native tests were not rerun for this model/documentation supplement.

The preceding complete integration verification run is
[`20261023-gc-parent-repair.json`](../verification/measurements/20261023-gc-parent-repair.json):
101 Verus checks, 73 rejected semantic mutants (four environment-only),
167 TLC cases (eleven environment-only, two genuine liveness negatives),
474 Python tests and 34 native tests. It adds parent availability/repair, an actual
response classifier, 32 environment contracts, four-host syscall tests and a real
100 GB native-core publication/authentication/GC run. Full I/O/lock refinement,
writer-process restart and whole-system progress remain unproved. Earlier
[incomplete-write cleanup](../verification/measurements/20261023-incomplete-publication.json),
[catalog encoding](../verification/measurements/20261023-catalog-encoding.json),
[path identity](../verification/measurements/20261023-path-identity.json) and
[pending publication](../verification/measurements/20261023-pending-publication.json)
retain their exact-binary measurements and proof scope.

The preceding [standalone finalization / 100 GB run](../verification/measurements/20261023-publication-finalization.json),
[publication-retry fix](../verification/measurements/20261023-publication-retry.json),
[directory-durability fix](../verification/measurements/20261023-directory-durability.json),
[GC discovery fix](../verification/measurements/20261023-catalog-discovery.json),
[bounded catalog read-ahead](../verification/measurements/20261023-catalog-buffer.json),
[GC stale-stat fix](../verification/measurements/20261023-gc-stat.json),
[empty-refresh/history optimization](../verification/measurements/20261023-catalog-history.json),
[lost-tail fix](../verification/measurements/20261023-catalog-replay.json),
[unlocked 100 GB scan](../verification/measurements/20261023-unlocked-reactivation.json),
[phase controls](../verification/measurements/20261023-reactivation-timing.json),
and [memory fix](../verification/measurements/20261023-reactivation.json) retain
separate binaries and scopes. The earlier 8.68-second anomaly remains
unattributed; these fixes do not explain that sample either.

## Running the finite models

straw uses TLA+ and TLC to check bounded models of published extent reads and
ownership/GC. Run the checked-in specifications, including negative controls:

```sh
# Requires Java 17+ and Python 3.10+, but no Python straw installation.
python verification/runners/check_tla.py
# Offline: provide an existing TLC jar.
python verification/runners/check_tla.py --jar /path/to/tla2tools.jar --output /tmp/straw-tla
```

The runner downloads TLA+ tools from its configured release URL, or uses `--jar`.
It records the jar's SHA-256 for provenance without enforcing a checksum.
`target/tla/report.json` records source/configuration hashes, state counts,
exit status and results. Each case directory preserves its
exact `.tla`, `.cfg` and `tlc.log`, including counterexamples. `--case NAME` selects
a case; omit it for the full gate. The default timeout is 300 seconds per case.
A timeout, parse error, unexpected invariant or missing counterexample fails the
gate. Safety negatives require TLC invariant violations (exit 12), with the
specified invariant and a state trace. Temporal negatives require exit 13, the
single configured property's name and a looping/stuttering counterexample.
CI runs this gate and uploads the evidence.

TLC provides finite-state model checking, not a TLAPS inductive proof or a
Rust refinement proof. The separate Verus layers below cover specific Rust
bodies and unbounded model properties. No layer establishes that every straw
API is bug-free; deployment assumptions require separate qualification.

## Published reads: `ExtentVisibility.tla`

Publication requires durable data. As a conservative fault assumption (not a
claim about normal JuiceFS behavior), after publication each reader's **fresh
open** can observe any subset of the length, header, footer, index, envelope,
payload and tensor chunk as not yet visible. The two header categories represent
all-zero and partially zero-filled headers. Subsets deliberately overapproximate
byte/cache-page combinations; visibility may regress across opens before the
configured convergence bound. A successful header read is not a fence for the
remaining regions. Each reader has its own state and retry budget.

The model abstracts hashes as authentication without collisions. `Corrupt=TRUE`
represents permanent failures in the checksummed regions, not arbitrary semantic
reference errors or nonzero wrong header bytes (those still fail immediately).
It checks:

- `NoUnverifiedReturn`: a successful read has authenticated the requested data
  and follows durable publication.
- `NoPrematureFailure`: retryable validation failures consume the shared budget
  before surfacing an error.
- `EventuallyTerminates`: under weak fairness of publication, opens and checks,
  each read ends in success or error, including permanent corruption.
- `VisibleInBudgetSucceeds`: if fresh opens become fully visible by the final
  permitted attempt and the data is not corrupt, every read succeeds.

The `documented_*` cases set `VisibleBy=0`: after publication every fresh open
has a complete correct view, representing the stronger documented contract.
Both the fixed and historical policies pass under that assumption. A historical
failure in a delayed-view case therefore does not itself show a violation of
the documented JuiceFS contract.

The defensive fault suite explores one reader with the actual five-retry budget, two independent
readers with one retry each, visibility beyond the retry budget, and permanent
corruption. This is not an arbitrary-reader or arbitrary-retry theorem. Eventual
visibility **without a bound** cannot guarantee success with bounded retries;
exhaustion is an allowed outcome and never permits unchecked bytes.

Historical modes encode the policies before v0.1.2, in v0.1.2, and in v0.1.3.
Their configurations isolate all-zero headers, partial headers, and stale footers,
respectively; all must violate `NoPrematureFailure`. They are abstractions of those versions,
not executions of historical binaries. A `skip_validation` control must violate
`NoUnverifiedReturn` so that making reads always succeed cannot pass the gate.

### Counterexample and fix

The v0.1.3 counterexample is: data synced and reference published → reader opens
an extent with a valid header but an invisible later region → validation fails
without a retry. Header-only retry cannot handle independently stale footer,
index, envelope, payload or chunk bytes. The public API regressions reproduced
footer and index failures against the unmodified v0.1.3 wheel.

`rust/store.rs` now retries the complete requested read on retryable visibility
failures. Header, index, envelope and payload/chunks share the existing five
waits (100, 250, 500, 1000, 2000 ms), rather than nesting budgets per stage.
Every retry drops the failed attempt's file/buffer, reopens, and revalidates.
A session may reuse an already authenticated index for the identical descriptor;
it still authenticates each record/envelope and touched chunk. A checksum failure
cannot distinguish stale bytes from permanent corruption, so both are retried;
persistent failures still return `CorruptData`. Wrong identity/schema, nonzero
bad headers and unrelated I/O errors are not converted into successful reads.
The 3.85 seconds is total retry sleep, not a deadline on filesystem I/O.
Manifest/dependency traversal can perform multiple separately bounded reads.

| Model step | Implementation | Regression evidence |
|---|---|---|
| Sync / Publish | `Store::write_prepared`, `finish`, staging | Existing publication/crash tests |
| Open / Check | `Config::inspect_once`, `frame_with_index`, `read_frame`, `read_frame_range` | `tests/test_extent_visibility.py`: delayed header/footer/index/frame/envelope/payload/manifest and tensor chunks |
| Retry with shared budget | `retry_extent_with_wait`, `Reader::index_once` and read entry points | Native `one_retry_budget_covers_header_index_envelope_and_payload` replays changing stale regions without timer races |
| Reject corrupt data | Authentication on every requested record/chunk | Persistent corruption, nonzero bad header and cached-index tests |

The delayed-region tests alter local test files to simulate reader observations;
they do not establish that a particular filesystem produces every modeled view.
In particular, the official JuiceFS documentation describes default close-to-open
consistency and flushing on fsync; with client writeback disabled it uploads data
before committing metadata. It does **not** specify arbitrary post-fsync zero
holes as normal behavior. Open caching/writeback change the guarantees. The
footer/index counterexamples here establish a limitation under the modeled
faults, not a newly reproduced JuiceFS defect. See the
[deployment contract and measurement method](FILESYSTEM.md#visibility-contract-and-measurement).
The actual filesystem assumption is documented in [FILESYSTEM.md](FILESYSTEM.md).

## Ownership and GC: `Ownership.tla`

The model explores two queues, two packs (one publication's manifest/dependency
closure), one independent reader, one checkpoint, one crash, and at most one
checkpoint rewind per queue. Packs seal independently. A durable catalog update
is atomic in this abstraction; queue WAL write/sync/reply and GC scan/mark/unlink
are separate actions. The catalog lock excludes retain/release while collection
chooses and tombstones its candidates. Recovery prunes uncommitted queue roots;
committed roots and independent readers survive.

It checks types, durable queue ownership, active reader/checkpoint protection,
transitive reachability, acknowledged WAL durability, sealing before collection,
and durable tombstones before physical deletion. Six negative controls remove
retain-before-WAL, dependency protection, GC locking, tombstone ordering, reader
protection, or retention on checkpoint rewind. Each must fail its named invariant.
These mutations demonstrate sensitivity; they are not six newly found code bugs.

| Model actions | Code correspondence |
|---|---|
| Write / Sync / Stage / Seal | `Store::write_prepared`, `Config::stage`, `Catalog::pack` |
| Adopt / QueueWrite / QueueSync / Reply | Coordinator retention and `Journal::append` |
| Pin / Unpin / SaveCheckpoint | Catalog durable owner sets; caller's actual read-completion acknowledgment |
| Rewind | `Catalog::retain_reactivated` before consumer-state WAL append |
| Scan / Mark / Unlink | `Catalog::transaction`, `collect` |
| Crash | Queue WAL replay and ownership reconciliation |

No new ownership violation was found for the modeled protocol. Catalog byte-level
crash behavior remains covered by `verification/wal/checks/crash_campaign.py`, not by treating
an atomic catalog action as a proof of its implementation. The existing independent
Python ownership model remains a complementary check.

Unmodeled boundaries include arbitrary dependency graphs and cycles, task lease
scheduling/epochs, arbitrary request histories, checkpoint replacement, multiple
simultaneous crashes, memory safety, hash collisions, and filesystem/OS internals.
The model does not assert GC liveness: an unacknowledged reader or crashed writer
may retain data forever. Maintain these assumptions, the code mapping and the
counterexample regressions together when changing the protocol.

## GC deletion, including failures after unlink

`GCDeletion.tla` separates durable tombstoning, unlink and parent-directory sync.
A failed operation may leave a name absent without making that absence durable;
a crash can restore an unsynced entry. Before stabilization the model permits
arbitrarily many retries. Under eventual fault cessation and weak fairness of
the GC actions, a permanently ownerless sealed pack is durably reclaimed.
`DurableCompletion` requires the directory barrier before success, and
`DeleteAfterMark` requires a tombstone before unlink. Mutations that skip sync
for an absent file or unlink before marking must violate these invariants.
This is a one-pack liveness check; `Ownership.tla` separately checks ownership
interleavings and intentionally makes no general GC liveness claim.

The missing-file retry exposed a real implementation gap. If unlink succeeded
but directory sync failed, a subsequent `collect` skipped the barrier because
`stat` returned `ENOENT`. `Catalog::collect` now drives the verified deletion
controller through both Unlink and SyncParent, including an already-absent pack.
A regression injects two consecutive sync failures and then verifies successful
recovery without counting the bytes twice. Late retain still fails against the
durable tombstone. Other errors propagate; directory existence and exclusive
library management of storage paths remain part of the storage contract.
The current implementation first unlinks all pending tombstoned targets, then syncs each
distinct parent once. Already-absent targets still contribute their parent.
`delete_batch_finished` verifies that all members await SyncParent and that the
adapter reports all barriers successful before the batch can finish. Files and
bytes are reported only on whole-call success; a later retry does not count names
already removed. An error aborts the call and leaves tombstones available for retry.

`GCBatchDeletion.tla` explores three permanently unowned packs in two directories
(two packs share one parent), with arbitrary pre-stabilization failures/retries.
`EnvSyncedDirectory` in the independent persistence environment makes one barrier
persist preceding name changes only in that directory. The positive model has
144 states and checks tombstone ordering, durable completion and conditional
reclamation. Three algorithm controls must violate `DurableCompletion`: syncing
before the final unlink, omitting parents of absent names on retry, and dropping
one directory. These are unsafe batching variants, not failures of the previous
per-file barrier implementation. Runtime regressions cover failures on the first/second directory
barrier and after partial unlinks, including repeat calls with all names absent.

The Rust batch-check loop is proved for arbitrary slice lengths. Path grouping,
parent-directory coverage and successful syscall dispatch are still adapter
obligations, not Verus-proved. The earlier single-pack unbounded liveness proof
has not been mechanically composed with this multi-pack I/O adapter. Batching
alone retained historical stat/scanning; see the [paired measurements](FILESYSTEM.md#batched-directory-barrier-comparison-2026-10-23).

### Durable reclamation receipts

After every affected parent is synced, GC appends and syncs an optional
`reclaimed` catalog event. Replay removes those paths from the pending-deletion
index and pack candidates, retaining permanent tombstones to reject late retain
and path reuse. Later calls skip their payload stat/unlink/directory barriers.
Absence alone is insufficient: a missing or torn receipt causes a safe retry;
a surviving complete receipt is synced during catalog recovery before reuse.
Empty calls do not append receipts, but still acquire/refresh/sync the catalog.

`GCReceipt.tla` checks this order, durable absence before skipping, permanent
fencing and eventual retirement under stabilization/fairness. Its one-pack
positive case now has 64 states. The separate `EnvStatPresence` environment
(`ENV-GC-STAT`) allows either presence observation to remain stale. Stat need
not converge, but calls must return and successful unlink/NotFound must still
establish actual absence. Four negative controls record before directory sync,
treat absence as a receipt, drop the tombstone, or skip unlink after stat
NotFound. Each violates its named invariant; the last reproduces the old
adapter branch and violates `ReceiptIsSafe`. This is finite-model evidence,
not an extension of the unbounded deletion theorem through receipt recovery.

The old adapter treated stat NotFound as successful Unlink, then synced the
directory and wrote a receipt. If the file still exists, a barrier merely
persists that existing name. Every pending path now gets an unlink attempt;
stat is diagnostic accounting. A successful unlink with unknown size counts
one file and zero bytes, so `reclaimed_bytes` is not an exact capacity audit.
Native regressions cover existing/missing names, unlink/barrier failures and
reopen. Four-host injected stale-stat tests leave orphans in all eight old
cases and actually delete the file in all 100 fixed cases. Injection does not
show the deployment naturally exhibited this fault. The fix does not repair
pre-existing erroneous receipts.

The actual `reclaim_*` controller proves SyncParents precedes RecordReceipt.
`reclamation.rs::mark/retire` proves the actual HashSet updates: pending remains
a subset of tombstoned paths, repeated tombstones cannot reschedule completed
work, and receipts cannot retire unmarked paths or remove the permanent fence.
These contracts explicitly require `ENV-STRING-KEY`, the standard String
hash/equality law that pinned vstd does not discharge. JSON decoding, syscall
dispatch, frame durability and replay-to-index correspondence remain unproved.

`tests/test_gc_receipts.py` covers receipt append cut points, every byte prefix
and single-byte corruption of a complete receipt frame, duplicate tombstones,
unmarked receipts, restart and late-retain rejection. Four-host interoperation
checks the preceding batched binary ignoring receipts and the new binary
finishing old deletions. This is not arbitrary-version compatibility. A first
nonempty GC adds a catalog append/sync; permanent tombstones and the log still
grow, and cold replay remains proportional to history. Measurements in
[FILESYSTEM](FILESYSTEM.md#durable-receipt-comparison-2026-10-23) cover warm repeated calls, not a latency guarantee.

## Filtered lease selection and completion/recovery

`LeaseSelection.tla` checks all filters up to four entries from three tasks and
batch limits 1..4. It requires unique assignments, no lost assignment slots,
limit compliance and termination under weak fairness. The legacy variant
retains duplicate filter entries and must violate `UniqueAssignments`.

This reproduced a real Rust bug: `acquire(max_tasks=2, task_ids=["a", "a", "b"])`
could return two leases for `a`, with the second immediately superseding the
first. `rust/selection.rs::first_occurrences` now removes repeated scheduling
keys before issuing leases. It preserves sorted acquisition order and applies
the batch limit to distinct tasks. Verus proves the complete production loop:
output length equals input length, and each Boolean is true exactly for the
first occurrence of that full key. No bound on filter size is used in the proof.
`tests/test_lease_selection.py` checks duplicate inputs, batch sizes, heartbeat
authorization, priority/control/prefix filtering and recovery.

`Completion.tla` separates admission, complete WAL write, sync, in-memory apply,
reply and recovery. It covers one task, two attempt numbers, two epochs, two
submission IDs, two logical digests and one crash. Requests may be repeated
arbitrarily in this finite state space. Recovery may keep a complete frame from
an interrupted append, syncing it before use, or retain the previous durable
prefix. It checks acknowledged durability, one acknowledged completion per
task, current-lease admission, matching receipt replay across epoch changes,
conflict rejection, and return/recovery progress under weak fairness. Physical
relayout is abstracted away: the logical digest is the retry identity.

Five mutations move lease validation ahead of receipt lookup, skip authorization,
reply before sync, discard durable recovery, or ignore conflicting content.
Each must violate its named invariant. These are sensitivity controls, not five
new Rust defects. Corresponding production paths are `Coordinator::valid_lease`,
`complete_task`, the commit controller and `Journal::recover`; existing expiry,
idempotency, reply-loss, torn-WAL and new commit-cut-point regressions exercise
them. **Completion policy and replay remain finite-model/test evidence**, not an
unbounded Verus proof of the full API. The deduplication loop has actual Rust
proofs; JSON-to-scheduling-key extraction and the callers remain outside them.

## Failed catalog replay and tentative cache state

Another client can finish a valid catalog frame and fail before sync. A reader
may apply that frame and advance its in-memory offset, then fail its own replay
sync. If storage subsequently loses that unconfirmed tail while the client
survives, the old implementation misclassified the shorter log as loss of a
confirmed prefix. Repeated calls on that object returned `UnsafeRecovery`, while
a fresh object recovered. A targeted syscall fault reproduced this on the old
wheel; it was not an observed JuiceFS service power failure.

The catalog tracks this object's confirmed durable byte lower bound separately.
When an unread suffix exists, it marks possible tentative cache changes **before**
parsing/apply. Even a partially failed apply therefore requires rebuilding owners,
packs, tombstones, pending deletions, closure cache and sequence on the next
locked fresh open. File size and the complete replay prefix must reach the
confirmed floor; otherwise recovery rejects without truncating bytes. Only
successful replay sync confirms the new position before caller mutation.

An empty refresh has changed no confirmed cache state, so a sync failure can
leave that cache reusable. The next operation still locks, reopens, checks file
size/new frames and syncs; it must observe subsequent peer updates. Previously,
unconditional invalidation added a full replay of about 46.63 MiB in the largest
measured history. This change removes that redundant work. First open and
recovery after tentative changes still replay full history, with no new time or
memory bound and no wire-format change.

`CatalogReplay.tla` checks one confirmed prefix and an optional uncertain owner
frame. Tail/no-tail positives have 28/8 states, checking confirmation/durability,
coherent returned cache state, no false corruption, no rebuild for a clean empty
refresh, and conditional completion under stability/fairness. Five algorithm
controls keep a stale offset, retain stale owners during rebuild, confirm early,
mark only after partial apply, or invalidate an empty refresh unconditionally.
The last violates `NoUnneededRebuild`, a work-avoidance policy, **not a latency
guarantee**.

`PartialApplyAndRepair` explicitly combines a partial memory effect with external
removal of a failed unconfirmed tail. It is not automatic repair by straw or a
proof that persistent corruption heals. Actual corrupt frames are rejected
without altering their bytes. This is one serialized refresh, not arbitrary
concurrent appends, byte framing or complete GC/queue recovery.

Verus proves actual `replay.rs::start/prepare/accepts/confirm` for all `u64`
positions and Boolean flags: prepare preserves the durable floor and sets the
rebuild flag to old flag OR possible changes, so an unchanged step preserves a
pending rebuild. Accepted positions reach the floor; confirmation is monotonic
and rejection leaves state unchanged. Five semantic controls must fail. The
caller's marking point, I/O, JSON apply, complete cache clearing and locking are
still unproved adapters.

Native regressions retain lost retain/release/seal/tombstone/receipt tails,
repeated failures, and lost/incomplete confirmed prefixes. New cases check peer
release/seal visibility after an empty failure, continued rejection of confirmed
prefix loss, and removal of a partially applied owner after explicit tail repair.
The new binary additionally passed 40 lost/surviving-tail trials on four hosts.
The Linux fsync fault shim is not linked into the library or Verus-proved.

## Integer bounds and usage caches

`ExtentArithmetic.tla` exhaustively checks small unsigned words: a frame end or
usage total is exact when representable and rejected otherwise. Its historical
variants reproduce the unchecked *inner* frame-size sum and wrapped usage sums.
`UsageCache.tla` models reservations, completion, cancellation, acknowledgement
and cache refill. A cache is exact or absent; overflow invalidates it and must
not interrupt applying a durable event. These models use small words (TLC does
not represent machine `u64` arithmetic); native/Python regressions exercise
`2**64 - 1` and valid sparse offsets crossing 2 GiB, 4 GiB and 2 TiB.

The regressions reproduced three real pre-fix failures: an authenticated index
with a wrapped frame length was accepted; a publication's token sum wrapped to
zero; warm and cold coordinator usage totals wrapped to zero. Checks now reject
unrepresentable publication/manifest totals before writing payloads/manifests.
Diagnostic usage queries return an explicit overflow error; durable completions
and scheduling continue, and recovery retains the committed results. This is a
wire-integer representation limit, not a reintroduction of logical backlog quotas.

Range reads previously allocated an entire checksum chunk. Tensor metadata
limits checksum count, so large tensors can have very large chunks. Verification
now streams through at most 1 MiB of scratch, in addition to the requested output
and index/metadata. Every touched chunk is still fully authenticated before any
output is returned. `tests/test_extent_bounds.py` checks distant corruption,
read-buffer boundaries, high offsets, overflow/recovery and a 16-byte range from
a 64 MiB chunk in a process with only 32 MiB additional address space. Reading a
complete record still requires memory for the complete returned value; use ranges
for large tensors. Small slices can still have large **I/O amplification** when
the checksum chunk is large; bounded scratch does not remove that cost.

## Complete payload verification during checkpoint rewind

`Catalog::retain_reactivated` previously called `Reader::read` on every reachable
record and discarded the returned payload. Both direct and transitive 64 MiB
records aborted fresh processes given only 32 MiB additional address space.
This path now uses `Reader::verify`: read and SHA-256-check every payload with
at most 1 MiB scratch. All missing roots must pass before the owner append and
subsequent queue WAL update. Public whole-record reads still allocate their
returned payloads.

`ReactivationScan.tla` checks one attempt with two records, lengths 0..3, a
two-byte buffer, every corrupt-byte subset and complete/short/error read
responses. Its 2,724 positive states satisfy bounded scratch, complete coverage,
authentication before authorization, no authorization after failure and
termination under weak fairness. Four controls allocate the entire record,
skip its tail, ignore corruption or accept a read error; each must violate its
named property. These small sizes bound exploration, not the API. The production
1 MiB scratch cap is an algorithm policy, not an environmental numerical bound.

`EnvironmentRead.tla` isolates read responses (`ENV-READ`); detecting corruption
still assumes `ENV-AUTH`. The model starts with a complete closure and protected
source owner. It does not compose real locking, graph traversal or WAL recovery,
or prove syscall termination. The existing visibility model handles retry
budgets; this model adds no retry limit.

The actual Rust `verification_capacity` proves `min(length, 1 MiB)` for all
`u64` lengths, alongside `next_buffer`'s bounded, strictly increasing, exact
steps. The surrounding `read_exact`/SHA-256 loop and catalog adapter have no
complete Verus proof. Native regressions cover empty records, buffer boundaries,
tails, short reads and retries. Python regressions cover constrained memory,
direct/transitive payloads, unchanged owners/WAL when a later record is corrupt,
successful retry and GC protection.

This bounds payload scratch only: manifests, indexes, graphs and catalog state
still consume memory. Every reachable payload must be read. The memory-fix run
still held the catalog lock; the subsequent change below releases it. Both
measurement rounds preserve their own binaries and scope.

## Unlocked authentication and a fresh restoration decision

Coordinator's Retain phase now selects missing owners and checks their sources
under the catalog lock, releases both the file lock and in-process catalog
mutex for complete authentication, then locks/refreshes again and checks every
closure/source/tombstone before any owner append. Ordinary roots are retained
within that last transaction, before queue WAL append.
`Config::verify_reactivation` returns an opaque `VerifiedReactivation` bound to
the absolute pool path, run ID, original owner/reference pairs and actually
verified reference digests. It is neither a durable owner nor a reader pin and
cannot authorize reads or be reused across pools.

GC can race the scan. An open may fail; an already-open descriptor may also
finish after unlink. Neither permits authorization from a stale snapshot.
A root whose owner existed at the first snapshot is not rescanned; if that
owner disappears and this attempt has no verification for the reference, the
final check rejects it and requires another attempt. A replacement legitimate
source owner can satisfy the check. Such failures before commit append neither
owners nor queue WAL, and introduce no temporary durable pin to recover or GC.
Partial effects of append failures retain the existing recovery semantics.

`ReactivationRace.tla` explores two packs and 165 positive states, including
source release/re-retain, GC mark/unlink, verification errors and verification
finishing after unlink. It checks no dangling restoration, a source at admission,
matching verification identity, no partial restoration and termination under
weak fairness. Four algorithm controls reuse stale snapshots, unlock between
checking and appending, omit the source check or accept an unmatched proof.
One environment-only control weakens catalog serialization. The stale-snapshot,
early-unlock and environment-lock controls select only `TypeOK` and
`NoDanglingRestore` to expose the dangling-reference consequence beyond the
earlier source-premise violation. Reports/configurations record that scope;
positive cases check every property.

Verus proves the entire actual `protocol::reactivation_permitted` loop: permission
is exactly verified, nonempty, and every pack owned and untombstoned. This covers
arbitrary-length status arrays. Catalog-to-snapshot conversion, private proof
construction/binding, I/O, locks and WAL composition lack a complete Rust proof.
Atomic durable catalog steps, immutable extent identities and complete closures
remain premises. This model is not mechanically composed with the byte-scan
model or full crash recovery.

Python regressions pause before/after authentication while independent processes
release, collect, replace protection or perform unrelated writes. They check no
owner/WAL addition on rejected restore and confirm cursors after restart. Native
regressions check all roots before any append, pool/run binding and rejection of
an unverified root whose owner disappeared. Two scan cutpoints join recovery tests.

`Config::retain_reactivated` provides the unlocked path. The lower-level
`Catalog::retain_reactivated` remains a compatibility/control path within the
caller's transaction and therefore still holds the lock during verification.
Coordinator uses the new path. Closure metadata traversal, catalog replay/sync,
final checks and append still hold the lock. Contention/source loss can delay
or fail a restore; no lock fairness, API latency bound or full conformance proof
follows from these tests and measurements.

## Dependency traversal, closure caching and GC

Previously, `Reader::validate_inner` and catalog closure computation recursed
through dependencies. Isolated processes exited with SIGSEGV for valid chains
of 2,048 inline manifests with a 512 KiB stack and 16,384 with the default 8 MiB
stack, in both operations. Heap-backed worklists now pass the 16,384-node chain
with a 512 KiB stack through validation, retain, staging release, reachable-pack
protection and final GC. No depth admission limit was added.

The actual generic `rust/traversal.rs::start/schedule/next` functions are
Verus-proved: schedule an identity/data pair only on first encounter, maintain
unique pending identities contained in seen, and pop the correct pair without
forgetting seen. Contracts cover arbitrary data types and queue lengths under
the explicit `ENV-STRING-KEY` library premise. Production uses these functions.
JSON-to-identity/edge decoding, global closure correctness of the surrounding
loops, file reads and allocation remain outside a composed Rust proof.

`DependencyTraversal.tla` explores all directed graphs and cache subsets on three
nodes, including cycles, sharing and unreachable nodes. Its 15,104 positive
states check exact transitive closure at completion, no repeated expansion, and
termination under finite-graph/weak-fairness premises. Controls omit a successor,
repeat a node or reduce a cached closure to its current node; each must violate
its named invariant. The model checks reference-node reachability; projection to
physical packs and composition with ownership remain adapter obligations. It
does not turn `Ownership.tla` into an arbitrary-graph end-to-end proof. Valid
local ordinals still precede their parent manifest; cycles are extra defensive
model exploration.

The catalog reuses complete cached closures and still checks every resulting
path against permanent tombstones. A cold traversal caches only its requested
root, avoiding a separate suffix closure for every intermediate chain node.
Caching many explicit roots can still consume increasing memory. Validation
checks task/attempt on the root's direct records, preserves independent
dependency identities and returns only the root's direct records. Regressions
cover cold/warm caches, collecting only unreachable packs, another owner's
protection, late-retain rejection and malformed transitive logical references.

`ENV-GRAPH` requires a finite immutable reference graph, complete successor
expansion and complete cached closures; the finite model does not prove actual
JSON/I/O conformance. Heap memory grows with graph, indexes and caches. The fix
removes Rust call-stack growth with dependency depth, not memory growth, JSON
nesting limits or all latency bounds.

## Native streaming payloads: two traversals and one length snapshot

`PayloadStream.tla` separates envelope hashing and actual writing. The input
model permits empty chunks, short/long traversals, changing length declarations
and ignored callback errors. Each pass has at most two callbacks and lengths
0..2: these are TLC exploration bounds, not API limits. The 127-state positive
case checks exact declared/frame/written length on success, no success after
failure, and return under finite-provider/weak-fairness premises. Hash equality
is abstracted to the necessary condition of equal byte counts, not proved byte
authentication.

Regressions against the preceding native code reproduced a declared five-byte
stream returning three bytes twice yet publishing a reference, a provider
swallowing callback failure yet allowing success, and a changing second `len()`
call making frame and envelope disagree. The writer now captures length once,
checks both traversals, rejects an oversized chunk before its write and short
traversals before the footer, and preserves the first callback error. A failure
during the second pass can still leave an unpublished tail; this is not rollback
and does not invalidate previously published extents. Three finite controls
remove length checks, discard callback errors or reread length, each requiring
its named counterexample.

The four actual `rust/stream.rs` functions `start/accept/fail/complete` are
Verus-proved for all `u64` values: exact accumulation without exceeding the
declaration or overflowing, absorbing failure, and completion only at the exact
length without failure. This proves state transitions. The trait/closure
adapter in `payload.rs::visit_exact`, first-Error retention, JSON field access and
actual I/O do not have a composed Rust conformance proof; native integration
regressions connect these boundaries. Providers must terminate: the guard
cannot preempt arbitrary custom Rust and establishes no timeout. See `ENV-STREAM`.

## Verus proofs of production Rust

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustc-dev,llvm-tools
python verification/runners/check_verus.py --download
# Or supply the pinned, already installed verifier:
python verification/runners/check_verus.py --verus /path/to/verus
```

The runner pins Verus `0.2026.10.18.7d82994` and verifies the release archive's
SHA-256. The official [Verus annotation syntax](https://verus-lang.github.io/verus/guide/exec_attr.html)
is enabled with `cfg(straw_verus)` in `rust/bounds.rs`, `rust/protocol.rs`, `rust/selection.rs`, `rust/reclamation.rs`, `rust/stream.rs`, `rust/replay.rs` and `rust/traversal.rs`. The proof harness imports
those files directly; ordinary Cargo builds compile the same executable function
bodies without the proof attributes or any new runtime dependency. Positive
proof inputs, source hashes, tool version and logs are saved in `target/verus`.

Six arithmetic/capacity functions are proved for **all `u64` inputs**, not just the finite
word sizes used by TLC:

| Function | Proved property | Production use |
|---|---|---|
| `add` | Exact representable sum, `None` exactly on overflow | Result/usage aggregation and cached completions |
| `frame_end` | Exact base + frame header + envelope + payload, or rejection | Authenticated index boundary validation |
| `adjust` | Exact addition/subtraction or overflow/underflow rejection | Active-reservation cache updates |
| `clip` | Exact clamped position within a read buffer | Requested-output intersection |
| `verification_capacity` | Exactly `min(length, 1 MiB)`, positive for nonempty records | Whole-payload/chunk verification scratch |
| `next_buffer` | Positive bounded step, increasing cursor, no overrun | Streaming checksum verification |

The production `protocol.rs` adds these proved functions:

| Function group | Proved property | Production caller |
|---|---|---|
| `execute_commit` | Actual effect calls, error propagation, apply quarantine and successful completion under `ENV-COMMIT-EFFECTS`; native effect mapping and unwinding remain separate | `Coordinator::commit` |
| `commit_start/next/advance/finished` | `View` equals the specification; Retain → AppendWal → Apply → ConsumeStaging; failure stops the current attempt; success requires all four successful effects | `Coordinator::commit` |
| `reactivation_permitted` | Verified, nonempty and every pack owned/untombstoned; full executable loop | Refreshed, locked `Catalog::retain_verified` |
| `candidate`, `Indexes::{roots,candidates}` | Enumerates all actual owner sets; candidates are exactly sealed, unowned and untombstoned packs in the index; loops terminate | `Catalog::collect` under its lock; replay-to-index and complete logical-root establishment remain separate obligations |
| `delete_start/next/advance/finished`, `delete_batch_finished` | Only tombstoned packs receive effects; Unlink precedes SyncParent; a failed step cannot report completion; batch success requires every member waiting for sync and successful barriers | `Catalog::collect`, including missing files |
| `reclaim_start/next/advance/finished` | SyncParents precedes RecordReceipt; errors stop the attempt; both successful effects are required for completion | `Catalog::collect` finalization |

A controller success means its adapter reported success for each requested
effect. Verus does **not** prove that an adapter's success implies durable I/O,
or that the unverified dispatcher called the correct operation. The production
callers explicitly dispatch on these effect enums; integration tests exercise
that boundary. Errors can have partial persistent effects and require recovery;
the controller does not roll them back or blindly restart an uncertain commit.

`control_plane.rs` proves retention, release, marking and unlink preserve
`live ⇒ present AND not tombstoned`, then inducts over an arbitrary history.
`candidate_correspondence` connects the executable eligibility specification to
the model's candidate set, conditional on an accurate snapshot. Retained sets
must already contain the full dependency closure. This is not a proof of the
JSON traversal or catalog serialization/locking implementation.

[`retry_liveness.rs`](../verification/protocol/verus/retry_liveness.rs) replaces
the old direct `progresses` premise and its helper theorem. For any natural-number
stage limit, the new theorem uses production `initial` / `advance` Views and
separates Idle, successful/failed effects, Crash and caller Retry events.
`ENV-VERUS-PROGRESS` requires eventual stable effect responses and separate weak
fairness for effect execution and caller retries. The proof derives eventual
rank decrease after stabilization, then completion. A failed attempt has rank
`limit + 1`, restart has rank `limit`, and successful effects reduce it to zero.
There is no bound on preceding failures or elapsed time.

Concrete infinite histories establish both satisfiability (four stages complete
after an error and retry) and the necessity of caller retries (environment
recovery alone leaves a failed attempt stuck). Six semantic negatives break
restart, effect advancement, I/O fairness, retry fairness, error cessation and
crash cessation; reports record the exact theorem filter. Real I/O, lock progress,
valid inputs, cached responses and recovery-to-event correspondence remain
unproved. Filesystem recovery alone does not establish success of every
application-stage effect.

The separate [`gc_liveness.rs`](../verification/gc/verus/gc_liveness.rs) model proves
reclamation across deletion attempts without `progresses`. Starting with a
permanently unowned, durably tombstoned pack, it uses the production controller's
View transitions and models failed I/O with partial effects, crashes, idle and
caller retries. `environment.rs` supplies successful unlink/sync responses,
eventual fault cessation, fair I/O execution and fair caller retries. After
stabilization, rank falls from failed (3) through restart (2), unlink (1) and
sync (0); there is no bound on preceding failures or wall-clock completion.
A concrete history witnesses that the premises are satisfiable. Another infinite
history proves storage recovery alone is insufficient when the caller never
retries. There is no automatic GC retry daemon. The real stat/unlink/fsync,
locking and replay adapters, truthful NotFound, and composition with ownership
remain outside this proof; see the stable contracts in [environment models](ENVIRONMENT_MODELS.md).

An earlier positive run reports **101 verified, 0 errors** (including derived methods,
not 101 public APIs). Seventy-three negative controls corrupt parent-response classification, incomplete cleanup/restart and PID checks, encoding classification, identity-cache lookup/binding/conflict checks, pending sync knowledge, finalization order/completion, publication-hold union/order premises, directory-barrier steps/completion, GC discovery decisions, replay-buffer capacity/cache/durable-prefix guards, reactivation admission, arithmetic/scratch bounds, commit order,
failure handling, tombstone/sync gates, candidate selection, lease-filter deduplication,
model ownership, retry reset, batch completion guards, receipt order/index updates, stream length/failure guards, repeated/lost frontier nodes, or the GC sync/stability/fairness environment.
 Each must produce a semantic verification failure. Four GC model/environment mutations target a named immediate-consequence theorem (recorded in the report); production always verifies the full crate. Launch, parse,
timeout, SMT resource exhaustion and toolchain failures never count as passing negative controls.
The runner copies and hashes every input and rejects `assume`, `admit` and
omitted proof bodies in straw proof files. The separately audited serde_json library module is the explicit exception for opaque external types and twenty-one primitive method specifications (nineteen access/clone/map/index/number/equality methods, a parser and a serializer) plus two narrowly audited external Value encoding/observation laws and one Map key-uniqueness law; it contains no executable straw function. Verus/vstd contracts for standard
library `Vec`, `BTreeSet`, `HashMap` and `HashSet`, the explicit String key/ordering premises, the SMT solver and compilation remain trusted. CI runs both Verus and TLC and retains their evidence.

New integration regressions also cover WAL-durable failures before/after Apply
and before ConsumeStaging: replay keeps one completion and its storage owner.
A possibly partial Apply poisons the owner until recovery. Journal append now
rejects malformed/empty event lists before writing; previously it could
acknowledge a frame that replay would reject. The shared structural predicate is
tested, but JSON framing/replay itself is not included in the Verus proofs.

Neither tool proves physical bandwidth or a latency SLA. Allocation success,
FFI, parsing, cryptography, the full read loop and OS/filesystem internals remain
outside the production Rust proof boundary.

## Applying Anvil's method to straw

[Anvil (OSDI 2024)](https://www.usenix.org/system/files/osdi24-sun-xudong.pdf)
connects executable steps to a state-machine model through conformance proofs,
then proves temporal properties over that model and its environment. Its
[artifact](https://github.com/anvil-verifier/anvil/tree/osdi24) separates executable
and specification interfaces; [verus-tla](https://github.com/anvil-verifier/verus-tla)
provides reusable temporal definitions and lemmas.

For straw, the useful part is the executable-step / model / temporal-property
separation. The code above implements the first slice for commit and GC, using
Verus-native specifications and a small rank proof; it does **not** import the
Kubernetes environment or vendor `verus-tla`. The queue needs per-request
progress, durable acceptance and continuous ownership safety.

Proposed end-to-end properties, **not yet proved**:

- Safety: an accepted submission has one durable receipt; restart preserves it;
  a stale lease cannot change it; retained reader/queue/checkpoint dependencies
  are never collected. Eventual repair is insufficient for lost payload bytes.
- Progress: a persistently eligible task eventually reaches an allowed terminal
  outcome if workers, the scheduler and storage keep making progress. Priority
  queues need an explicit non-starvation condition; weak process fairness alone
  does not imply fairness for each task.
- Reclamation: a sealed pack that permanently loses every owner is eventually
  deleted if collection is repeatedly invoked and filesystem operations complete.
  A crashed writer's open pack is not automatically covered by this property.
- Recovery: after faults cease, a newly fenced coordinator completes replay and
  can resolve uncertain submissions from durable evidence. Faults may be
  unbounded in number before stabilization; safety must hold before it as well.

The trusted boundary must state file/directory sync durability, torn journal
prefixes, locks, fresh opens, failure returns and owner fencing. Failed operations
must not be silently modeled as successful atomic effects. Measurements can test
these assumptions and calibrate performance; they cannot prove them. Bounded
reader retries only guarantee a checked result **or an explicit error** unless
visibility arrives within the budget. They cannot promise eventual success from
unbounded eventual visibility, and no measured p99 supplies such a theorem.

Current status: production arithmetic and commit/GC controller conformance,
unbounded catalog-model safety, conditional controller progress, finite protocol
models and deployment checks are implemented. Whole-API conformance and replay,
lease/receipt uniqueness, consumer transitions, scheduler fairness and
end-to-end recovery/reclamation proofs remain open. Passing this gate is not a
claim that all Rust or all control-plane behaviors have been proved.

## Catalog read-ahead and the append cursor

Replay uses a `BufReader` bounded at 64 KiB, with a `Take` limit equal to the
remaining length observed under the catalog lock. The buffer and shared-cursor
file clone exist only for a nonempty suffix and are dropped on every refresh
return or error. Truncation/sync still use the last fully validated/applied
offset, followed by an explicit seek before appending. The underlying cursor
may have read into the incomplete tail and is not a valid-prefix boundary.

`CatalogReadBuffer.tla` separates prefetch position, consumed bytes, complete
frame boundary, buffered remainder, truncation/sync and append. The separate
`EnvironmentRead.tla` allows arbitrary positive short refills. Positive cases
with a tail, without a tail and with a one-byte buffer have 74/50/53 states. Four
negative controls allow unbounded read-ahead, preserve a buffer across retry,
use the prefetch cursor as a valid boundary or omit the append seek. The model
covers fixed-size authenticated frames, an incomplete suffix and at most one
read failure; that is a search bound, not a runtime retry quota. Arbitrary byte
framing, checksums, catalog-cache/durable-floor recovery and peers are not
composed into this model.

Verus proves the actual `replay::buffer_capacity` equals
`min(remaining, 65536)` for every `u64` input and is zero exactly for an empty
suffix. The 64 KiB cap is an algorithm policy, neither a measured filesystem
constant nor a proved optimum. It bounds read-ahead only; individual JSON
frames, owner/closure indexes and process memory remain unbounded. Standard
library `BufReader`/`Take`, shared-description cursors and actual seek dispatch
remain outside the proof.

Native regressions test every byte prefix of a complete tail after history and
frames exceeding 64 KiB: recover, append immediately, reopen and check sequence
and ownership. Corruption around refill boundaries must not silently truncate
the log. Existing failed-sync/partial-apply/lost-tail tests remain, with another
40 lost/surviving-tail cases across four hosts. [Measurements](FILESYSTEM.md#bounded-catalog-read-ahead-comparison-2026-10-23)
check the reduction in read calls, not a latency bound for arbitrary history.

## Automatic GC discovery and ownership handoff

The old `Config::gc_enabled()` called `storage.log.exists()` every time. Stale
NotFound and I/O errors swallowed by `exists()` both returned false. Then
`catalog()` returned `None`: public retain returned successfully without a
destination owner. Releasing the source afterward allowed another client to
delete the payload. This affects both fresh and previously cached clients and
is separate from the unlink/stat defect.

The actual `discovery.rs::decide` now determines whether probing is needed.
Explicit or remembered GC requirements skip rediscovery. Otherwise a fresh
read-only open establishes presence; non-NotFound errors propagate. Config
clones share the requirement flag, which is reread after probing so another
clone's new requirement is preserved. Auto-discovered catalog initialization
opens an existing log rather than creating an empty replacement. Rust
`Config::gc_enabled` now returns `Result<bool>`; callers must handle errors,
for example `if config.gc_enabled()? { ... }`.

Verus checks the actual decision table for all inputs: explicit/remembered
requirements dominate; Unqueried requests Probe, Present requires GC, Absent
allows legacy and Failed is unavailable. Four semantic mutations forget a
requirement, ignore Present, turn Failed into Absent or skip Probe. Shared
atomic state/memory ordering, real File::open, response classification and the
complete call chain remain outside the proof.

`GCDiscovery.tla` connects discovery, atomic durable retain, source release
and GC deletion. Cold discovery has six states; remembered/explicit/legacy
cases have five each. Three algorithm controls and one environment-only
`EnvironmentCatalog.tla` control must violate `NoLostAcknowledgedData`. These
negative cases select TypeOK and the resulting data-loss invariant to explore
past the earlier ownership violation; positives check every invariant. Catalog
retain is an upper-layer atomic action, not a composed replay/lock/ownership
implementation proof.

`ENV-GC-DISCOVERY` requires pool mode to be established before participants
operate and trusts initial fresh-open NotFound as actual absence of a managed
catalog. Live legacy-to-GC migration is excluded. If initial open can falsely
return NotFound, the environment control shows the new algorithm can still
miss retention. Participants requiring GC should explicitly use
`online_gc=True`; this boundary is not proved away. After automatic discovery,
opening a missing catalog fails rather than downgrading to legacy.

Native regressions cover probe errors, clone sharing, another clone enabling
GC during a probe and disappearance before/after catalog-cache initialization.
Two public Python regressions fail on the old wheel and pass on the new one: a
known missing catalog and ELOOP during cold discovery. Four-host probes check
payload protection after handoff and source protection after open EIO, before
a successful retry. [Evidence and timing](FILESYSTEM.md#gc-discovery-and-handoff-faults-2026-10-23)
distinguish injected faults from natural deployment observations.

## Directory creation, failed barriers and publication

An existing directory can be the side effect of a failed mkdir/sync attempt.
The previous `mkdir` returned immediately when the name existed. A first attempt
could create `pool/raw`, fail its sync, and a retry could create descendants,
sync the payload and its leaf directory, then acknowledge without syncing
`pool`. The `raw` entry was not guaranteed durable. A later crash can make an
acknowledged payload unreachable even when its own bytes were synced.

`mkdir` now calls `create_dir_all`, resolves the path and syncs every directory
from the leaf through the filesystem root, including existing names. It returns
an error on any failed barrier; a retry repeats the complete chain. This applies
to payload directories, catalog initialization and writable journal opening.
No process-local durable-directory cache is introduced. Repeated small writes
therefore perform additional barriers; their cost is measured separately.

The actual `directory.rs` controller exposes the next uncompleted barrier,
advances only on success, keeps failure terminal and accepts completion only
after all barriers. Verus checks its four functions for all u64 counts/states;
four semantic mutations start complete, ignore a failure, repeat an index or
accept an incomplete chain. Path enumeration, `create_dir_all`, `canonicalize`,
the indexing/I/O loop and its correspondence to this controller are not proved.

`DirectoryDurability.tla` distinguishes visible directory entries from durable
parent entries. Creation may leave any prefix on error; a failed sync may or may
not have persisted its effect. A crash removes unreachable entries, while a
later payload/leaf sync cannot persist its ancestors. New, already-existing,
already-durable and one-level cases explore 318, 300, 72 and 90 states. The
four-host chain contains 13 directories including the root; an added Depth=12
case explores 2,730 states. This measured configuration is not a maximum
supported depth. Safety
requires acknowledged data to have a durable path; progress assumes eventual
I/O stability, weak fairness and caller retries. Another positive starts with
a nondurable existing ancestor prefix (312 states). Three algorithm mutations,
a dedicated partial-prefix configuration and a directory-barrier environment
mutant reach acknowledged-data loss. These
negative configurations select TypeOK and NoLostAcknowledgedData to continue
past the earlier path-durability violation; positives check both properties.
The partial-prefix counterexample starts after raw exists but is nondurable,
with no further I/O errors: the old policy persists child entries 2 and 3, skips
entry 1, acknowledges publication and loses reachability on crash. This matches
the missing retry barrier observed by the probe while abstracting exact syscall
order and complete publication.

`ENV-DIRECTORY` assumes a valid finite directory chain, stable root/identities,
stable symlink aliases and mount topology, and no external directory replacement
or deletion during use. All resolved ancestors must be accessible for sync,
including ancestors outside the pool. Canonicalizing the target does not prove
the durability of externally managed alias entries. The finite model is not a
composed proof of the filesystem namespace, publication framing or full queue.

Native regressions exercise failure at every ancestor barrier, full-chain retry
for existing names, resolved-parent enumeration and rejection of an existing
regular file. The public API probe logs actual directory fsyncs and injects EIO
on one private `raw` directory. Missing/repaired barriers are observed; actual
power loss and lost data are not. [Measurements and artifacts](FILESYSTEM.md#directory-durability-and-publication-cost-2026-10-23)
keep those claims separate.

## Uncertain publication, conflicting retry and GC

An interrupted publication keeps a private `publication:<writer>:<id>` holder
for external dependencies. Previously a retry replaced that holder before
checking identity/content. Retrying with empty or different dependencies could
return `IdempotencyConflict` after removing the original protection; GC then
actually deleted the source pack needed to finish the uncertain publication.
The destination had not returned success. This is a straw ownership-adapter
bug, reproducible without violating catalog serialization or sync guarantees.

`publish` and `record_set` now accumulate the union of prior and incoming roots
in this internal holder. A successful staging handoff precedes releasing it.
Ordinary public `retain` still replaces its named owner set. Failed or abandoned
attempts may keep extra roots indefinitely; no expiry or automatic abort cleanup
is introduced, and the catalog format is unchanged.

Verus proves the actual `holding.rs` loop preserves exactly the union for
arbitrary input sizes, conditional on `ENV-STRING-ORDER`. The loop invariant
preserves every old root and incorporates each processed addition. Two semantic
mutants forget old roots or ignore additions; an environment mutant removes the
String ordering premise. Closure calculation, BTreeSet-to-Vec conversion,
serialization, append durability and the full publication adapter remain
unproved. This is an incremental production-Rust proof, not a proof of the
entire new `Catalog::hold` method.

`PublicationRetry.tla` checks three finite configurations (42, 6 and 42 states):
changed roots, an empty retry and a two-pack original closure. Catalog actions
are atomic/durable and the original hold already exists. GC interleaves between
protocol steps. Three mutations replace the hold, release on rejection or
release before staging; each must violate `NoLostDependency`. Those negative
configurations select TypeOK and the data-loss invariant to continue beyond
the earlier ownership violation; positives also check preserved holds,
completed handoff and eventual completion. Progress requires a correct retry
and weak fairness. The model does not compose uncertain catalog-frame recovery,
closure expansion, pack sealing or all publication failures.

Four public API regressions fail on the preceding wheel and pass on the fix.
[Four-host observations](FILESYSTEM.md#publication-retry-and-gc-2026-10-23)
separate actual dependency deletion, callback-induced uncertainty, successful
recovery and manual cleanup of abandoned baseline fixtures.

## Standalone publication completion and GC

With `segment_target_bytes=0`, the previous `write_records` path registered a
file as open and staged its reference but never sealed its catalog entry.
`record_set` used that path for its manifest, so both data and manifest could
remain open. Explicit `seal()`, `close()`, ownership release and reopening did
not repair the entries because standalone files were absent from the current
packed-file slot. `publish` already sealed its output. This was permanent
retention of successfully completed, unowned files, not early data deletion.

A common finalization adapter now stages roots, then seals standalone output
before returning. `write_records` and `publish` use it directly; `record_set`
inherits it through its internal manifest write, then establishes the final
result's staging before consuming temporary record staging. Packed output only
stages and remains appendable. Every retry starts the two idempotent operations
again, so a seal error may have already persisted without causing duplicate
output. In the publish path the operation hold is released after finalization.
No catalog-format change or automatic repair of historical open entries occurs.

The actual `finalization.rs` controller exposes a `View` and executable
start/next/advance/finished functions. Verus proves agreement with Stage/Seal
steps, bounded progress, terminal failure and completion requiring the appropriate
one or two steps. Four semantic controls seal before stage, skip sealing, ignore
failure or accept an incomplete standalone result. The surrounding I/O dispatch,
root/segment extraction and the complete publication adapter remain unproved.

`PublicationFinalization.tla` starts after durable data and open registration.
Failed staging/sealing can have either durable outcome, retries repeat the
sequence, and GC can run between steps. Standalone unstable/stable configurations
have 24/6 states; packed configurations have 18/6. Four negative controls expose
unreclaimed released files, deletion before staging, ignored seal failure or
prematurely sealed packed output. Negative configurations select TypeOK plus
their named consequence, allowing exploration past earlier invariant failures.
Positive configurations also check eventual reclamation under eventual I/O
stability, weak fairness, caller retry and eventual explicit release/packed close.
This finite model does not prove arbitrary multi-file publication, whole-catalog
recovery or backend object deletion. Existing lower-level GC models are separate.

Eighteen API regressions cover all three APIs, packed/standalone mode and
before/after seal or catalog-sync failures. The old wheel fails the two
standalone completion checks; its publish and packed controls pass.
[Four-host and 100 GB evidence](FILESYSTEM.md#standalone-finalization-and-gc-2026-10-23)
records the binaries, ownership states, real file sizes and cleanup separately.

## Complete file, uncertain sync and same-writer retry

The standalone path previously assigned pending state only after file sync and
its after_data_sync callback. A failure at either point forgot an already
complete private file. The same-ID retry wrote a new UUID file and left the old
partial/catalog-open entry indefinitely. Packed mode already remembered before
sync. This wasted complete-file writes and defeated the live writer's retry
fence; it was not a failure of a successful filesystem durability guarantee.

Both modes now store the complete description before the initial file barrier.
The actual generic `Pending<T>` preserves the identity/content/reference/path/
record-count tuple and starts with unknown sync. A successful response confirms
it monotonically. Standalone retry syncs the original partial (or its same final
path after a prior rename) when that result remains unknown, then authenticates
the complete extent and finishes directory persistence. Even a known successful
sync does not skip retry authentication. Packed retry retains its prior full
inspection/resync behavior. Retries do not write payload chunks again, although
input hashing and the full checksum scan still cost work proportional to size.

Verus proves start/record_sync/needs_sync on the actual generic Rust bodies:
exact description preservation, unknown initial state, monotonic confirmation
only from success, and a sync requirement whenever success is not known. Four
mutants violate these contracts. Surrounding Option assignment, response
classification, path/descriptor binding, syscall ordering and byte validation
are not proved by these functions; TLA+ and implementation probes provide
separate evidence for those boundaries.

`PendingPublication.tla` checks before/during/after-sync errors for standalone
and packed modes, plus corruption with unknown and confirmed sync. File-sync
failure may persist its effect. Nine finite positive configurations check one
completed file, fencing, durable authenticated returns and eventual success or
corruption rejection under fair retries/eventual stable I/O. Four algorithm
controls remember too late, trust failure, skip a required barrier or skip
validation; a separate environment-only control makes successful file sync lie.
Negative cases select TypeOK and their named consequence. This does not prove
process-restart recovery, missing-path/rename progress, failures before a complete
frame/description exists, arbitrary retry input semantics or final catalog/GC
composition. Historical abandoned partials are not automatically repaired.

Six standalone regressions fail on the preceding wheel while six packed controls
pass. Sixteen fixed-wheel cases also reject corrupted bytes and check no retry
payload writes, correct sync counts and final GC. [Four-host evidence](FILESYSTEM.md#pending-files-and-sync-errors-2026-10-23)
uses actual fsync interception as well as callback failure after reported success.

## Namespace identity and GC protection

A readable in-pool symlink reference could be successfully retained under its raw
path while GC tracked the same sealed file under another path. Releasing original
staging then allowed deletion despite the reader owner. Catalog ownership closures,
pack entries, tombstones and receipts now use resolved pool-relative path identities.
Replay also normalizes legacy spellings, including dangling aliases after deletion.
New events store normalized keys; existing log bytes and public references are unchanged.

[`NamespaceIdentity.tla`](../verification/storage/tla/NamespaceIdentity.tla) consumes
`ENV-NAMESPACE` through `EnvironmentNamespace.tla`. Five positive cases cover
canonical/reader/writer aliases and legacy owner/tombstone replay (10/10/10/9/5
states). Six algorithm controls omit normalization at owner, pack, replay, fencing,
tombstone or receipt boundaries; the named properties must fail. A seventh control
changes only the environment's identity response and violates `NoLostOwnedData`.
The model has one physical file, three names and serialized durable catalog steps.
Its eventual-reclamation property assumes weak fairness and an explicit owner
release; it does not compose arbitrary graph closure, byte replay or I/O failures.

The two actual functions in [`rust/identities.rs`](../rust/identities.rs) are directly
included by Verus. Under `ENV-STRING-KEY`, lookup returns the stored identity and
remember inserts an absent binding or preserves an existing binding, rejecting a
conflicting value. Three semantic mutants must fail. These proofs do **not** prove
the path resolver, prefix construction, truthful readlink responses, cache call
sites or catalog-to-model correspondence. Cache hits do not detect external
retargeting; stable bindings remain an environment requirement.

Twenty Python regressions fail on the old wheel and pass on the final wheel;
two native tests cover missing/dangling paths and all four legacy catalog fields.
The Python cases include small extents at sparse offsets across 2 GiB, 4 GiB and
2 TiB, not physical TB transfers. Four-host alias/legacy/cross-client evidence and
cold replay cost are in [FILESYSTEM](FILESYSTEM.md#namespace-identity-and-gc-2026-10-23).
Bind-mount/inode equivalence, arbitrary namespace changes, mixed old unsafe GC
collectors and repair of already deleted data remain outside the guarantee.

## Catalog encoding and replay work

New events normalize all owner, pack, tombstone and receipt keys before writing
`path_encoding=1`. Replay checks type/version before applying any field, validates
relative-path syntax and uses version-1 identities directly. Legacy entries still
resolve their names; existing logs are not rewritten. This removes per-path replay
lookups for new events while retaining the alias ownership repair.

[`NamespaceEncoding.tla`](../verification/storage/tla/NamespaceEncoding.tla) has new, legacy
and unknown-encoding positives (11/6/2 states). Four negative controls omit one
normalization, trust legacy aliases, accept an unknown encoding, or unnecessarily
resolve certified keys; they must violate `CertifiedKeys`, `DecodedIdentities`,
`UnknownRejected` and `NoResolvedLookups`, respectively. The last is a work-count
property, not a wall-clock bound. The four-field model is not a full graph/GC/crash
composition; weak fairness supplies conditional eventual decoding.

Verus directly proves `identities::encoding` distinguishes missing, version 1 and
all other u64 values. Three classifier mutants fail. It does not prove JSON type
handling, producer normalization, framing or the replay adapter. Native tests pass
raw aliases through the writer, reject escaping paths before catalog I/O, and
replay receipts after deletion. Twelve Python cases check all new path fields,
eight invalid marker forms and three legacy alias recovery phases. The preceding
binary fails the nine new-encoding assertions and passes the three compatibility
cases; those failures describe the added protocol, not a newly discovered bug in
its former encoding contract. All prior alias and sparse-offset regressions remain.

The first full Python run exposed a separate test race: a 50 ms restoration
timer could expire before a delayed reader began, so the test never exercised a
stale view and then failed its retry-log assertion. An 80 ms pre-read pause
reproduced that gap. The fixture now forwards native stderr and restores only
after observing the first retry; cleanup also restores bytes if the operation
never reads. A regression checks that postponing the reader does not remove the
fault. This is test synchronization, not evidence of a filesystem time bound.

## Incomplete-write cleanup and writer reuse

Before a complete extent description existed, the old implementation returned
write errors directly. Standalone mode left open catalog entries and `.partial`
files that normal GC skipped. Packed mode left dead tails between acknowledged
prefixes and successful retries, pinned for as long as the older data remained owned.

After successful open/positioning and before the first byte, `write_prepared`
saves the private description and original length. It clears that state only on
handoff to complete-file pending sync. Failed packed writes truncate to that exact
length and sync the file. Standalone failures unlink their partial, sync its parent,
then seal the never-exposed final name. Cleanup failure retains the description
and blocks new writes/seal; the next call restarts the idempotent effects. Peer GC
may have permanently tombstoned a name after an uncertain persisted seal, which
satisfies this private retirement retry. General pack registration still rejects
tombstones.

Verus directly proves the actual `incomplete.rs` functions (seven added checks,
including derived obligations): immutable description,
restart, effect order, failure stopping, completion and PID equality. Six semantic
mutants must fail. Python writer mutation/lifecycle, metrics and catalog methods
check PID before inherited mutex acquisition; a child must neither wait for a lost
owning thread nor clean its parent's private tail. That guard placement and real
syscall dispatch remain outside the Verus proof.

`IncompletePublication.tla` starts after failed writing with an accurate process-local
description. It distinguishes current/durable lengths and names, allows failed effects,
peer GC and storage crash. Three positive configurations explore 252/444/136 states,
checking prefix length, fencing, durable cleanup before reuse, seal order and
idempotent retirement. Eventual stability, weak fairness and caller retry imply
reusability. Five algorithm controls and one truncate-environment control violate
named properties. Length abstracts acknowledged contents: byte preservation is
supplied by `ENV-INCOMPLETE`, not proved by a length invariant.

All 42 added Python cases fail on the preceding wheel and pass on the fixed wheel:
24 API/mode/failure combinations, eight cleanup/peer-GC boundaries, three sparse
large offsets and seven fork entry points. Sparse starts at `2**31-5`, `2**32-5`
and `2**41-5` exercise exact u64 offsets/prefix preservation, not physical TB throughput.
Four-host probes compare real 2 KiB/8 MiB payloads and inject EIO before/after actual
syscalls; see [measurements](FILESYSTEM.md#incomplete-writes-and-cleanup-retries-2026-10-23).

Process death loses the descriptor. Abandoned publication holds and historical
garbage can still leak safely. Fresh registration/create/seek failures are covered
by the separate preparation guard/model described below. Storage-crash actions retain a live client and do not model writer restart.
Complete lock/catalog/byte-I/O refinement and recovery are unproved. This change
does not scan or automatically seal abandoned writers, repair old leftovers, or
introduce a TTL-based reclamation rule.

## Missing parents and GC completion

Writers register open packs before directory creation. Historical writers could
seal an empty reservation with no parent after failed creation and normal close.
Current fresh-setup cleanup repairs its own parent; GC still accepts old states. The old
collector accepted unlink NotFound but could never open/sync that parent, so every
collection failed without a receipt. Other files in the same batch might already
be deleted: a failed call is not rollback.

GC now distinguishes opened, missing and other-error responses. Existing parents
retain one fsync. Missing parents use the existing mkdir/sync helper before receipt.
Errors preserve tombstones/pending indexes for same-client or new-client retry.
Verus now checks the actual `gc_io::classify` and `sync_parent` dispatch as well as
the ordered `directory_io::sync_all` loop used by mkdir. Semantic controls reject
misclassified I/O errors, omitted/failed barriers and skipped repair. Complete
path enumeration, native handle binding and aggregate repair durability remain open.

`GCParentRepair.tla` starts with a permanently unowned, tombstoned name. It explores
actual missing parents, fault-period false missing responses, partial creation,
sync errors, crash and retry. Four positive configurations have 8/38/5/28 states;
receipts require durable absence and stabilization/fairness/caller retry imply
completion. Two old-policy configurations, including one starting with no faults,
produce infinite retry cycles violating **the temporal property `EventuallyRetired`**.
This is not a synthetic safety error flag for nonprogress. Two algorithm controls
and one sync-environment control violate `ReceiptIsSafe`.

Temporal negative controls require exactly one configured PROPERTY, TLC exit 13,
the named temporal violation and a looping/stuttering counterexample. Safety
negatives still require exit 12 and the named invariant. Parsing failures,
timeouts and wrong property names never pass. Prior model inputs/results are retained.

Eleven of 12 added Python cases fail on the preceding wheel; its healthy existing-parent
control passes. The fixed wheel passes all cases: missing prefixes, real 2 KiB/8 MiB
records, protected roots, interrupted repair, fresh-client replay and no repeated
payload I/O after completion. Four-host repair calls execute 12 directory barriers
including root, motivating an additional `DirectoryDurability` Depth=11 fixture.
This measured edge count is not a supported-depth limit or a composition proof.

`ENV-GC-PARENT` exposes creation permissions, idempotent mkdir and stable directory
identity. Durable file absence does not promise durable existence of empty placeholders.
External replacement/rebinding of old directories is excluded. New directory actions
are not mechanically refined to the unbounded GC liveness theorem. Crashed/unclosed
writers' open entries, early standalone registration failures and historical leaks
remain outside this recovery path.

## Registration and open failures before the first payload byte

The preceding wheel loses a fresh name when catalog registration reports an error
after taking effect: even normal close leaves an open reservation. A public-API
reproduction covers both packed and standalone modes at three complete-frame/sync
cut points; three subsequent GC calls cannot retire it. This is a live-writer
bookkeeping bug, separate from the crashed-writer boundary.

`Store::write_prepared` now saves a new private name before registration, then
registers, creates/opens and captures the offset within a fallible setup scope.
Failure drops any opened descriptor before cleanup. Unknown setup outcomes use
unlink, parent repair/sync and sealing; errors retain the guard. An existing pack
is never removed by this setup path. A successful setup preserves the identity
and captures its rollback offset before the first header byte.

Verus proves the actual generic `preparing` and `capture_prefix` bodies, including
identity preservation, exact u64 prefix and mode selection. Three added semantic
mutants must fail. Real guard placement, seek/open behavior and complete
Store/catalog dispatch remain unproved.

`PublicationPreparation.tla` has stable/unstable positive configurations (6/54
states), a legacy lost-reservation counterexample, two cleanup/order negatives
and an unlink-environment negative. It checks conditional resolution under
eventual recovery, weak fairness and live caller retry; it does not prove
writer restart or compose the complete-file, ownership and directory models.

All 34 added regressions fail on the preceding wheel and pass on the repaired
wheel. Twelve exercise uncertain registration plus close/retry; seven setup
cut points, three existing-prefix cases, ten cleanup/peer-GC cases and two mkdir
obstructions cover the adapter. Some old failures are because new fault hooks do
not exist in that binary, not independently observed old bugs. The twelve
missing-parent GC regressions reconstruct the historical empty missing-prefix
state after current setup cleanup repairs its own parent.

## Complete-publication handoff and request admission

The production generic `Handoff<T>` in `rust/finalization.rs` preserves the exact description and API. `handoff`, `allows`, `acknowledge` and `same_api` prove initialization, request admission, preservation through acknowledgement and enum equality. Positive function bodies are unchanged for verification; no assume/admit is added. Four semantic negatives admit other requests, ignore API, start inactive or fail to acknowledge. Actual String matching, call placement, JSON/content hashing and full Store/catalog adapters remain unproved.

The runtime installs this state after constructing a complete description and before sync, retaining it through pending sync, staging, standalone seal, record_set outer staging/raw-owner consumption and internal hold release. Pending checks precede cache hits. Other API/ID requests and closed writers are rejected before creating dependency holds. record_set uses the internal prepared write to avoid premature acknowledgement by the public write_records wrapper.

`PublicationHandoff.tla` has six positive cases (three APIs, two modes), five algorithm negatives (legacy, early acknowledgement, forgetting after errors, ignoring API, skipping release) and one environment-only lying-stage negative. It checks safety and conditional liveness. The old counterexample demonstrates lost retry identity/retention, not loss of acknowledged data. See [ENV-HANDOFF](ENVIRONMENT_MODELS.md#from-a-complete-file-to-a-successful-return) for environment and composition limits.

There are 43 new Python regressions: 40 sync/catalog/seal/internal-handoff cutpoints, one reclaimed-cache replay and two closed-writer checks. The old wheel fails 42 and passes the replay control; the fix passes all. An initial 106-test focused run had three fixture assertions that overlooked legal partial-to-sealed renaming; the corrected run passes. These are not three additional product defects. An initial Verus attempt could not connect derived enum equality to its contract; the actual exhaustive `same_api` implementation discharges it. Both earlier logs remain separate evidence.

## Durable dependency graphs for staging consumption

The production loop `rust/consumption.rs::covered` proves success exactly when every child key is present in the existing graph or same-frame additions, under the explicit String key law. It also proves bounded iteration without substituting a positive function body. Three semantic negatives accept missing nodes, ignore additions or skip scanning. Actual consume reuses the already verified generic frontier. Reference/key binding, JSON, truthful edges, call order and atomic frames remain adapter obligations; the complete consume method is not proved.

`StagingConsumption.tla` models one lost complete-release response, root GC, an independent reader preserving a leaf, leaf restaging and fresh replay/retry. Positive depths 1, 3 and 12 have 2, 6 and 24 logical nodes. It checks persistent graph closure/exactness, reader safety, complete staging consumption at success and conditional eventual return. Legacy omission and replay loss produce temporal counterexamples; missing definitions and skipped cached edges violate safety invariants. An environment-only missing-edge mutant violates `ConsumeComplete`. Actual shared-pack availability is not refined to these nodes.

The 61 new Python regressions cover six catalog cutpoints, three APIs/two modes, shared roots, linear direct edges, restaged dependencies, read errors, legacy logs and malformed graphs. The old wheel fails 44 and passes 17; new-format/hook checks are not 44 distinct old defects. A native regression checks simultaneous graph/owner rebuilding after an unconfirmed frame is lost. The initial repair also persisted direct raw roots; four 1/512-record cost assertions fail there. The final adapter separates raw roots from manifest traversal, with regressions for a raw root also reached by a manifest and an empty manifest. This adapter optimization leaves Verus/TLC inputs unchanged; their full reports are reused after exact input-hash checks, while final Python/native suites are rerun. The first focused TLC run passed three positive cases but its runner rejected combining a temporal negative with an invariant filter. An intermediate run removed the released-root-recipe invariant to satisfy that restriction. Final review restores every invariant for positive cases and the replay temporal negative. The runner now permits explicit, reported temporal invariant selections that retain TypeOK and are subsets of configured invariants. Only the legacy temporal case omits ReleasedHasRecipe, retaining the other five invariants to isolate its progress failure. A final full TLC rerun validates the stronger configuration; earlier logs and archives are preserved separately from product defects.

## Dependency-owner cleanup after preparation failure

`publish`/`record_set` establish internal `publication:*` holds before validation/writing. Keeping dependencies after failure for retry is an existing contract; the defect was that a live writer did not remember these preparation owners and could not clean them even on explicit close. The fix records each owner before hold without overwriting previous failed requests. Only the completed request transfers its owner to handoff responsibility. Explicit seal/close cleans other unfinished preparations; internal pack rotation preserves them.

Cleanup moves one owner into a retryable current slot and clears it only after successful release. Errors preserve the identity; the next operation finishes started cleanup first. Verus proves the actual generic Rust `holding::finish` clears its Option only when the caller-provided success flag is true and otherwise preserves the exact value. Two semantic mutants clear unconditionally or never clear. The pending set, call placement, real durability binding and complete Store methods remain unproved.

`ProvisionalHold.tla` has 74/68/26 states for hold/build/complete positives. Properties cover tracked owners, preparation/complete-handoff protection, no final abandoned hold and conditional completion. Seven negatives cover legacy missing identities, late capture, forgetting failed cleanup, premature complete-handoff cleanup, rotation cleanup, eager release on failure and later requests overwriting earlier preparations. Available second-request input, atomic catalog actions and weak fairness remain abstractions; arbitrary graphs/shared packs/process death are not composed.

The 81 new Python regressions have 68 baseline failures and 13 passes; absent new hooks do not count as independent bugs. All 247 new/related tests pass, covering multiple requests, original retries after source-staging release, cleanup errors, seal/close and uncertain complete-handoff protection. Explicit seal/close abandons incomplete preparations, after which their original retries are not guaranteed. Complete uncertain handoffs remain protected.

The first repair treated early error as abandonment. Its focused tests and narrower model passed, but the full suite caught failure of the unchanged `test_incomplete_destination_publication_keeps_dependency` (676 passes, one failure). This exposed an omitted existing retry contract in implementation/model; the old regression was preserved. The final version keeps failed preparations and adds an `eager_error` negative. Intermediate binary/model/measurements/failure logs are archived separately and excluded from final evidence. An earlier invalid test run loaded the system wheel with the same version; private installation and native-path verification corrected the harness. Initial Verus attempts used unsupported runner filtering and then caught an Option-reference postcondition type error; correcting the contract proved the unchanged function body.

## One-record multi-host reads and uncertain handoff composition

`SharedRecordWorkflow.tla` starts from an already durable source record and connects preparation hold, failure before complete output, source-staging release, multiple readers, original handoff, close of another failed preparation, lost consumption response, GC and fresh-client release retry. One payload file and one manifest are abstract nodes, not an expansion of 100 GB into TLC bytes. ReaderCount 1/2/4 positives have 15/17/29 states; four readers with failures allowed have 44 states.

`EnvironmentRead.tla` complete/short/error outcomes abstract a reader's entire assigned slice as eventual success or failure, without expanding the internal five retries. Reader failure terminates this workflow while preserving its preparation owner; reclamation after that error is not guaranteed. Only full success removes a pending reader, and all must finish before continuation. Error-free configurations check eventual done; the failure configuration checks eventual done or failed and protection in failed states. Weak fairness applies to response actions; **real syscall/network request termination remains unproved**, and measured p99 does not establish it.

Six algorithm controls clean failed-reader dependencies, eagerly clean failed preparation, bypass the reader barrier, omit the result owner, leak the owner on close or omit the consumption recipe. The last must produce a genuine temporal counterexample. An environment-only edge-omission control must violate `ReleasedHasRecipe`. This composes specific stages' abstract ownership contracts, not mechanical refinement of byte framing, real locks, call placement, full crash recovery or arbitrary multi-file graphs. The new native probe is execution evidence, not Verus-proved code.

## Busy writer access and callback wait cycles

A publication keeps the native writer mutex while invoking Python fault hooks. Previously `metrics`, `seal()` and `close()` waited for that same mutex. A hook that called one of them could never return; the equivalent two-thread case blocks when the writer's hook waits for the caller to signal continuation. The fix makes all six native writer access sites use one nonblocking helper. Busy access raises `ResourceLimitExceeded`; poison raises `RuntimeError`; rejected accesses cannot close/seal or alter the active writer. Independent reads and catalog operations do not acquire this writer mutex.

`WriterAccess.tla` has seven states for both normal and poisoned-later positive configurations. Its blocking-algorithm negative requires a genuine `EventuallyFinished` temporal violation (three states); mutating a busy writer violates `BusyPreservesWriter` (three), and admitting poison violates `PoisonDenied` (six). An environment-only change falsely grants a second token and violates `ExclusiveAccess` (three). The positive progress scenario catches the busy response and returns normally from the callback, as the probe does. Propagating that exception through publication abort/cleanup is a separate path. The model abstracts the rest of I/O into finishing steps under weak fairness; it proves neither real I/O termination nor arbitrary callback behavior.

Verus checks the unchanged production body of `rust/access.rs::decide` with generic tokens and arbitrary poison flags: successful admission preserves the exact supplied token, absent tokens yield Busy, poison yields Poisoned. Three semantic mutants ignore poison, deny a valid token, or misclassify busy. The `std::sync::Mutex::try_lock` call, token mapping, Rust guard destruction, GIL/FFI and whole Python adapter are explicit remaining correspondence obligations. The actual integration test checks reentry and a paused writer on another thread, 2 KiB/8 MiB records, standalone/packed files, unchanged committed prefixes, subsequent publication and final GC. This is not a claim of general callback reentrancy through other locks.

## Staging cleanup after accepted transactions

The queue retained roots, persisted its WAL, applied state and then released staging. A failure in the last step left same-ID retries returning business-cache results; fresh recovery replayed business state without cleanup. Cancelling the task therefore left a staging owner. The accepting frame now records `StagingHandoff`; live and recovered pending work must finish before new operations or cached responses. The catalog persists the receipt with release and dependency recipes in one frame.

Retries alone are insufficient: after a durable release loses its response, a publisher may restage the same reference. Historical handoffs must recognize completed IDs instead of releasing that new protection. Recovery shares one catalog refresh/critical section for historical receipt checks; completed IDs neither read manifests nor append frames. Incomplete IDs require their queue owners before cleanup. Recovery rejects misplaced, repeated, incorrectly bound or rootless markers. Discarded unconfirmed replay rebuilds the receipt index together with owners and recipes.

`CommitCleanup.tla` has eight positive configurations for errors before/after release, success, live/recovered retry, subsequent staging and an independent reader. Seven algorithm controls cover legacy cleanup loss and forgotten replay receipts as two true temporal counterexamples, plus dropped pending work, absent receipts, repeated consumption, receipt-only persistence and premature pruning. The environment-only false receipt violates `CompletedConsumption`. The model begins with one retained root; atomic WAL/catalog frames remain adapter contracts without full multi-queue/graph/recovery refinement.

Verus checks actual generic `rust/consumption.rs::pop_completed`: confirmation removes exactly the last entry, while error preserves the Vec. The actual `record_receipts` loop yields the union of prior and supplied receipts under String key laws. Four semantic mutations discard work on error, never finish, omit new receipts or forget old receipts. The verifier reports 115 checks, not 115 fully verified Rust methods. Confirmation flags, hash/JSON binding, call placement, file/directory durability, locks and complete Coordinator implementation remain unproved.

37 added Python regressions cover failure cuts, dependencies, restart, cached-result/GC barriers, later staging, collected-manifest history and malformed receipts. Native regressions check marker identity/order/legacy handling and unconfirmed index rebuild. Old coordinators reject the new WAL marker with `UnsafeRecovery`; new coordinators recover legacy logs without automatically consuming ambiguous staging. See the separate [environment obligations](ENVIRONMENT_MODELS.md#committed-cleanup-and-subsequent-staging).

## Actual queue recovery with large-record readers

The native `queue_record_probe` drives a real tensor record and manifest through production Coordinator calls: error after acceptance but before staging release, close/reopen, real accepted/cancelled history, four input leases, cancellation and another recovery. GC must retain every reader's input. Four hosts read disjoint complete checksum chunks; the supervisor acknowledges successful readers individually and invokes GC after each. Only the final acknowledgement permits data/manifest reclamation. Subsequent recovery must check historical receipts without opening manifests or appending catalog frames.

`QueueRecordWorkflow.tla` has seven positive configurations for 1/2/4 readers, 0/128/1024 historical handoffs and possible reader failure. Seven algorithm negatives cover lost cleanup intent, a missing final acknowledgement and forgotten replay receipts as three true temporal counterexamples, plus cancellation/recovery dropping readers, early pruning and cleanup of a failed reader. Two environment-only controls permit premature acknowledgement or falsely report a handoff receipt. The model explicitly records each historical handoff ID and checks every receipt during reader recovery and recovery after GC; a history counter alone would not establish these obligations. This composes a particular abstract workflow, not a mechanical refinement of the entire control/storage implementation.

Production Rust/Python is unchanged this iteration. The added Rust source is a measurement program, not Verus-proved code. Production proof inputs are identical and Verus is rerun in full. Prior 754 Python / 38 native integration results are reused only after checking production sources, tests, Cargo.lock and the manifest's dependencies/features/library configuration; the added example is separately built, executed and linted. History-recovery time, peak RSS and payload throughput remain separate [FILESYSTEM](FILESYSTEM.md) measurements, not environment deadlines.

## Large-record range copying and authentication

`RangeRead.tla` adds finite protocol exploration of the range-read loop: each checksum chunk is scanned through bounded buffers, output privately accumulates only the requested intersections, and return follows complete authentication. Distinct position labels expose offset mistakes that repeated content could conceal. Configurations cover short final chunks, a chunk larger than the file, single-unit buffers and every empty/nonempty interval in the configured length, plus persistent corruption and short/error read responses.

Seven positive cases check safety and termination; no-spurious-error plus termination excludes an always-reject strategy. Nine algorithm controls cover unbounded scratch, copying the whole buffer, confusing relative/absolute offsets, skipped authentication, skipped final chunks, early return, accepted I/O errors, rejection of valid input and a true temporal counterexample that stalls at authentication. An environment-only authentication mutation must violate `NoCorruptReturn`. This is not mechanical refinement to `read_frame_range`.

The [position-dependent-data record](../verification/measurements/20261024-positioned-range.json) maps each global 64-bit word position to different content and computes distinct chunk hashes. Four native readers check every returned byte in addition to production authentication. An independent Python implementation checks complete small-workload bytes/checksums and the prefix/suffix of every remote request. Actual 100 GB and earlier sparse TB-offset evidence remain separate. Production Rust and Verus inputs are unchanged; the new measurement example is outside production proof coverage.

## Fixed tensor granularity and actual Rust span proof

`TensorGranularity.tla` checks fixed sizes for new records, complete digest metadata, authentication using encoded sizes, empty ranges and the new-record byte bound. Lengths 0–13, base chunks 1/2/3 and a legacy three-digest budget keep exploration small; they are not production quotas. Six algorithm controls reintroduce growing new chunks, reinterpret legacy metadata, skip the prefix, include an exclusive endpoint, read payload for empty ranges or truncate the digest list. The old policy violates the newly specified cost bound; that does not establish a corrupt-return defect.

Production `Config::read_frame_range` calls `rust/bounds.rs::authenticated_span`, whose same executable body is verified by Verus. For all u64 inputs it classifies valid ranges/nonzero chunks and computes complete touched-chunk boundaries clamped to payload length without overflow. Extra authentication is at most `2 * (chunk - 1)` bytes; a one-byte request needs at most one chunk, and empty ranges need none. Four Rust semantic controls break prefix alignment, the exclusive endpoint, tail clipping and empty ranges and must be rejected. The modulus lemma comes from the pinned Verus release's vstd, with no added unproved axiom or omitted body.

The Python producer selects 4 MiB. Python code, metadata/checksum binding, JSON decoding, actual seek/read and the complete copy loop are outside this arithmetic proof; `RangeRead.tla` remains a separate finite model. The per-attempt payload bound excludes envelope/index work, retries and physical object reads. Legacy records use the bound for their encoded size. Tests cover more than 512 digests, cross-policy same-ID conflicts and original-metadata retries, legacy reads and corruption outside the returned interval within a touched chunk. See the [measurement and verification record](../verification/measurements/20261024-tensor-granularity.json).

## Typed-reference validation order

The old path compared `TensorRef` fields only after native payload return. A bad 64 MiB reference with 16 MiB of additional reader address space failed allocation before reporting the typed mismatch, while a bad shape's empty slice succeeded. The fix binds dtype, shape and length between authenticated-envelope validation and payload allocation, and checks empty slices too. Python checks the local shape/byte-count relation before issuing the native request.

L1 `TypedRead.tla` explores valid intervals at lengths 0/6 and four layouts, including allocation failures. Properties cover no payload work for mismatches, matching successful returns, error classification, no spurious typed errors, empty payload work and termination under weak fairness. Three positive cases, five algorithm controls (one true temporal counterexample) and one decoding-environment control exercise these obligations. Small integers bound exploration; the experimental 16 MiB memory margin is not an allocator contract.

L3 production `rust/typed.rs::matches` returns true exactly when byte lengths, dtype strings and every shape dimension agree, with no fixed rank bound in the loop. Five semantic controls ignore length/dtype/rank/dimensions or reject a valid descriptor and must fail verification. Python shape products, JSON-to-Layout mapping, call placement, full I/O and numeric timing bounds remain outside the proof. Byte authentication/copying still depend on the separate `RangeRead` model and integration tests; a matching layout never permits skipping checksums. See the [measurement and verification record](../verification/measurements/20261024-typed-preflight.json).

## Reusing a descriptor within a read attempt

`ReadAttempt.tla` explores ordinary, cold-session and warm-session reads, independent missing regions and persistent corruption with at most two retries. Five positives check a single successful descriptor acquisition, release before retry wait, authenticated return, no failure within the visibility contract and termination under weak fairness. Open abstracts successful acquisition; internal NotFound retries in `open_committed` and other open errors are outside this model, so descriptor counts are not syscall counts under arbitrary failures. Five algorithm controls double-open, retain a descriptor across retry, skip envelope/payload authentication or stop retrying; the last must produce a true temporal counterexample. One environment-only control breaks the fresh-open visibility attempt bound. Double-open violates cost policy, not old-version correctness; no millisecond bound follows.

The actual `bounds::frame_position` Verus contract covers every u64 parameter, proving extent/frame representability and exact absolute positioning. Four semantic controls omit extent containment, extent-address overflow, the correct origin or the frame header and must be rejected. `Config::frame_with_index` invokes it before seek. Real descriptor/index binding, cache-key comparison, Drop/syscalls, JSON and complete I/O control flow are outside this arithmetic proof.

`inspect_opened` returns the same file after index validation; ordinary and cold-session reads use it for envelope/payload reading. Warm sessions open a fresh file with their authenticated index. Sessions never retain data-file descriptors, and errors release this attempt's file before retry wait. Two native regressions check reading through the supplied descriptor after its path becomes unavailable and release/reopen behavior for failed views in ordinary/cold/warm reads. Private-file renames simulate per-open observations; these tests do not relax the production immutable-namespace contract.

## Composing name lookup and data retries

[`ReadLookup.tla`](../verification/storage/tla/ReadLookup.tla) adds fourteen cases. Seven positives cover small/actual/zero retry budgets, missing names or data beyond the conditional bounds, persistent corruption and other open errors. Six algorithm controls abandon missing names early, retry other errors, exceed the lookup budget, reset the outer budget, return success on missing, or stop outer retries. The last must produce a true temporal counterexample. An environment-only control weakens the name-visibility bound and must violate `NoFailureWithinContract`.

Positive cases check total call/sleep budgets, release before retry, authenticated success, immediate propagation of other errors, exhaustion of the appropriate budget, and termination under weak fairness. The actual-budget case explores 5,399 states. Thirty opens and 4.75 seconds of requested sleep bound this read policy, not storage response time. No p99 is assumed to be a deadline; nonreturning syscalls are outside the model.

Production Rust/Python and all twenty-one Verus inputs are unchanged. The 119 positive checks are rerun; the preceding 105 semantic negative controls are reused after exact-input checks, not claimed as newly executed. Likewise the 782 Python/40 native tests are reused only after production/test source checks. New four-host syscall injection is recorded separately. No new actual Rust theorem or complete I/O refinement is claimed; see the [measurement record](../verification/measurements/20261024-read-lookup.json).

## Offline orphan cleanup and shared ownership

[`OfflineCleanup.tla`](../verification/offline/tla/OfflineCleanup.tla) adds sixteen cases. Seven positives cover a catalog, a queue namespace, a genuine orphan, historical root references, active participants, failed closure validation and a shared marker added after the report. Eight algorithm controls authorize from stat, ignore open errors, skip reinspection, ignore queues, ignore quiescence, delete retained packs, ignore scan failure, or stop cleanup progress. The last must produce a true temporal counterexample. An environment-only control makes fresh-open NotFound untruthful. Positives allow failed deletion with effects; termination means completion or explicit refusal, not automatic retries or reclamation of all orphans.

The old policy's `NoOwnedDeletion` counterexample corresponds to Python `inspect.py` and its `Path.exists()` guard, not the Rust deletion controller. Shared protection represents a checkpoint owner or another queue's accepted input; stopping processes does not revoke those references. The new helper fresh-opens both markers, raises `UnsafeRecovery` on uncertain errors, and preserves report regeneration/comparison before deletion. Regressions and syscall injection check actual Python execution; no mechanical Python-to-TLA+ refinement is claimed.

Rust and all twenty-one Verus proof inputs are unchanged. The 119 positive checks are rerun; 105 semantic negatives and 40 native tests are reused after source checks. Changed Python production code receives a rebuilt wheel and the complete Python suite. Existing Rust proofs do not cover this Python guard; the [record](../verification/measurements/20261024-offline-guard.json) keeps those evidence types separate.

## Offline alias preservation

[`OfflineAliases.tla`](../verification/offline/tla/OfflineAliases.tla) adds seventeen cases: nine positives (four alias shapes with/without a separate canonical root, plus unknown resolution), seven algorithm negatives and one incomplete-trace environment negative. `AcceptedReadable` checks both physical contents and required path bindings. `NoTargetSubstitution` prevents unlinking an orphan alias's target; `NoUnknownDeletion` refuses uncertain resolution; `SuccessfulCleanup` checks actual orphan removal on successful completion. Failed unlink/sync may have effects. `Terminates` means completion or refusal, not unconditional reclamation; the stalled algorithm must produce a true temporal counterexample.

Algorithm controls use raw names, protect only the final file, protect only its ancestors, delete partials unconditionally, unlink resolved targets, ignore resolution errors, or stall. The final-file-only negative requires a directory alias: a file alias to that same final file is already conservatively protected by identity comparison. The initial chain-only negative therefore correctly produced no violation; the archived corrected configuration exercises the missing directory binding.

The implementation in [`inspect.py`](../src/straw/inspect.py) resolves pool-relative identities and traces accepted reference bindings, including links and directories before `..`. It filters both sealed/pack and partial candidates. `retained_paths` now reports normalized physical identities; deletion lists retain lexical candidate names. Successful caches are inspection-local. Nineteen new Python regressions cover alias reads, record/dependency closure, independent orphans, dangling aliases, target preservation and uncertain resolution. The model supplies a complete trace; the Python resolver itself and its correspondence are not Verus-proved.

The [measurement record](../verification/measurements/20261025-offline-alias.json) separates fresh wheel/Python/TLC checks from the unchanged Rust proof boundary. Rust/native and all twenty-one Verus inputs remain unchanged; 119 positive checks are rerun, while 105 semantic negatives and 40 native tests are reused only after exact-input checks. No new end-to-end filesystem or full recovery proof is claimed.

## Runtime cost and verification

Proof/spec code and ghost histories are excluded from ordinary builds. This does not make the accompanying implementation fixes and refactors free. The [controlled release-wheel comparison](FILESYSTEM.md#runtime-comparison-during-verification-work-2026-10-25) retains both improvements and slowdowns, including higher JuiceFS catalog replay latency. Treat throughput and tail latency as separate deployment acceptance evidence; neither Verus success nor a small benchmark establishes absence of regressions. This measurement does not extend the proved code boundary.
