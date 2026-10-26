# Environment models and assumption review

[中文版](ENVIRONMENT_MODELS_zh.md) · [Verification layers](FORMAL_VERIFICATION.md)

An environment model specifies what may happen **after straw issues an operation**:
which observations, responses, partial effects and external interleavings the
algorithm must tolerate. It is separate from the algorithm's choice of operation
and the safety/progress property we want. A model can be mathematically consistent
and still describe the deployment incorrectly.

The reviewable inventory is
[`verification/environment_contracts.json`](../verification/environment_contracts.json).
Each contract has a stable `ENV-*` ID, layer, status, source and evidence links.
An **explicit model** enumerates behavior; an **assumption** states a required
contract; a **direct unproved premise** is stronger than what we have derived;
**measurement only** supplies observations. None of these labels is a deployment
certification. Missing models are listed explicitly in the inventory.

Inventory version 2 separates `review.provided_by` (who supplies the guarantee),
`review.implementation_obligation` (what the straw adapter must do) and
`review.unproved` (the missing correspondence proof) for every contract.
For example, successful sync establishing durability is an environment contract;
**saving the pending file description before issuing sync is an algorithm
obligation**. It cannot be assumed to happen on straw's behalf. These fields
support independent review; they do not add a proof.

See the [implementation/proof correspondence](VERIFICATION_BOUNDARIES.md) for caller duties versus external guarantees. Global reclamation under `ENV-CROSS-LOG` additionally requires application holders to finish/release; permanent readers remain protected. Only permanently unowned objects qualify for conditional reclamation. WAL callers under `ENV-AUTH` must hash the correct header/payload spans; proved byte comparisons do not establish that caller obligation.

`ENV-JSON-LIBRARY` lists eighteen serde_json primitive access/clone/map/index/number/iterator/equality contracts; `ENV-JSON-PARSE` separately supplies successful deserialization, without an acceptance theorem. Actual catalog raw decoding checks the run and carries field/operation meaning into Target history. Actual append serialization/history binding is checked under ENV-JSON-ENCODE; native namespace/set/cache conformance remains a separate straw obligation.

Actual queue metadata editing uses the additional Map `get_mut`/`remove` contracts: mutable lookup lends exactly one existing value, an unchanged borrowed value leaves the map unchanged, and removal changes only the chosen key. Clone preserves canonical content and every finite-depth observation. These are external-library premises; ordered metadata editing is verified straw code. Native dispatch, error quarantine and complete coordinator/GC history correspondence are separate obligations.

`ENV-CATALOG-RESOLVER`, `ENV-SET-BUILDER` and `ENV-COLLECTION-ITERATOR` separate namespace identity, exact bulk set construction and consuming HashMap iteration. Actual field-to-operation construction is proved under these contracts. Native path/cache and bulk-set adapters remain straw obligations; the two library iterator primitives remain trusted. This does not close raw parsing/run binding or complete business replay.

`ENV-COMMIT-EFFECTS` separates native per-transaction retain/accept/apply/cleanup obligations from the verified dispatcher. `ENV-COMMIT-APPLICATION` permits caught unwinds that bypass Result handling. `ENV-TRAINING-CHECKPOINT` is supplied by the application: coherent checkpoint selection and replay protection are separate from queue processing acknowledgement. These contracts do not grant automatic model recovery.

`ENV-GC-BATCH-EFFECTS` separates target/parent binding, unlink, directory barriers and receipts for the actual batch dispatcher. The filesystem supplies truthful name-absence and sync durability; straw must choose the right targets, maintain durable marks, handle NotFound, repair missing parents and record matching receipts. Verus proves batch coverage and the actual `gc_io` unlink/parent dispatch under `ENV-GC-PRIMITIVES`, plus the ordered ancestor-barrier loop under `ENV-DIRECTORY`. Native target/path/handle binding, ancestor enumeration and aggregate repair durability remain separate obligations. Persistent absence also requires sealed objects not to be recreated by other writers; paused/crashed-writer handling cannot be hidden in a sync assumption.

`ENV-REQUEST-REPLAY` separates request lifetime from payload lifetime: accepted receipts can outlive data removed by application retirement and GC. The environment may repeat IDs, conflict content, lose replies and crash repeatedly. Receipt reconstruction, conflict checks, authorization and early cached replies are straw duties; the finite model leaves lower atomic acceptance and full cross-log refinement open. Clone/get/insert primitive contracts support actual receipt code, with full key/hash and coordinator call composition still unproved.


`ENV-JSON-LIBRARY` also exposes borrowed `str` Index::index_into as a narrow primitive: the pinned implementation delegates to Map::get and returns None for absent/nonobject fields. Actual queue_event selection and its WAL/JSON target equivalence are proved under this library contract. The additional mutable index primitive delegates to Map::get_mut and specifies the selected field plus preservation of every other field; absent fields leave the value unchanged. Actual queue_task creation, retry, lease, terminal, failure and continuation updates are checked under these library laws. Complete caller dispatch, reader roots, cache correspondence and history composition remain straw obligations. Actual queue_state now proves nested task selection, exact leaf dispatch, peer/non-task-root preservation and the ordered uniform-change batch/error-prefix relation, using the existing library contracts. Complete outer event selection, counters, caches and mixed history remain straw obligations. Actual queue_admin now proves bounded order allocation, ordered submission/producer effects, epoch assignment/revocation and reader/receipt table binding. Global counter/index/history correspondence remains open; native capacity admission is tested after cached-response lookup and before writes, while complete caller refinement is unproved. Compound queue_transition bodies now prove the ordered task/reader/receipt effects and exact error prefixes. The derived continuation-root theorem additionally requires the existing String content identity premise to bind the cloned attempt to the same map key; full root extraction, admission and catalog ownership remain open. Native Value equality is a separate trusted primitive: true implies identical finite observations, not identical serialization. The pinned sorted-map/float-roundtrip feature configuration is audited. Actual reader-root/ordered-release and completion/producer updates are proved; fresh attempts, worker retirement filtering, full roots and business-history composition remain caller obligations.

`ENV-JSON-LIBRARY` now includes the normal-return Map::retain primitive: complete original-key coverage, callback-result/final-value binding and no new keys. The actual immutable-capture worker callback, reader-root binding and event dispatcher are verified separately. Native cache callback semantics, API admission/freshness, root/ownership composition and unwind behavior remain open; queue_history now supplies target admission and the business history fold; see [the dispatch boundary](FORMAL_VERIFICATION.md#actual-queue-event-dispatch-and-worker-retirement).


`ENV-JSON-VALUE-CONSISTENCY` separately states the Value parse-observation law. It preserves null/bool/as_u64/text, array order and object keys/values across independent successful interpretations of the same bytes. It ignores allocation/layout and asserts no acceptance or custom Deserialize determinism. `ENV-STRING-KEY` separately states logical String identity from equal character content. The catalog field/operation/history equivalence proofs consume these premises; native physical prefix selection and complete Coordinator/GC composition remain open.

`ENV-JSON-ENCODE` isolates the external serializer relation and its Value-only compatibility law. It requires the actual compiled `float_roundtrip` feature; a generic custom serde type is not promised to round-trip. straw must preserve input fields, bind the run, use the exact serialized bytes/operations and maintain history. Catalog interpretation uniqueness is now proved under the separately reviewable `ENV-JSON-VALUE-CONSISTENCY` and String content-key premise; native adapters remain open.


The current queue-history boundary separates three responsibilities. `ENV-JSON-LIBRARY` and parser/encoding contracts supply library behavior; actual `queue_history` proves preparation, target admission, ordered business updates, receipts and exact partial failures. `Caches` and `Recovery` are independent mutable callback objects with read-only business inputs: their contents and termination are unconstrained, and normal-return business preservation follows from the borrow boundary. Native cache consistency, recovered-consumption identities, panic quarantine and run/File/fencing conformance remain implementation obligations. `State::apply_business` binds the actual decoded vector; the successful byte-prefix/business fold is proved without assuming independent JSON reparses are identical. See the [history record](../verification/measurements/20261026-queue-history.json).

`ENV-QUEUE-APPLICATION-HOOKS` separately records each actual before/after callback invocation and its returned success flag. It guarantees neither success nor termination and does not grant durability or correct business/cache contents. The application controller proves exact callback/error ordering and real journal quarantine; native hook-name mapping, full CommitIo context and arbitrary unwinding remain separate. Recovery exposes only fresh, fully replayed business state and keeps failed journal states quarantined. See [the application proof boundary](FORMAL_VERIFICATION.md#actual-application-guard-and-fresh-recovery).

`ENV-QUEUE-GC-CALLS` separates the actual roots-to-retirement and locked collection code from its JSON/digest, WAL and named filesystem primitives. The preflight callback has no successful-effect assumption and cannot borrow queue/WAL state; progress-source/cache and outer lock provenance remain straw caller obligations. `ENV-NAMESPACE-PRIMITIVES` isolates filesystem/path observations and borrowed component iteration from straw cache and resolver obligations. `ENV-WRITER-RETIREMENT` names fixed-gate exclusion, permanent fencing, and the independent requirement that submitted I/O finish or be cancelled before lock handoff; a PID, timeout or advisory lock alone does not prove that requirement. These contracts keep native adapter conformance and complete history refinement explicit. The JSON Map key-uniqueness law is a foreign library representation fact, not a root-preservation assumption.

## File and buffer primitive boundary

`ENV-FILE-PRIMITIVES` separates OS/library/callback guarantees from the actual `wal_source` and `wal_sink` dispatch proofs. A buffered logical cursor may differ from the underlying file position; consuming the buffer must preserve identity/bytes/durability, and a later seek supplies the required cursor. Returned errors may have effects. `Source` maintains explicit validity and refuses cached-length success after a failed effect; `Appender` derives Writer guarantees from file and hook operations. Empty reads may succeed past EOF. Native thin wrappers, snapshot consistency, outer lock/open/run binding and arbitrary unwind behavior remain distinct obligations. See [the file-adapter proof scope](FORMAL_VERIFICATION.md#actual-replay-and-append-descriptor-dispatch).

`ENV-CATALOG-TRANSACTION` isolates lock/open/metadata primitives from straw context, refresh and callback obligations. The actual transaction dispatcher checks binding and fresh identity, recovers before invocation and validates context before success. Native adapters must establish the corresponding state; legal callbacks must preserve identity when they leave an active transaction. Error-return paths and caught-unwind regressions are separate evidence, and lock errors do not imply release. See [transaction scope](FORMAL_VERIFICATION.md#actual-catalog-transaction-entry-and-exit).

The catalog callback review is a concrete example: the dispatcher assumed a correctly scoped native callback, but the previous Rust API exposed `&mut Catalog`. Native tests showed that ownership extraction could preserve write authority after unlock, and a caught panic could leave direct methods usable. Borrowed `LockedCatalog` callbacks and unwind invalidation repair those implementation gaps. These were adapter/API defects found while reviewing the boundary, not filesystem violations or bugs automatically discovered by the conditional Verus proof.

## Dependency traversal source and cache boundary

`ENV-CLOSURE-SOURCE` is a straw adapter obligation: one successful expansion must return the bound node's own canonical pack and every record/dependency successor, with stable identities. Actual `closure_node` proves field/array checks and complete ordered digest/reference binding under Digest and Value contracts, with graph binding conditional on the graph domain and key context. Native authenticated reads, serialization/SHA, namespace and complete `ClosureReader` correspondence remain open. Under that interface, actual `closure::collect` proves full reachable-pack equality, cycle-safe finite traversal, complete requested-root caching, eviction preservation and no partial cache on errors. `empty_cache` supplies the base; `resolve` rejects any reachable tombstone. Cached graph/context binding and complete native retain/GC remain separate obligations. `ENV-GRAPH` continues to state the finite TLA+ models' graph/cache assumptions; its review now records this narrower actual-Rust result. [Formal scope](FORMAL_VERIFICATION.md#actual-dependency-traversal-and-closure-cache).

## GC candidate-to-mark boundary

Actual `gc_mark` composes recovered catalog roots and candidates with resolved JSON encoding, durable catalog append and a complete pending-deletion snapshot. It introduces no new whole-operation environment assumption. `ENV-JSON-LIBRARY` adds two external primitives: empty Map construction and exact u64 Number construction. Their weakened-contract controls must fail verification. Existing Writer/Resolver/JSON/String contracts remain; lock freshness, canonical recovered identities, complete logical roots, physical deletion adapters and retry progress are straw obligations, not filesystem promises. See [formal scope](FORMAL_VERIFICATION.md#actual-gc-discovery-durable-mark-and-pending-snapshot).

## Boundaries at each layer

| Environment layer | Operation / input issued by straw | Environment behavior or required contract | Review location and gap |
|---|---|---|---|
| Inputs and runtime | Filter IDs, integer arithmetic, collection operations | Duplicate/empty filters are possible (`ENV-INPUT`); streams may under/over-deliver, change length or ignore callback errors (`ENV-STREAM`); machine integer and collection contracts are trusted (`ENV-RUNTIME`); the reclamation index requires the String hash/equality key law (`ENV-STRING-KEY`), and publication holds require the String ordering law (`ENV-STRING-ORDER`); retries may change dependencies (`ENV-PUBLICATION-RETRY`); faithfully decoded envelope/caller layout fields (`ENV-TYPED-DECODE`) | `EnvironmentInputs.tla`, Verus/vstd contracts. The String-specific laws are not discharged. No allocator, compiler/binary or full FFI proof |
| Namespace identity | Resolve a relative pack path and replay old names | Truthful symlink resolution and stable prefix/alias bindings, including normal/missing responses (`ENV-NAMESPACE`) | `EnvironmentNamespace.tla`, `NamespaceIdentity.tla`; actual filesystem resolver and replay correspondence unproved; bind-mount/inode equivalence excluded |
| Offline cleanup | Inspect accepted roots and unlink orphan names | Truthful standalone detection, real quiescence and complete reference-resolution bindings, including aliases before `..` (`ENV-OFFLINE`, `ENV-OFFLINE-BINDINGS`) | `EnvironmentOffline.tla`, `EnvironmentBindings.tla`, `OfflineCleanup.tla`, `OfflineAliases.tla`; complete Python/OS refinement unproved |
| Local writer access | A fault callback or another thread requests metrics/seal/close while publication holds its mutex | Exclusive nonblocking mutex tokens, busy/poison responses and a callback that may depend on the requesting caller (`ENV-LOCAL-ACCESS`) | `EnvironmentLocalAccess.tla`; actual `access::decide` is proved, but standard mutex/GIL/FFI and adapter correspondence remain unproved |
| Read visibility | Fresh open and read/authenticate regions | Each open may see a different subset of missing regions until an attempt-based bound; persistent corruption is possible (`ENV-VIS`). Individual complete/short/error responses use `ENV-READ`. Authentication is collision-free in the model (`ENV-AUTH`) | `EnvironmentVisibility.tla`; defensive model exceeds normal documented JuiceFS observations. Actual cache pages and hash implementation are abstracted |
| Durability and recovery | Write, unlink, file/directory sync, replay | Successful sync makes preceding state durable (`ENV-SYNC`); visible directories may be nondurable, failed creation may leave prefixes and ancestors need separate barriers (`ENV-DIRECTORY`); failure is not rollback; a name can remain absent or return after crash (`ENV-DELETE-FAILURE`); stat presence may be stale (`ENV-GC-STAT`); successful unlink / trustworthy unlink NotFound means current absence (`ENV-GC-IO`); a sealed empty reservation can lack its parent and open NotFound is not durable absence (`ENV-GC-PARENT`); an uncertain complete frame may survive (`ENV-WAL-RECOVERY`); serialized appends can leave any unconfirmed header/payload/trailer prefix (`ENV-WAL-PREFIX`) | `EnvironmentPersistence.tla`, `EnvironmentWal.tla`. Service/device power-loss behavior and the complete byte-level replay adapter are not proved |
| Physical batch reclamation | Unlink indexed targets, synchronize shared parents, record receipts | Stable target/parent binding, marked immutable names, truthful unlink and directory durability; errors may already have effects (`ENV-GC-BATCH-EFFECTS`, `ENV-GC-PRIMITIVES`, `ENV-DIRECTORY`) | Actual batch/unlink/parent dispatch and ordered ancestor barriers proved; native target/path/handle binding, aggregate repair durability and full roots/progress remain open |
| Incomplete writes and live writers | Save offset, truncate/unlink, sync and retry | One owning writer controls the private path; truncation preserves the prefix at the accurate saved offset (`ENV-INCOMPLETE`); failures may already remove suffix bytes | `EnvironmentIncomplete.tla`, `IncompletePublication.tla`. Offset capture, file identity, PID guard placement and writer-crash recovery remain unproved |
| Writer process lifecycle | Already authorized writes, pause/resume and close | Paused processes may resume; process death is separate from storage faults; suspicion does not revoke old write capability (`ENV-WRITER-LIFETIME`) | `EnvironmentWriter.tla`, `WriterLifetime.tla` describe the baseline; explicit retirement is modeled separately by `EnvironmentWriterRetirement.tla` / `WriterRetirement.tla` |
| Publication setup | Register a fresh name, create/open, capture offset | Errors may have registered the name or created an empty file (`ENV-PREPARATION`); private identities are unique | `EnvironmentPreparation.tla`, `PublicationPreparation.tla`; guard placement and the complete I/O adapter remain unproved |
| Complete-publication handoff | Stage, seal, release internal owners, return success | Failed responses may have effects; callers may offer another ID/API or conflicting contents (`ENV-HANDOFF`) rather than waiting for the original handoff | `EnvironmentHandoff.tla`, `PublicationHandoff.tla`; actual request matching, guard placement and full I/O/GC refinement are unproved. A library return is not remote RPC acknowledgement |
| Staging release and post-GC retry | Expand dependencies, release, reopen/retry | Every unowned manifest may be collected after a lost release response; immutable reference identities name complete direct edges (`ENV-CONSUMPTION`) | `EnvironmentConsumption.tla`, `StagingConsumption.tla`; one-frame graph/release, full replay and byte binding remain adapter obligations; legacy logs may lack the graph |
| Post-commit staging cleanup | WAL acceptance, staging release, cached responses and recovery | Cleanup may fail after durable acceptance; receipt lookup must be truthful, and publishers may restage the same reference (`ENV-COMMIT-CLEANUP`) | `EnvironmentCommitCleanup.tla`, `CommitCleanup.tla`; atomic intent/receipt persistence, recovery and entry barriers are straw obligations, not proved adapters |
| Catalog and other clients | Lock, refresh, retain/release, scan/mark | Reported staging/sealing failures can have durable effects (`ENV-PUBLICATION-FINALIZE`). Initial fresh-open absence is trusted and pool mode is set before clients start (`ENV-GC-DISCOVERY`). Scan-to-tombstone and restoration recheck-to-append critical sections serialize against compliant peers (`ENV-CATALOG-SERIAL`). Peers supply complete dependency closures and release after actual read completion (`ENV-PEERS`) | `EnvironmentCatalog.tla`, `EnvironmentPersistence.tla` / `PublicationFinalization.tla`, and peer actions in `Ownership.tla`. Distributed lock implementation, freshness and atomic durable catalog updates are assumptions |
| Dependency graph and closure cache | Expand a reference; reuse a cached closure | Finite immutable graph; successful expansion includes every successor and cached values are complete transitive closures (`ENV-GRAPH`) | `EnvironmentGraph.tla`, `DependencyTraversal.tla`. Traversal models successful operations; real I/O errors, JSON decoding and recovery are not composed. Heap memory is not bounded |
| Coordinator and time | Issue/use lease, process completion, replace owner | Exactly one externally fenced owner (`ENV-FENCING`). Expiry is a nondeterministic Boolean event while idle (`ENV-CLOCK-ABSTRACT`) | `EnvironmentClock.tla`, `Completion.tla`. No numeric clock drift, delayed-RPC queue, leader-election or split-brain model |
| Execution progress | Schedule steps; retry after a failure | Weak action fairness (`ENV-FAIR`); the GC fault period eventually ends, with no retry-count bound (`ENV-STABILIZE`); callers retry failed GC attempts (`ENV-GC-RETRY`) | `EnvironmentProgress.tla`, `environment.rs`. The GC deletion-retry model derives completion from these premises; per-task nonstarvation is unproved |
| Verus model/adapter boundary | Advance a controller; update durable catalog | Generic controller consumes stable effect responses and separate effect/retry weak fairness (`ENV-VERUS-PROGRESS`); actual histories must map to serialized catalog steps (`ENV-VERUS-CATALOG`) | `verification/common/verus/environment.rs`. Actual I/O, locks and recovery adapters are **not proved** to produce these histories, responses and fairness conditions |
| Deployment and performance | Execute the complete workload on real hosts | Observed visibility, throughput and control latency depend on workload/cache/client state (`ENV-PERFORMANCE`) | Measurement artifacts and [FILESYSTEM](FILESYSTEM.md). No measured p99 is a hard latency bound or theorem premise |

These are relative boundaries. `Ownership.tla` treats a catalog update as one
atomic durable step, while lower-level GC deletion and WAL models split relevant
effects into stages. Those lower-level models do not automatically prove that
the upper-level atomic contract is implemented. That composition/refinement
obligation remains open. Other clients' acquire/release/rewind actions stay in
`Ownership.tla`: they are modeled protocol participants, not arbitrary attackers
or a proved implementation of every peer.


The catalog state and physical replay interfaces are separate review boundaries. `catalog_log.rs` now proves typed frame-history folds, index/cursor/sequence consistency, partial-apply quarantine, rebuild and matching confirmation. `ENV-REPLAY-APPLY` still requires the raw payload/decoder binding; `ENV-REPLAY-IO` still requires Source's buffered snapshot and primitive-result correspondence. Source exclusively borrows the actual File, but no contract treats that borrowing as proof that sync happened. `ENV-VERUS-CATALOG` still requires the complete locked durable history. These are straw obligations, separate from truthful filesystem responses.

## Mapping to the verification layers

L0–L5 in [FORMAL_VERIFICATION](FORMAL_VERIFICATION.md) classify evidence; the table above classifies component boundaries. Independent review should identify both.

| Verification layer | Environment consumed here | What remains after this layer passes |
|---|---|---|
| L0 contracts | Deployment/protocol declarations and explicit omissions | Overstrong or missing response assumptions; declarations supply no proof |
| L1 TLA+ search | Separate `Environment*.tla`, configured input domains and fairness | Finite search limits, implicit premises and correspondence to real code |
| L2 Verus mathematics | Histories, I/O/crash responses, fairness and caller retry conditions in `environment.rs` | Which assumptions an unbounded theorem needs and whether lower layers implement atomic catalog actions |
| L3 production Rust functions | Function preconditions, vstd contracts and caller-provided descriptions/results | Call placement, binding values to real files and syscall interpretation; a proved function does not discharge its caller's obligations |
| L4 integration tests | Fixtures, fault hooks/syscall injection and process scheduling | Whether injections represent the intended responses and which cutpoints they cover; other executions are not exhausted |
| L5 deployment measurements | Actual JuiceFS clients, caches, network, load and cleanup procedure | Representativeness of samples; quantiles and bandwidth observations do not automatically become L1–L3 guarantees |

One `ENV-*` may be consumed at several layers. Follow the inventory to affected consumers after a lower-layer counterexample. For a new observed behavior, first determine whether the current model permits it before deciding to repair code or revise the environment.

## Who must satisfy a contract between layers

An environment is relative to the component being verified. It can include
**straw's own unverified lower-level code**. An execution outside an environment
assumption identifies a contract boundary; it does not by itself identify a
filesystem fault.

| Contract consumed by the upper layer | Parts responsible for satisfying it | Distinctions for independent review |
|---|---|---|
| `ENV-VIS`, `ENV-AUTH`: read observations and authentication | Filesystem/client caches plus straw's fresh opens, region selection, validation and error classification | An observation outside the allowed delay/corruption model versus reading the wrong region, skipping validation or failing to reopen |
| `ENV-PUBLICATION-FINALIZE`: staging and sealing responses | straw publication adapter and serialized catalog/recovery implementation | A successful standalone API returning before catalog seal, a failed operation whose effect survived, or staging missing before seal; a `.sealed` name alone does not satisfy the catalog contract |
| `ENV-INCOMPLETE`: incomplete-write cleanup | Filesystem truncate/unlink/sync, OS process identity and straw writer/recovery adapters | Truncation losing the prefix versus a wrong saved offset, lost cleanup descriptor, missing barrier or child touching a parent file; live-writer retry is not crash recovery |
| `ENV-WRITER-LIFETIME`: processes and open reservations | OS scheduling/death, straw write/close lifecycle and fencing in a future recovery protocol | Paused, dead or prevented from executing old work; retention without close despite healthy storage differs from failure to delete an already sealed target |
| `ENV-PREPARATION`: uncertain setup effects | Unique private names, filesystem open/unlink/barriers, catalog replay and the live writer adapter | An error may have created state; distinguish lost cleanup descriptions, premature offset capture, deleting an existing prefix, missing barriers and process death |
| `ENV-COMMIT-CLEANUP`: committed cleanup receipts | WAL/catalog durable prefixes, straw marker/replay/index adapters and caller retries | Cached responses skipping cleanup, missing durable intent, split receipt/release persistence, or falsely reported durable receipts; repeating a completed release can consume later staging |
| `ENV-CONSUMPTION`: immutable dependencies and post-GC retry | Manifest authentication, catalog frames/replay, other owners and GC | Root-owner absence does not prove the whole closure was consumed; distinguish missing true edges, incomplete saved graphs, split persistence, replay loss and legacy logs without recipes |
| `ENV-HANDOFF`: request interleavings and successful staging | Callers, catalog responses and straw handoff adapters | Other requests are allowed; overwriting an unreturned description is an implementation error. Successful staging without its promised owner violates the catalog contract, whose lower-level adapter still needs investigation |
| `ENV-PENDING-PUBLICATION`: complete-file sync responses | Filesystem durability, stable private paths and straw pending/retry adapters | A successful barrier losing bytes versus saving the description too late, switching files on retry, skipping a barrier or omitting authentication; preserving the description is an algorithm obligation |
| `ENV-NAMESPACE`: consistent identities for aliases | Filesystem lookup, stable deployment namespace and straw path/cache/replay adapters | A false readlink/absence response or retargeted alias versus using raw keys in ownership/tombstone/receipt paths; cache correctness alone does not prove resolution |
| `ENV-GRAPH`, `ENV-PEERS`: complete closures and owner lifetimes | straw's reference decoding, traversal/cache and other queue/reader/checkpoint callers | A dependency missing from the input, an edge lost by traversal/cache, or release before actual read completion; storage consistency alone does not explain these |
| `ENV-DIRECTORY`: durable directory chains | Filesystem entries, stable aliases/mounts and straw create/path-resolution/ancestor barriers | Skipping barriers after failed creation, syncing only the leaf, suppressing sync failure, external directory/alias replacement or a lying successful barrier |
| `ENV-GC-DISCOVERY`: initial managed-pool discovery | Pool initialization, filesystem name lookup and straw's open/error handling and shared requirement flag | A false initial open NotFound, an error interpreted as absence, a forgotten GC requirement, or a mode change while legacy clients remain active |
| `ENV-CATALOG-SERIAL`, `ENV-VERUS-CATALOG`: serialized durable catalog steps | Distributed locking plus straw's critical section, refresh, append/sync and replay adapter | A nonexclusive lock versus bypassing the lock, using stale snapshots or reporting a nondurable step as successful |
| `ENV-SYNC`, `ENV-GC-IO`, `ENV-DELETE-FAILURE`: deletion and persistence responses | Filesystem plus straw's ordering and interpretation of stat/unlink/directory sync | Loss after a real successful barrier versus a missing barrier, syncing the wrong directory or treating untrustworthy NotFound as actual absence |
| `ENV-GC-PARENT`: availability of the deletion barrier | Private namespace, creation permissions, directory open/mkdir/sync and GC classification/retry adapters | A never-created parent versus false absence for an existing one; perpetual retry without repair versus treating absence as a receipt; creation cannot replace existing directory identities |
| `ENV-APPEND-IO`: append byte effects and synchronization | Filesystem descriptor/byte semantics, SHA, and straw Appender | Shared append/catalog confirmation is proved under Writer; distinguish a dishonest primitive response from wrong native descriptor, callback handling or payload/operation binding |
| `ENV-WAL-RECOVERY`, `ENV-WAL-PREFIX`: durable prefixes and uncertain/incomplete frames | Storage durability plus straw's frame validation, truncation and recovery sync | Loss of acknowledged durable bytes versus dropping a valid frame, accepting a corrupt frame or using recovery results too early |
| `ENV-CROSS-LOG`: independent logs and deletion responses | Both durable log prefixes, directory barriers and straw cross-log call ordering | Advancing another log before durable retain/retirement, trusting recovery caches too early, or losing confirmed storage; failure choices for the logs must be independent |
| `ENV-VERUS-PROGRESS`: stable effects and two weak-fairness conditions | Effect responses, scheduling, caller retry and their controller-View correspondence | Recovery without caller retry, persistent invalid application inputs, nonreturning locks/I/O, or recovery that does not establish the required modeled history |
| `ENV-FENCING`, `ENV-CLOCK-ABSTRACT`: one owner and expiry responses | External owner management, real clocks and the lease/RPC adapter | Multiple writable owners, or an observed ordering that Boolean expiry cannot describe; numeric drift and delayed RPCs have no combined model yet |
| `ENV-FAIR`, `ENV-STABILIZE`, `ENV-GC-RETRY`: execution progress | Scheduler, storage service and the caller that invokes GC again | Persistent errors, action starvation and abandoned retries violate different premises; a progress-premise failure alone does not imply an ownership-safety failure |

The inventory's `models[*].contracts` lists each contract's consumers. Review
from consumer model to `ENV-*`, then to the code/service providing the guarantee
and its evidence. Mark an upper-layer assumption as derived only after proving
that the implementation and lower-layer model satisfy that contract. Passing
tests or moving a premise to a separate file does not discharge it.
`ENV-VERUS-PROGRESS` now supports a derivation of completion across failures and
retries, instead of directly declaring eventual next-stage success. Actual I/O,
recovery adapters and whole-control-plane correspondence remain unproved.

## Extracted executable specifications

The TLA+ algorithms import these independent modules:

- [EnvironmentVisibility.tla](../verification/storage/tla/EnvironmentVisibility.tla): allowed fresh-open region observations.
- [EnvironmentNamespace.tla](../verification/common/tla/EnvironmentNamespace.tla): stable resolved namespace identities (`ENV-NAMESPACE`).
- [EnvironmentBindings.tla](../verification/offline/tla/EnvironmentBindings.tla): complete resolution bindings for accepted references (`ENV-OFFLINE-BINDINGS`); final identities alone are insufficient.
- [EnvironmentReaders.tla](../verification/protocol/tla/EnvironmentReaders.tla): callers acknowledge readers only after actual completion; cancellation, timeout and recovery are insufficient (`ENV-PEERS`).
- [EnvironmentRead.tla](../verification/common/tla/EnvironmentRead.tla): complete, short or failed reads; errors may consume a prefix and one attempt may see persistent corruption (`ENV-READ`).
- [EnvironmentAuthentication.tla](../verification/storage/tla/EnvironmentAuthentication.tla): collision-free chunk authentication consumed by the range-read model (`ENV-AUTH`); binding expected bytes to references remains an adapter obligation.
- [EnvironmentIncomplete.tla](../verification/storage/tla/EnvironmentIncomplete.tla): truncation preserves the prefix; failed operations may remove unpublished suffix bytes (`ENV-INCOMPLETE`).
- [EnvironmentWriter.tla](../verification/gc/tla/EnvironmentWriter.tla): process pause, resume, death and unreliable suspicion (`ENV-WRITER-LIFETIME`).
- [EnvironmentCommitCleanup.tla](../verification/protocol/tla/EnvironmentCommitCleanup.tla): post-commit error responses and truthful durable receipts (`ENV-COMMIT-CLEANUP`).
- [EnvironmentLocalAccess.tla](../verification/storage/tla/EnvironmentLocalAccess.tla): exclusive nonblocking mutex tokens and busy/poison responses (`ENV-LOCAL-ACCESS`).
- [EnvironmentConsumption.tla](../verification/protocol/tla/EnvironmentConsumption.tla): direct dependencies of immutable references and disappearance after release (`ENV-CONSUMPTION`).
- [EnvironmentHandoff.tla](../verification/protocol/tla/EnvironmentHandoff.tla): requests during an unfinished handoff and the successful-stage contract (`ENV-HANDOFF`).
- [EnvironmentPreparation.tla](../verification/storage/tla/EnvironmentPreparation.tla): uncertain registration/open effects on fresh private names (`ENV-PREPARATION`).
- [EnvironmentParentRepair.tla](../verification/gc/tla/EnvironmentParentRepair.tla): missing parents, open responses and idempotent creation (`ENV-GC-PARENT`).
- [EnvironmentCrossLog.tla](../verification/protocol/tla/EnvironmentCrossLog.tla): independent queue/catalog complete-frame recovery projections and file/directory barrier responses (`ENV-CROSS-LOG`); cross-log commit is not one atomic transaction.
- [EnvironmentWal.tla](../verification/wal/tla/EnvironmentWal.tla): serialized three-unit append prefixes, successful barriers and complete-frame authentication (`ENV-WAL-PREFIX`). A crash may retain a partial unconfirmed frame; this is not a byte-decoder proof.
- [EnvironmentPersistence.tla](../verification/common/tla/EnvironmentPersistence.tla): successful sync effects and allowed post-failure
  name/frame observations.
- [EnvironmentCatalog.tla](../verification/common/tla/EnvironmentCatalog.tla): catalog access under the serialized-lock assumption.
- [EnvironmentClock.tla](../verification/protocol/tla/EnvironmentClock.tla): the Boolean expiration response.
- [EnvironmentInputs.tla](../verification/common/tla/EnvironmentInputs.tla): external request/filter domains, including duplicate filters and conflicting retry roots.
- [EnvironmentGraph.tla](../verification/common/tla/EnvironmentGraph.tla): immutable reference graphs and complete cached closures.
- [EnvironmentProgress.tla](../verification/common/tla/EnvironmentProgress.tla): weak-fairness and eventual-stabilization premises.

These definitions have no retry policy, assignment selection, tombstone choice
or receipt acceptance logic. Those decisions remain in the algorithm modules.
The models still compose algorithm actions and environment choices into one
`Next`; we have not built a universal asynchronous filesystem/RPC simulator.

The generic Verus theorem in
[`retry_liveness.rs`](../verification/protocol/verus/retry_liveness.rs) now derives
completion from stable effect responses and separate weak fairness for effect
execution and caller retries. The old `progresses` premise and helper theorem
are removed. Production `initial` / `advance` Views express failures, Crash,
Retry and idle steps for any finite stage limit. An infinite counterexample
shows why caller retries are necessary. `ENV-VERUS-PROGRESS` still requires valid
inputs, eventual operation returns and faithful effect reporting; filesystem
recovery alone does not prove all application stages eventually succeed.
`serialized_catalog_history` remains the abstract-history premise of the
higher-level unbounded ownership proof.

The separate unbounded GC retry proof in
[`gc_liveness.rs`](../verification/gc/verus/gc_liveness.rs) does **not** consume `progresses`.
It models successful I/O, failure with partial effects, crashes, a new invocation
after failure, and idle steps, using the production deletion controller's
`initial` / `advance` View transitions. `environment.rs` defines I/O/crash
responses, eventual fault cessation and weak fairness of I/O and retries.
After stabilization, the rank descends from failed attempt (3), through new
attempt (2), Unlink (1), and directory sync (0). Failures before stabilization
have no fixed count bound.

The pack must already be durably tombstoned, permanently unowned, and never
recreated by another client. Successful Unlink, including trustworthy NotFound,
must mean actual current absence. Stat NotFound no longer supplies this premise.
**Negative-cache behavior of unlink itself and conformance of the actual syscall,
catalog lock and replay adapters remain unproved.**
There is no automatic retry daemon: if callers stop invoking GC, storage recovery
alone need not finish deletion. Verus also proves this infinite counterexample
and a concrete successful retry history satisfying all the premises, checking
that they are jointly satisfiable.

Three additional Verus environment controls let sync success lie, remove retry
fairness, or allow errors forever. An algorithm control leaves the controller
failed across retry. All must fail semantically; reports distinguish
`environment_negative` from `algorithm_negative` and hash the actual mutated
environment inputs.

Pure arithmetic and filter-loop Rust proofs need the runtime contracts, not a
filesystem-progress premise. The collection specifications supplied by vstd are
trusted dependencies; the project's prohibition on `assume`/`admit`/omitted
bodies does not mean vstd, the solver or the compiler was proved here.

The production `reclamation.rs` HashSet mutations and generic `traversal.rs` frontier have the explicit precondition
`environment.rs::string_hash_contract()` (`ENV-STRING-KEY`). It states that
standard `String` hashing/equality obey vstd's key model. The pinned vstd does
not discharge this String-specific premise; the proof is conditional on it.
This is a runtime/library contract, not a filesystem or fairness assumption.
The proof still checks the actual mutations: duplicate tombstones do not
reschedule completed work, and receipts remove only marked pending paths.

`GCBatchDeletion.tla` additionally consumes `EnvironmentPersistence.tla`'s
`EnvSyncedDirectory`: a parent barrier persists preceding changes for its member
names, leaving other directories untouched (`ENV-SYNC`, `ENV-GC-IO`). The model
supplies a stable pack-to-parent mapping. The Rust adapter must construct the
complete parent set even for absent tombstones; that mapping/coverage is not
proved by the verified batch-completion loop. Early barriers and missing parents
have independent algorithm-negative counterexamples.

`GCReceipt.tla` composes deletion durability with complete-frame recovery
(`ENV-SYNC`, `ENV-WAL-RECOVERY`). A complete receipt can survive a reported append
failure because all parent barriers precede its write. A missing/torn receipt
leaves deletion pending. A receipt does not discard the permanent tombstone.
This requires the same no-recreation and compliant-peer contracts as deletion;
it does not model a malicious client forging valid receipts. Byte-level receipt
replay, actual adapter ordering and mixed-client behavior are tested separately
in `tests/test_gc_receipts.py` and `verification/gc/probes/gc_receipt_probe.py`, not proved
by this finite model or the HashSet contracts. The unbounded single-pack GC
liveness proof has not been extended through receipt persistence and recovery.

`PayloadStream.tla` consumes `EnvStreamLengths` from the input environment.
The algorithm does not trust a custom provider's length or callback-error
propagation and checks both traversals. Provider return is still a premise:
callback-count/length bounds only restrict exploration and cannot prove arbitrary
custom code terminates. The Rust length guard covers all `u64` inputs; composed
trait/closure and I/O conformance remains unproved. These are input-contract
failures, not JuiceFS visibility faults; more reader retries cannot repair them.

`EnvironmentGraph.tla` isolates `ENV-GRAPH`: finite immutable adjacency,
complete expansion and cached values equal to complete transitive closures.
`DependencyTraversal.tla` consumes it and checks omitted-edge/incomplete-cache
controls. `Ownership.tla` still assumes complete input closures; composition is
not proved. The actual generic frontier reuses `ENV-STRING-KEY` without proving
that library law. File/JSON-to-node/pack correspondence, actual cache correctness
and allocation success remain independent review obligations.

`ReactivationScan.tla` consumes `ENV-READ`, separating complete/short/error
`read_exact` responses from scan policy. An error may have consumed a prefix,
but those bytes cannot authorize an owner; corruption detection assumes
`ENV-AUTH`. This is one scan over a complete protected closure, without composing
retries, real locking, reference decoding or WAL recovery. Real syscall
termination remains outside the adapter proof. The 1 MiB payload scratch cap is
straw's algorithm policy, enforced by the actual Rust capacity function. It is
neither a measured filesystem constant nor a process-wide memory bound.

`ReactivationRace.tla` separately reviews unlocked authentication. Peers can
release, retain, mark and unlink while scanning; an open descriptor can still
finish successfully. **Verification supplies neither current ownership nor a
pin.** Final authorization must refresh and recheck sources/tombstones in the
same locked transaction as append. Identity binding relies on immutable extents
and complete references (`ENV-AUTH`, `ENV-GRAPH`); serialized critical sections
require `ENV-CATALOG-SERIAL`. An environment-only change to
`EnvironmentCatalog.tla` permits peers to bypass the final lock and violates
`NoDanglingRestore`. That control and two algorithm race controls explicitly
select this invariant plus `TypeOK`; positive cases check every property.
This does not compose byte scanning, real descriptors, private verification
tokens or WAL recovery into one proof. Coordinator now scans outside the lock;
the low-level compatibility API may still scan inside it. Metadata walks and
final checks/writes also require locking, with no lock-wait bound implied.

`CatalogReplay.tla` adds an `ENV-WAL-RECOVERY` interleaving: after replay sync
fails, storage may lose an unconfirmed complete tail while **the client process
and its parsed cache survive**. Parsing does not establish durability; storage
recovery need not restart the client. The algorithm must rebuild indexes after
possible tentative cache changes while retaining this object's confirmed durable-prefix floor. An empty refresh may reuse
its confirmed cache, but must still reopen, check new frames and sync. This is an
algorithmic work policy, not a new environment guarantee.
Stable I/O and fair retries remain qualitative premises. Actual cache rebuilds,
byte replay, locks and syscall correspondence still require implementation
review. The four-host fault probe deliberately simulates this response; it does
not establish that the JuiceFS deployment actually lost such a tail. See
[FORMAL_VERIFICATION](FORMAL_VERIFICATION.md#failed-catalog-replay-and-tentative-cache-state).

## Writer lifetime and eligibility for collection

`ENV-WRITER-LIFETIME` separates running, paused and dead processes. A paused
writer can resume after its catalog authorization, inside an in-flight write.
Already authorized kernel/server work can also complete while the process is
paused or dead: `EnvWriterCompletes` is independent of process execution.
Eventual storage recovery neither revives a dead process nor makes it close.
Suspicion may occur in any process state; it is not a death certificate. The baseline
without explicit retirement can retain an open reservation indefinitely. The new
incarnation retirement API supplies persistent fencing and registered publication
reconciliation under ENV-WRITER-RETIREMENT. It still requires truthful exclusion,
namespace/sync behavior and independent completion/cancellation of submitted I/O
before gate handoff. A PID or timeout cannot replace these requirements.

[`WriterLifetime.tla`](../verification/gc/tla/WriterLifetime.tla) starts with one
registered, unowned open pack and an authorized write. The normal policy checks
writer protection and conditional progress after close; the clean scenario also
checks eventual deletion. The controls distinguish different conclusions:

- **Unsafe proposed algorithm:** pause, suspicion, collector seal, writer resume,
  GC mark, then the old write proceeds. This violates `NoWriteAfterRetirement`.
  Current straw does not implement this proposal; the trace does not establish
  loss of an acknowledged publication.
- **Death does not drain writes:** `writer_dead_has_inflight` rejects sealing and
  marking a dead writer with outstanding I/O. `writer_dead_late_write` reaches
  `NoWriteAfterRetirement` through death, seal, mark, then write completion.
  `writer_crash_safe_leak` checks the current policy under this stronger environment.
- **Current progress boundary:** a writer is already dead, all storage operations
  succeed and scheduling is weakly fair, yet an open reservation remains forever.
  `RequiredProgress` has an infinite stuttering counterexample. This does not
  refute the deletion theorem for an already sealed, durably tombstoned,
  permanently unowned target.

Review whether the provider supplies inactivity, suspicion, or fencing that
actually prevents old operations. Future recovery must account for work
authorized before the pause. A PID, timeout or lock acquisition alone has not
been proved to supply this fence. Lock/session expiry, partitions and recovery
have no combined model here. Actual close, catalog serialization and in-flight
I/O are not mechanically refined to this model; completion and release are
abstracted together, without composing byte durability or complete ownership.

The [four-host record](../verification/measurements/20261023-writer-lifetime.json)
contains 16 real SIGSTOP/SIGCONT/SIGKILL cases, paused after the header outside
the catalog critical section. Eight resumed writers completed and authenticated
2 KiB/8 MiB payloads, released, closed and were collected. Eight killed writers
retained their open pack through three subsequent GC calls each. At termination
each file held only a 12-byte header: 96 bytes were explicitly removed as private
fixtures, not by GC. Finite observations do not prove an infinite leak, test
cross-host locks/power loss, or determine a safe timeout or throughput.

Review this layer independently:

```sh
python verification/runners/environment_manifest.py --contract ENV-WRITER-LIFETIME
python verification/runners/environment_manifest.py --model WriterLifetime
```

## Parameters: policy, environment, or exploration bound

| Parameter / value | Classification | Meaning |
|---|---|---|
| `MaxRetries` and retry delays | Algorithm policy | Attempts/waits the reader will spend; total sleep is not an I/O deadline |
| `verification_capacity`'s 1 MiB cap | Algorithm policy | Payload verification scratch, excluding metadata/graphs/return values; no storage response-time bound |
| `VisibleBy` | Environment premise | Fresh opens are complete from this **attempt number**, not after this many milliseconds |
| `AllowedGaps`, `Corrupt` | Environment behavior | Permitted incomplete observations / persistent corruption |
| `Readers`, `Tasks`, identity sets, `MaxFilter`, small `WordMax`, crash/epoch counts | Finite exploration bounds | Restrict TLC's state space; they are not API quotas or production size limits |
| Weak fairness / eventual fault cessation | Temporal premises | Qualitative progress assumptions; no quantitative deadline |
| Measured p99 / maximum | Deployment evidence | An observed percentile/sample maximum; cannot populate a universal convergence or lease bound |

Published byte sizes from a few KiB through one real 100 GB record, plus sparse
TB-offset checks, test different implementation boundaries. Sparse offsets do
not test TB throughput, and large data runs do not prove metadata service
availability. Keep the workload and measurement scope with the assumption review.

## Evidence and independent review

Both formal runners copy the environment inventory beside their reports. TLC
records the algorithm hash, configuration hash, exact environment module hashes,
contract IDs and `case_kind` for each case. Verus records all actual proof inputs
and the environment hashes. A registry/import mismatch fails before verification.
CI archives these files; a source distribution includes them too.

Review the inventory without starting TLC or Verus:

```sh
python verification/runners/environment_manifest.py --check
python verification/runners/environment_manifest.py --list-layers
python verification/runners/environment_manifest.py --layer durability --layer progress
python verification/runners/environment_manifest.py --contract ENV-GC-IO
python verification/runners/environment_manifest.py --model GCReceipt
```

`--list-layers` derives a component-layer/contract index from the inventory.
`--layer` exports every selected layer's contracts, providers, implementation
obligations, unproved correspondence, evidence and affected consumers. These
layers are component boundaries, distinct from the L0–L5 evidence levels.
`--contract` narrows review to named contracts. `--model` exports a consumer's
premises across all layers and lists other consumers of each premise. The three
selection modes are mutually exclusive and each can be repeated; no arguments
exports the entire inventory as JSON. Every review report includes a `layers`
index derived from that same inventory.

A durability-layer review is only part of a GC review: GC also depends on
ownership, namespaces, scheduling and retry. Use `--layer durability` for that
boundary, then `--model GCReceipt` or `--contract ENV-GC-IO` to trace consumers.
`ENV-PERFORMANCE` intentionally has no formal consumer: measurements remain
outside theorem premises.

The checker rejects missing review fields, duplicate IDs/list entries, invalid
evidence paths, unknown contracts, omitted TLA consumers and registry/import
mismatches. Reverse dependencies come from the same inventory, without a second
handwritten index. This checks **structural consistency of declared dependencies**;
it cannot prove that semantic premises are complete or evidence satisfies them.
The supported import syntax is a single-line `EXTENDS`; new import forms require
updating the checker.

The version 2 [review validation record](../verification/measurements/20261023-environment-review.json)
preserves the inventory hash, negative checks and complete TLC/Verus reruns,
separately from deployment performance measurements.

The [layer-query validation record](../verification/measurements/20261024-environment-layers.json)
covers 19 component layers, 38 contracts and 34 consumers, and checks unchanged
dependency content for 73 existing report selections. This run repeats the
nineteen environment controls and positive Verus verification (115 checks).
Production code, environment contracts and model sources are unchanged; prior
full-suite results are not reported as new runs.

To run only the environment controls, retaining the consuming algorithms and
exact environment inputs:

```sh
python verification/runners/check_tla.py --case environment_sync_lies --case environment_wal_forgets \
  --case environment_lock_not_serial --case environment_visibility_bound_broken \
  --case environment_reactivation_lock_not_serial --case environment_discovery_open_lies \
  --case environment_directory_barrier_lies --case environment_file_sync_lies \
  --case environment_namespace_identity_lies --case environment_truncate_loses_prefix \
  --case environment_gc_parent_sync_lies --case environment_preparation_unlink_lies \
  --case environment_handoff_stage_lies --case environment_consumption_edges_lie \
  --case environment_workflow_edges_lie --case environment_local_mutex_grants_busy \
  --case environment_handoff_receipt_lies --case environment_queue_record_early_ack \
  --case environment_queue_record_receipt_lies --case environment_range_authentication_lies \
  --case environment_typed_decode_lies --case environment_read_attempt_visibility_lies \
  --case environment_lookup_bound_lies --case environment_offline_open_lies
```

Twenty-four TLC controls change **only environment source**, leaving the algorithm
source unchanged:

| Weakened environment | Expected failed property |
|---|---|
| Fresh open falsely reports an existing shared marker as absent | `NoOwnedDeletion` |
| Name lookup remains missing beyond its conditional attempt bound | `NoFailureWithinContract` |
| Directory sync reports success without durable absence | `DurableCompletion` |
| Recovery may discard an already durable completion | `AcknowledgedDurable` |
| Catalog access bypasses scan-to-tombstone serialization | `NoDanglingReferences` |
| Peers mutate between restoration recheck and owner append | `NoDanglingRestore` |
| Fresh opens remain incomplete beyond `VisibleBy` | `NoFailureWithinVisibilityContract` |
| Initial fresh open falsely reports NotFound for a managed pool | `NoLostAcknowledgedData` |
| Directory sync succeeds without persisting a newly created child entry | `NoLostAcknowledgedData` |
| Complete-file sync succeeds without persisting its bytes | `DurableVerifiedReturn` |
| Alias resolution returns the raw name instead of its shared identity | `NoLostOwnedData` |
| Successful truncate wrongly removes the acknowledged prefix | `PrefixPreserved` |
| GC parent sync succeeds without durable deletion before receipt | `ReceiptIsSafe` |
| Setup unlink reports success while the private file still exists | `NoAbandonedPrivateFile` |
| Publication staging reports success without establishing a root owner | `CompletedHandoff` |
| Successful manifest expansion omits true dependencies | `ConsumeComplete` |
| Workflow consumption graph omits the payload dependency | `ReleasedHasRecipe` |
| Local mutex grants a token while the outer writer holds one | `ExclusiveAccess` |
| Post-commit cleanup lookup falsely reports a durable release receipt | `CompletedConsumption` |
| A large-record queue reader acknowledges before finishing its read | `NoLostReaders` |
| Large-record queue recovery falsely reports a durable release receipt | `NoUnconsumedStaging` |
| Chunk authentication accepts different observed/expected byte sequences | `NoCorruptReturn` |
| Typed decoding substitutes caller fields for authenticated envelope fields | `ReturnedMatches` |
| Descriptor-reuse reads still open incomplete views beyond the visibility attempt bound | `NoFailureWithinContract` |

They are reported as `environment_negative`, separately from algorithm mutations.
A required counterexample demonstrates that the theorem depends on that contract;
it is not evidence that JuiceFS actually violates it. For example, removing
serialization can represent either a broken lock service **or** implementation
code bypassing a working lock; the model alone cannot choose between them.

For a reported failure:

1. Preserve the operation/receipt IDs, observed bytes and order, exact binary,
   algorithm/configuration/environment hashes, and deployment settings. Confirm
   the loaded native binary, not just its package version.
2. Decide whether the observation is admitted by the current environment model.
   If admitted and the specification fails, examine the algorithm/property. If
   the model permits a correct response but the actual code violates it, examine
   implementation correspondence. If excluded, first check the documented
   deployment contract: the environment model may be too strong or incomplete,
   or the deployment may violate a required guarantee. Do not assign blame from
   a green model alone.
3. Add the concrete observation as a regression and, when justified, a new
   environment behavior. Re-run the consumers of that contract and their
   negative controls. Explain any changed state counts, assumptions and claims.
   Do not strengthen an assumption just to restore a passing proof.

### Incident review record

Use this table in an issue or experiment record. Leave unresolved entries as
unknown and name the next check that can distinguish the candidate causes;
a passing model does not assign fault to the environment.

| Entry | Required detail |
|---|---|
| Review boundary | Component layer, consuming model, `ENV-*` IDs and the L0–L5 evidence level being used |
| Observations | Operation/response order and observed effects; distinguish actual responses, inferred state and unobserved state |
| Reproducible identity | Binary and model/configuration/environment hashes, deployment settings, raw logs or counterexample paths |
| Allowed behavior | Whether the environment permits the observation; the relevant operator/premise and unresolved mappings from the execution to model steps |
| Responsibility and impact | Guarantee provider, straw adapter obligation and affected consumers; assess safety and progress conclusions separately |
| Finding and follow-up | Algorithm error, adapter error, incomplete/overstrong environment model, unmet deployment contract, or unresolved attribution; supporting evidence, regression and consumers/controls to rerun |

For GC, successful unlink followed by failed directory sync and an absent name
on retry is permitted by `ENV-DELETE-FAILURE`. If the implementation then skips
the directory barrier and writes a reclamation receipt, examine adapter ordering;
that observation does not itself violate the sync contract. Before attributing a
lost deletion to successful sync, establish the synced directory's identity,
operation order and crash-recovery evidence, then assess `ENV-SYNC`. If nobody
invokes GC again, `ENV-GC-RETRY`'s progress premise is unmet; this alone does not
imply a deletion safety failure.

Performance attribution follows the same boundary. `ENV-CATALOG-SERIAL`
requires mutual exclusion and freshness, not a lock-acquisition duration;
`ENV-FAIR` supplies qualitative model progress. The reactivation probe now times
candidate inspection, verification, final catalog access, append and sync,
separately recording experimental marker writes and pauses. The
[phase controls](../verification/measurements/20261023-reactivation-timing.json)
did not reproduce the earlier 8.68-second sample, which remains unattributed.
Final catalog time includes locking, refresh and authorization adapters: that
span alone cannot identify a slow lock service, change `VisibleBy`, or establish
a hard latency guarantee.

The separate [small-slice stage measurements](../verification/measurements/20261024-read-latency.json)
locate another set of slow requests inside `open64`: two opens dominate the
slowest JuiceFS samples. This evidence belongs to `ENV-PERFORMANCE` and changes
neither `ENV-VIS` nor `ENV-FAIR`. Waiting inside the call can still include kernel,
scheduling, network and server behavior. Correspondence between the model's
abstract Open action and real syscall completion remains an adapter/progress
obligation; observing one return does not prove all calls eventually return.
Any descriptor-reuse optimization must review both binding/authentication within
an attempt and fresh open after failure; average cost cannot justify omitting
the latter.

The remaining major gap is a composed model of JuiceFS internals, network/lock
services, numeric clocks and complete Rust recovery. Current evidence supports
review of the stated contracts and concrete executions, not that composed model.

## Adapter counterexample: stale stat

`ENV-GC-STAT` allows either stat presence observation to be stale. The old
`gc.rs` skipped unlink after stat NotFound, yet advanced the controller as if
Unlink succeeded. Existing controller proofs did not establish correctness of
that dispatch. Adding the stat step to `GCReceipt.tla` exposes a
`ReceiptIsSafe` counterexample. The adapter now calls unlink and advances from
its actual result; stat only supplies diagnostic accounting. This is a straw
adapter failing to satisfy `ENV-GC-IO`, not evidence that directory fsync lied.

Four-host natural observations found no stale NotFound in 1,200 checks, while
one injected stale stat reliably reproduces the old defect. The allowed model
behavior, deployment observations and implementation fix are distinct evidence.
Trustworthy unlink NotFound remains an assumption: this change removes the
need to trust stat NotFound, not all negative-cache assumptions. Real syscall
adapter conformance remains unproved, and existing erroneous receipts are not
automatically revalidated. See [the measurement scope](FILESYSTEM.md#gc-stale-stat-and-reclamation-receipts-2026-10-23).

## Refill environment and buffering policy

`ENV-READ` now includes `EnvReadChunkSizes`: a successful non-EOF refill may
return any positive prefix bounded by request capacity and snapshot remainder.
`CatalogReadBuffer.tla` consumes this response domain; the prefetch cursor does
not mean its bytes have been authenticated or applied. The 64 KiB capacity,
per-refresh buffer disposal and append seek are straw policies. Shared clone
cursors, read responses and seek semantics remain standard-library/OS adapter
contracts. One failure and small byte lengths are finite search bounds, not a
one-failure environmental guarantee. This model does not provide a unified
proof of byte replay, durable prefixes and multiple clients.

## Initial GC discovery and pool mode

`ENV-GC-DISCOVERY` separates stat observations from fresh-open discovery. Stat
may be stale or fail; the old adapter downgraded either to no GC and could
acknowledge a no-op retain. The new adapter uses fresh open and keeps the GC
requirement across Config clones. Errors do not disable GC; after automatic
discovery, opening a missing catalog fails instead of creating an empty
replacement. Explicit GC initialization still allows creating the first
catalog; this is not a recovery guarantee for a lost catalog.

The remaining boundary is **initial** fresh-open NotFound. Automatic legacy
compatibility trusts it as actual absence of a managed catalog, with pool mode
established before participants operate. Changing only that environmental
response produces a data-loss counterexample. Explicit `online_gc=True` avoids
using initial name absence to select legacy mode; it does not prove locking,
sync or the storage service. Live legacy-to-GC migration, historical-data
migration and shared atomic-state refinement remain unproved.

`GCDiscovery.tla` consumes this contract and abstracts catalog retain as an
atomic durable action. Passing other models does not compose those proofs.
The actual Verus decision function receives an abstract Observation; real open
responses and error classification remain adapter obligations. Four-host fault
injection is not evidence of naturally occurring deployment faults.

## Directory names are not durability receipts

`ENV-DIRECTORY` separates directory visibility from parent-entry durability,
including prefixes left by failed creation or failed sync. `DirectoryDurability`
consumes this contract. Each parent barrier covers entries in that directory;
syncing a new file or its leaf directory cannot establish its ancestor entries.
The implementation now repeats the complete resolved ancestor chain, even when
names exist. Stable directory identities, aliases and mounts, accessible
ancestors through the root, and correct path/I/O adaptation remain premises.
See [proof boundaries](FORMAL_VERIFICATION.md#directory-creation-failed-barriers-and-publication).

### Conflicting retries and internal publication ownership

`ENV-PUBLICATION-RETRY` allows a caller to retry the same identity with a
changed dependency set. Rejection does not imply rollback of earlier effects.
`PublicationRetry.tla` starts after a durable internal hold and interrupted
publication, with the original source released. This initial condition does
not authorize releasing a source after an arbitrary failed API call: the model
already assumes that protection was established. Other clients may run GC
between hold, rejection, correct retry and staging handoff. They must not mutate
another writer's private `publication:*` owner IDs.

The environment supplies complete immutable pack closures, serialized durable
catalog actions and, for progress, a correct retry under weak fairness. straw
must preserve previous attempt roots, stage the completed result, then release
the operation holder. These are algorithm obligations, not filesystem promises.
Abandoning the publication can leave retained roots indefinitely. This model
does not derive catalog atomicity from byte replay/locking or prove arbitrary
publication recovery and cross-process idempotency.

The actual `holding.rs` union loop has a separate runtime premise:
`environment.rs::string_order_contract()` (`ENV-STRING-ORDER`) requires String
ordering to satisfy vstd's ordered-key contract. This law is not proved here.
An environment-only mutant weakens the premise to true and makes the actual
loop proof fail; two algorithm mutants drop prior roots or ignore additions.
The proof establishes exact set union for arbitrary input sizes under the
library contract; closure decoding, collection conversion, catalog append and
call-site handoff remain outside it. Review these adapters when an observed
hold differs from the model, before blaming storage durability.

### Publication finalization and reclaimability

`ENV-PUBLICATION-FINALIZE` supplies a durable file registered as open, complete
staging roots, serialized catalog operations and compliant GC/peers. A failed
stage/seal operation may leave the old durable Boolean value or its successful
effect; `EnvironmentPersistence.tla::EnvUncertainBoolean` isolates this choice.
The model describes the durable result observed after serialized recovery,
without deriving it from raw frames, real locks or filesystem response timing.

straw chooses Stage before Seal and acknowledges standalone publication only
after both succeed. Packed publication stages but remains writable until an
explicit close/rotation. These are algorithm choices, not environment promises.
Caller retries, eventual stable I/O and weak fairness are needed for eventual
completion. File-to-reference correspondence, multi-file record_set composition
and the byte-level adapter remain separate review obligations. Actual deletion
is an upper-layer GC action here; the deletion/receipt models carry its lower
layer contracts. Existing open entries from abandoned writers must not simply
be inferred sealed from a `.sealed` filename or an old successful API result.

### Complete-file retries and sync knowledge

`ENV-PENDING-PUBLICATION` distinguishes a successful sync response from a failed
response whose effect may nevertheless persist. `PendingPublication.tla` starts
with a complete frame, open registration and its exact private file description.
The live writer may receive another identity, retry the original or encounter
persistent corruption. `ENV-SYNC` supplies the file barrier; `ENV-AUTH` supplies
checksum detection. A successful barrier does not establish the later final
name's directory durability. Path lookup/rename errors, crashes/restarts and
ownership handoff are not composed into this model.

Keeping the description before sync, fencing other identities and retrying an
unknown barrier are straw obligations. A confirmed sync may be reused for an
immutable standalone file; retry still authenticates its bytes. The abstract
progress result assumes modeled operations can execute and the caller retries
under eventual stable I/O/weak fairness; corruption may terminate with error.
It does not guarantee successful recovery when the original path has disappeared.
The real adapter retains pending state and reports such failures.

The actual generic `pending.rs` functions preserve the description and maintain
sync knowledge only from successful responses. They do not prove the filesystem
response contract or that the surrounding assignment precedes the syscall.
Review the adapter when an observation contradicts the model; the previous
standalone path remembered the descriptor too late even with correct fsync.

## Namespace contract and the implementation boundary

`ENV-NAMESPACE` requires truthful resolution inside a stable pool root. A normal
component or NotFound response must not conceal a symlink. Relevant aliases and
prefix bindings must stay stable for cached operations and later catalog replay;
previously absent private prefixes may become ordinary directories/files. File
unlink is allowed, with dangling aliases still identifying the same retired path.
Removing or retargeting aliases needed by retained history is outside this contract.

straw must normalize every owner/pack/tombstone/receipt boundary, reject escaping
paths and preserve normalization on replay. The alias data-loss regression occurred
with ordinary filesystem responses: it was a straw adapter bug, not evidence of a
JuiceFS consistency failure. The cache's two verified functions establish only
lookup/preservation; they do not discharge the resolver or its use by the adapter.

This is resolved-path identity, not inode identity: bind-mount aliases are not
unified, hardlink names remain separate, and unregistered extra links are not
reclaimed. Cached paths/prefixes and permanent history can grow without bound.
The resolver's 40-expansion dangling-link fallback guard is an algorithm choice,
not a measured filesystem constant or a bound on total kernel traversals.
Review `python verification/runners/environment_manifest.py --contract ENV-NAMESPACE`
against [the exact-binary measurements](../verification/measurements/20261023-path-identity.json).
Cold replay overhead is measured independently; no timing enters this safety premise.

## Recorded identities and producer obligations

`NamespaceEncoding.tla` separates writer normalization from replay's use of
`path_encoding=1`. Normalizing all four path-bearing fields **before** writing the
marker is straw's algorithm obligation, not something the filesystem provides.
Replay trusts compliant catalog producers (`ENV-PEERS`) and stable path identities
(`ENV-NAMESPACE`). An absent marker supplies no normalization guarantee; future
or malformed encodings must not be interpreted as version 1.

The actual Rust `identities::encoding` classifier is verified for every
`Option<u64>`. JSON-to-option decoding, normalization loops, marker placement,
byte framing and replay dispatch remain adapter obligations checked by tests.
The model explores a four-field event and conditional completion under weak
fairness; it does not refine all real frame histories or prove the producer's
Rust implementation. Supplying a truthful marker removes repeated readlink work;
it does not strengthen filesystem consistency, prove path resolution, bound
catalog memory or certify arbitrary writers. Timing and intercepted API counts
are recorded separately from these premises.

## Incomplete writes, truncation and process boundaries

`ENV-INCOMPLETE` supplies an accurate saved length, exclusive private path and
truncate responses; the algorithm chooses effect order. Packed cleanup may only
discard the unpublished suffix beyond that offset. Standalone cleanup removes
only its never-exposed private `.partial`. Successful truncation/removal still
needs a file/parent barrier before writer reuse or sealing its unexposed final
name. Failed operations may have effects, so the descriptor must survive. A
permanent peer-GC tombstone after an uncertain persisted seal also satisfies
retirement of that name.

Storage-crash actions keep the client descriptor and restore durable length/name
state. This is **not writer-process restart**. Lost descriptors after process
death and historical garbage may still leak safely. Fresh setup failures now
have the separate `ENV-PREPARATION` guard/model described below. No timeout automatically seals abandoned writers. Offset
capture, PID guard placement and complete syscall dispatch remain unproved;
preserving a generic descriptor does not prove its path or length was correct.

Linux [truncate semantics](https://man7.org/linux/man-pages/man2/truncate.2.html)
and [fsync/directory barriers](https://man7.org/linux/man-pages/man2/fsync.2.html)
provide interface-contract sources. Failure side effects are conservatively
allowed, not claimed to occur on every error. Four-host before/after syscall EIO
injections test adapters; they do not emulate real power loss or certify deployment
durability. [Measurements](FILESYSTEM.md#incomplete-writes-and-cleanup-retries-2026-10-23)
are separate evidence, never hard p99 guarantees.

## GC parent availability and progress premises

`ENV-GC-PARENT` allows a sealed empty catalog reservation to lack its parent:
registration precedes creation, failed creation can be followed by normal writer
close/seal. Compliant clients can leave this state. The old collector only retried
opening that directory and could never finish even after faults stopped. A model
premise saying sync eventually succeeds does not discharge this adapter obligation.

`GCParentRepair.tla` separates opening, absence, idempotent creation, sync, receipts
and retries. NotFound can mean actual absence or a fault-period false absence for
an existing parent, so it cannot justify skipping the barrier. GC invokes the
existing create/sync helper on missing responses; other errors propagate. Failed
creation/barriers keep the deletion obligation. A new client finding the repaired
parent present still syncs deletion. A real mkdir followed by injected EIO may be
reconciled as success by the standard library's directory-exists check; the required
syncs still follow. Four-host traces report this distinction separately.

The existing stable-private-namespace contract excludes replacing and rebinding
old directory identities. Missing prefixes never held this file; mkdir either
creates them or preserves an existing directory. Creation permissions are required.
Receipt safety proves durable file absence, **not durable existence of empty
placeholder directories**. New directory actions are not mechanically composed with
the unbounded Verus GC liveness theorem. Actual `gc_io` Result classification and
unlink/parent dispatch, plus the `directory_io` ancestor-barrier loop, are proved.
Native ErrorKind/path/handle binding, complete ancestor enumeration and aggregate
repair durability still need review.

## Fresh publication setup and cleanup obligations

`ENV-PREPARATION` isolates register/create/open effects from the write itself.
An error may leave an open catalog entry or an empty private file. Unique new
names, truthful unlink absence, stable paths and serialized catalog replay are
provider contracts; saving the descriptor **before registration**, retaining it
through errors, and repairing/syncing its parent are straw obligations. A zero
rollback offset is not justified for an existing append pack.

The new guard retires only a fresh private name. Once open/offset capture succeeds,
the actual generic `capture_prefix` preserves that same identity and switches to
normal write rollback. Existing packs retain their previous prefix on setup
failure. Successful cleanup can leave a sealed empty catalog entry even when
registration had no effect; later GC retires it. This costs metadata work but
does not leave an indefinitely open reservation. Failed cleanup fences reuse
until a live caller retries. A peer tombstone satisfies an uncertain prior seal.

`PublicationPreparation.tla` checks descriptor obligations, durable removal before
seal, no abandoned private name, and conditional resolution. It abstracts parent
repair and catalog effects, rather than mechanically composing their models.
`preparing` and `capture_prefix` have actual Rust proofs; guard placement, real
seek results, syscall identity and the complete adapter do not. Process death,
abandoned dependency holds and historical leftovers remain separate gaps. See
the [exact-binary record](../verification/measurements/20261023-publication-preparation.json).

## From a complete file to a successful return

`ENV-HANDOFF` separates caller behavior from algorithm obligations. After a failed response, another ID, another API with the same ID, or conflicting contents may arrive. straw must preserve the original description through staging, required standalone sealing, outer manifest staging and internal-owner release. Caller restraint is not an environment guarantee.

The old behavior fails within this allowed environment: staging takes effect but reports failure, another publication replaces the retry cache, and the original retry writes a second file while the old owner remains. The fix binds the complete description to API/ID until handoff completes, rejecting other requests before creating their holds. Peer GC may interleave; `seal()` cannot discard the description. Replaying a formerly acknowledged reference after release/GC does not create a new unresolved guard.

`PublicationHandoff.tla` checks preservation of the description and unreturned file, staging and internal-owner release at success, and completion under stable storage, weak fairness and original-request retry. Its environment negative changes only `EnvStagedRoot` to lie on successful staging. That demonstrates reliance on the catalog contract, not an observed deployment violation. The model starts after data durability; pending-file, complete dependency-graph, directory and process recovery models are not mechanically composed. Library return is not receipt of an RPC response. Explicit close/abandonment, holds created before a complete file and crashed-writer recovery remain separate gaps.

Export the layer independently:

```sh
python verification/runners/environment_manifest.py --contract ENV-HANDOFF
python verification/runners/environment_manifest.py --model PublicationHandoff
```

The [exact-binary record](../verification/measurements/20261023-publication-handoff.json) retains tests, four-host response-loss comparisons and a single 100 GB retry. Measurements add no guaranteed delay bound.

## Staging release no longer requires surviving manifests

`ENV-CONSUMPTION` allows a durable release with a lost response, GC removal of unowned manifests, and a later retry/reopen. The old implementation re-read the graph every time, so retries could fail forever despite recovered storage and fair scheduling. This is an implementation gap within the permitted environment.

The catalog now records exact direct edges in the same frame as staging release, reconstructing them on reopen and rebuilding graph/owners together after an unconfirmed frame is lost. An absent root owner alone is insufficient: a dependency protected by another owner may be staged again after root GC. Retry must still consume that staging without touching independent owners. Every child must have an existing or same-frame definition. This checks structural closure, **not correspondence to authentic manifest contents**.

The model abstracts node availability separately; actual shared packs, byte authentication, catalog/WAL adapters and full GC are not composed. New code cannot reconstruct pre-upgrade releases whose logs lack edges and whose manifests are gone. Surviving legacy manifests can be learned. Graphs/log history grow with unique references; no compaction or memory-bound proof is supplied. Latency measurements do not discharge these contracts.

```sh
python verification/runners/environment_manifest.py --contract ENV-CONSUMPTION
python verification/runners/environment_manifest.py --model StagingConsumption
```

The [measurement and proof record](../verification/measurements/20261023-staging-consumption.json) separates algorithm/environment counterexamples, metadata cost and four-host retries.

## Dependency owners during incomplete preparation

`ProvisionalHold.tla` consumes `ENV-PREPARATION`, `ENV-PUBLICATION-RETRY` and durable catalog response contracts: failed hold/release operations may already have taken effect. Failed preparations keep their dependencies for retry, including across other requests; explicit `seal()`/`close()` abandons these preparations before a complete publication exists. straw records every owner before issuing hold and must not overwrite earlier identities. Once cleanup starts, failed release preserves the current identity and subsequent operations finish the started cleanup first.

A complete file transfers its owner out of the preparation set into handoff responsibility; preparation cleanup must not remove it. Older failed preparations can still be cleaned on close. Internal pack rotation must not initiate cleanup. These are implementation obligations, not assumptions excluding bad inputs, subsequent requests or GC before close.

The model separates hold failure, failure before completion and lost complete-handoff responses, including another request after a failed preparation and cleanup on explicit termination. It abstracts the second request's input as available and uses GC only to check complete-handoff protection; arbitrary peers, graphs and shared packs are not composed. Verus proves the actual `holding::finish` clears the current cleanup descriptor only when the release-success flag is true and otherwise preserves it. The pending set, call placement and syscall binding remain unproved. Process death, lost guards and historical internal owners still require recovery.

Remembering a cleanup identity does not prove its hold is durable. An initial failed hold may have had no effect, so callers must retain source ownership until destination protection is established. The four-host lost-hold-response cut occurs after real successful sync; it does not generalize to arbitrary hold errors.

## Environment boundaries across workflow stages

`SharedRecordWorkflow` declares its preparation, handoff, consumption-graph, reader-response and GC-peer contracts separately. ReaderCount=4 matches this four-host execution; the payload/manifest nodes represent the two final files. Throughput and chunk p99 belong to measurements, not progress premises. `ReadResponse` abstracts a whole assigned-slice operation as success/failure; short/error outcomes cannot authorize release. It supplies no real I/O deadline or composition of all actual retries. When reader failures are permitted, the model proves a protected failed outcome, not subsequent recovery/reclamation.

## Local writer mutex and callback progress

`ENV-LOCAL-ACCESS` is a runtime boundary, separate from distributed catalog locking. Rust's [Mutex contract](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.try_lock) supplies an exclusive acquired guard and a nonblocking `try_lock` response. A Python fault callback is allowed to request access to the same writer, or wait for another caller that requests it. The writer cannot proceed until that callback returns. Requiring that all callbacks return without examining this dependency would conceal an implementation deadlock.

`WriterAccess.tla` keeps that dependency explicit. The old blocking strategy has an infinite waiting execution despite weak fairness; the fixed strategy returns busy without changing publication state, lets the callback return and then permits a later call. Poison on the later attempt is denied. Only changing `EnvironmentLocalAccess.tla` to grant a token while the outer writer holds one violates `ExclusiveAccess`, demonstrating the mutex assumption separately from the blocking-algorithm counterexample.

The generic Rust `access::decide` proof preserves the exact token only when supplied and not poisoned; it denies busy/poisoned inputs. This does not prove the standard mutex, its result-to-token mapping, GIL/FFI or complete adapter placement. Other catalog/coordinator/journal locks and arbitrary callback termination remain outside this model. The two-second watchdog used by the probe is an observation window followed by explicit termination of its own child, not an environment latency bound or evidence that JuiceFS is slow.

## Committed cleanup and subsequent staging

`ENV-COMMIT-CLEANUP` is a lower-layer contract consumed by the queue; its provider includes straw's own catalog and recovery code. Durable WAL acceptance can be followed by a failed staging release and a same-ID retry or fresh recovery. Release can persist despite an error response, after which a publisher may restage the identical immutable reference.

The algorithm must place `StagingHandoff` intent in the accepting WAL frame and couple release, dependency recipes and `consumed_handoffs` receipts in one catalog frame. Recovery and cached responses pass the pending cleanup barrier. Completed IDs only consult receipts; unfinished work keeps its protecting queue owner. Explicit `release_publications` still releases the requested graph, separately from a queue commit's single handoff.

The reproduced leak is an algorithm/adapter defect: allowed environment behavior exposed missing tracking and retries. A lower layer falsely reporting or losing a confirmed receipt violates the provider's contract, but still requires distinguishing catalog replay from storage durability. `CommitCleanup.tla` checks separate algorithm mutations and an environment-only false receipt; a generic test-failure label cannot establish attribution.

The finite model starts with one retained root and includes cleanup errors, lost responses, recovery, later staging, an independent reader and GC. Arbitrary graphs, simultaneous queues, pre-WAL failures and complete process recovery are not composed. Verus proves the actual pending removal and receipt insertion helpers, not confirmation flags, ID/byte binding, call placement or the entire I/O adapter. Unmarked legacy WAL cannot safely distinguish unfinished cleanup from subsequent staging; upgrade does not automatically remove that historical ambiguity.

## Queue recovery, remote reads and GC

`QueueRecordWorkflow.tla` connects accepted-but-unfinished staging cleanup, completed handoff history, input leases, cancellation, recovery, reader acknowledgement, GC and recovery after reclamation. `EnvironmentReaders.tla` isolates `ENV-PEERS`: callers may acknowledge only readers whose actual reads have ended. Cancellation, expiry or a new epoch does not prove this. The application/supervisor supplies that evidence; straw must preserve the durable reader roots until identity-matching acknowledgement.

Distinguish lost reader roots during cancellation/recovery (an algorithm error) from premature caller acknowledgement (a lifecycle-contract violation). An environment-only eligibility mutation violates `NoLostReaders`. The real probe acknowledges only after checking successful remote process exit, shard identity and byte coverage. Its stdin protocol is not a production distributed-fencing/RPC proof. Coordinator recoveries on the four hosts are strictly sequential after the preceding process exits.

Positive configurations use 1/2/4 readers and the measured workload's 0/128/1024 completed historical handoffs. Each historical step records the distinct ID of an already successful handoff; reader recovery and post-GC recovery check every ID. JSON bytes, allocations and persistence time are not modeled; latency and memory remain measurements. A read must succeed before its running obligation ends, and acknowledgement retires its durable root. The failure-enabled configuration can terminate failed while retaining data, with no eventual-GC claim after read errors. Actual syscall/network termination, call placement, locking and WAL/catalog refinement remain unproved; weak fairness and measured p99 do not discharge these obligations.

Review the contract and its composed consumer separately:

```sh
python verification/runners/environment_manifest.py --contract ENV-PEERS
python verification/runners/environment_manifest.py --model QueueRecordWorkflow
```

## Range responses, authentication and copying

`RangeRead.tla` separates complete/short/error responses (`ENV-READ`) from chunk authentication (`ENV-AUTH`). Its new `EnvironmentAuthentication.tla` accepts only equal observed/expected sequences. SHA-256 implementation and the binding of expected checksums to immutable references remain unproved; other consumers' existing authentication abstractions do not automatically acquire a refinement proof.

straw must authenticate every touched chunk before returning exactly the requested bytes. Corruption outside the returned slice but within a touched chunk must also fail. The 1 MiB scratch policy, chunk traversal, copy offsets, checked-byte accounting and withholding partially copied output on errors are algorithm obligations. Full-chunk reads of a repeated pattern do not distinguish all offset errors; the new probe uses global-word-position data and separately checks returned bytes.

The model explores every valid interval within each configured length, including empty intervals. It checks exact output, authentication/checked coverage, corruption rejection, no return after failure, no spurious error on fault-free input and bounded scratch; weak fairness gives attempt termination. The [record](../verification/measurements/20261024-positioned-range.json) covers seven positive configurations, nine algorithm controls and one environment-only authentication control. Small integers bound interval exploration; 100 GB, 256 MiB and measured p99 do not become environment guarantees.

This is one attempt after validated reference/envelope input, while data remains protected. Real seek/read binding, indefinitely blocked syscalls, shared retry budgets, concurrent GC and full Rust-loop composition remain unproved. The actual `bounds::clip`, `next_buffer` and scratch-capacity Verus proofs are separate. The measurement harness is not Verus-proved production code.

## Tensor checksum granularity and environment boundaries

`TensorGranularity.tla` checks the new producer's fixed chunk policy separately from legacy encoded sizes. Fixed 4 MiB is a **straw algorithm choice**, not behavior required of JuiceFS or a latency bound. The model enumerates length/range/digest-count/span arithmetic only. It performs no I/O and introduces no assumption that a 4 MiB read is atomic.

`ENV-AUTH` still requires the metadata used for a read to be bound to the immutable record. The reader must honor the encoded size rather than reinterpret old checksums using the new size. `ENV-RUNTIME` retains trusted integer/library specifications and allocation limits. Verus proves the actual `bounds::authenticated_span` contract for all u64 inputs: validity, empty ranges, coverage, tail clipping and extra authenticated-byte bounds. This does not prove the Python producer, digest-list generation or full I/O loop.

For review, a new producer enlarging chunks and violating the bound is an algorithm-policy issue. Additional reads under a legacy encoded size are a known compatibility cost. Incorrect metadata/record binding requires investigation of the adapter or `ENV-AUTH` provider. Different timings for the same byte count belong to deployment measurements under `ENV-PERFORMANCE`. Metadata growth, retry multipliers and physical backend reads are outside the per-attempt payload bound. See the [granularity measurement record](../verification/measurements/20261024-tensor-granularity.json).

## Typed references and payload allocation boundaries

`ENV-TYPED-DECODE` requires adapters to faithfully supply the authenticated envelope and expected dtype, shape and length. Its providers are straw's Python/PyO3/JSON/index adapters; a violation does not directly establish a JuiceFS defect. `EnvironmentTyped.tla` also permits payload allocation success or failure. It supplies no numeric memory-capacity guarantee and excludes metadata allocation failures, process aborts and syscalls that never return.

`TypedRead.tla` separates metadata, field comparison, payload allocation/read and return. A mismatch must be rejected before payload work; empty slices cannot succeed unconditionally. Skipped/late comparison, early empty returns, rejection of valid requests and endless retry of a fixed mismatch are algorithm mutations. Substituting matching fields only in the environment is a separate control. Weak fairness concerns modeled actions, not I/O durations or measured p99.

Actual Rust `typed::matches` proves field equality in both directions, including arbitrary-length shape vectors. JSON decoding, Python shape-product checks, call placement and complete I/O refinement remain unproved. Regressions and four-host measurements exercise particular adapter executions; see the [record](../verification/measurements/20261024-typed-preflight.json). Metadata must still be read, authenticated and parsed. Early rejection concerns payload allocation/read after a typed mismatch is identified.

## Descriptor and retry boundaries within a read

`ReadAttempt.tla` separates Open, Index, Envelope, Payload, failed-attempt close and Retry. `ENV-VIS` still supplies fresh-open views, and `ENV-AUTH` still requires cached indexes to be authenticated and bound to the same immutable reference. Seeing an index does not guarantee visible payload. `ENV-FAIR` advances abstract steps without proving syscall return or a duration bound.

Reusing a descriptor within an attempt is straw policy. `SingleOpen` limits each outer attempt to one successfully acquired data-file descriptor: a checkable cost property, not a filesystem contract. Model Open abstracts successful acquisition, not failed underlying open syscalls. Actual `open_committed` may call File::open five times for NotFound, waiting 10, 20, 40 and 80 ms between calls; these name-lookup retries and other open errors are outside this model. That 150 ms sleep budget is separate from the 3.85-second extent-visibility sleep budget; neither is an API deadline. The old double-open variant violates this property without demonstrating unsafe returned data. Other checks cover release between attempts, authentication before return and success within the visibility attempt bound; exhaustion and persistent corruption may fail. Ordinary, cold-session and warm-session reads are modeled, but arbitrary API histories, real cache-key comparison, concurrent GC and complete byte parsing are not.

Rust `inspect_opened` passes this attempt's file together with its checked index to frame reading; `Reader.last` still stores only reference/index values. Actual `bounds::frame_position` is proved for every u64 input: return the exact absolute start iff extent addressing is representable and the entire frame fits the extent, otherwise reject. It proves neither binding of those values to a file nor OS support for every u64 seek. Real File transfer, Drop, open counts and fresh-open retries remain I/O-adapter obligations without full refinement; native regressions and call counting supply execution evidence.

## Separate name-lookup and data-visibility budgets

[`EnvironmentLookup.tla`](../verification/storage/tla/EnvironmentLookup.tla) declares `ENV-LOOKUP`: after namespace resolution, a **returning** read-only open acquires a descriptor or reports NotFound/another error. NotFound need not mean permanent absence. `NameBy` is a conditional local lookup-attempt bound within each data attempt, not milliseconds or a deployment guarantee. Other errors may occur at any attempt. Real namespace resolution, error classification, descriptor identity and syscall return remain adapter/environment boundaries.

[`ReadLookup.tla`](../verification/storage/tla/ReadLookup.tla) combines these responses with `ENV-VIS` and weak fairness. The actual policy allows five File::open calls, sleeping 10/20/40/80 ms after the first four NotFound responses. Other errors propagate immediately. Retryable validation failures after acquiring a descriptor use a separate 100/250/500/1000/2000 ms schedule. An outer retry resets the lookup budget, not the outer budget.

For the current single-descriptor ordinary/cold/warm-session read paths, the combined bound is **30 File::open calls and 4,750 ms of requested sleep**: `6 × (10 + 20 + 40 + 80) + 100 + 250 + 500 + 1000 + 2000`. Namespace resolution, I/O, authentication, allocation and scheduling time are excluded; this is not an API deadline. Success requires `NameBy <= 4`, `VisibleBy <= 5`, no other errors and no persistent corruption. Finite retries may fail without those premises.

Data validation is one abstract step here; `ReadAttempt` separately checks index/envelope/payload ordering. These are not full refinement or composition proofs. `ENV-LOOKUP` does not strengthen initial catalog discovery or establish trustworthy GC unlink NotFound. Export its review with `python verification/runners/environment_manifest.py --contract ENV-LOOKUP`. Controlled injection and natural name observations are recorded separately in [FILESYSTEM](FILESYSTEM.md#combined-name-lookup-and-data-retries-2026-10-24); injected failures are not natural cache statistics.

The modeled open response is explicitly the **Rust `File::open` return boundary**. [Rust 1.90's Unix implementation](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/fs/unix.rs#L1185) uses [`cvt_r`](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/pal/unix/mod.rs#L310) to retry EINTR internally. Thirty File::open calls therefore do not bound libc opens, kernel syscalls or RPCs. Persistent interruption need not satisfy the returning-call premise. A separate finite-EINTR experiment records each call; it sends no actual signals and proves no termination under infinite interruption.

## Offline cleanup: namespace and stopped-run premises

`ENV-OFFLINE` isolates the administrative tool's environment. Offline cleanup supports only a single queue without `storage.log` or `queues` markers. Other checkpoints/queues cannot be enumerated from the root queue's journal alone. Advisory stat may falsely report absence; fresh read-only open NotFound must still truthfully mean the marker is absent. Other errors establish no absence. Complete authenticated root closure and stable namespace bindings separately depend on `ENV-GRAPH`/`ENV-NAMESPACE`.

Shared markers or owners may appear after a report but before cleanup begins, so cleanup must check again. `all_participants_stopped=True` is a caller-supplied premise, **not evidence that this tool established quiescence or fencing**. Participants, markers and path bindings must stay stable from final inspection through unlink/directory barriers. Failed deletion may already remove unowned files and need a fresh report before retry; no rollback is promised.

[`EnvironmentOffline.tla`](../verification/offline/tla/EnvironmentOffline.tla) separates unreliable stat, trustworthy fresh-open absence and deletion failure effects. `OfflineCleanup.tla` checks historical root references, independent shared references, both guard checks and the stopped-run requirement. The full closure is a boolean abstraction. Python JSON/traversal, report equality, path resolution and actual syscall adaptation are not mechanically verified; this model does not replace online GC ownership/receipt models.

The old `inspect.py` treated a false `Path.exists()` result as authorization to assume no shared storage. Even with every participant stopped, checkpoint and peer-queue references remain valid and could be deleted. The fix uses fresh open without reading marker contents, refuses present markers and non-NotFound errors, and retains full reinspection in `cleanup_orphans`. An environment-only control still violates `NoOwnedDeletion` if fresh open itself lies about absence. This neither proves JuiceFS fresh-open fidelity nor repairs previously deleted data.

Use `python verification/runners/environment_manifest.py --contract ENV-OFFLINE` to review providers/obligations, or `--model OfflineCleanup` for its cross-layer premises. The [private-pool syscall comparison](../verification/measurements/20261024-offline-guard.json) identifies the tool's interpretation error under an already allowed response, not a naturally observed JuiceFS stat false negative.

## Offline references: data identity and resolution bindings

`ENV-OFFLINE-BINDINGS`, in [`EnvironmentBindings.tla`](../verification/offline/tla/EnvironmentBindings.tla), separates a complete path-resolution trace from `ENV-NAMESPACE`'s final identity. An accepted file alias can resolve to the same pack under a different name; a chain or directory alias can itself match the cleaner's `*.partial`/`*.sealed` enumeration. A symlink target containing `..` can traverse a directory absent from the final canonical path. Retaining only the final pack and its ancestors misses that dependency.

The environment supplies truthful successful `readlink` targets and EINVAL classification, complete traces and stable external bindings. **Stability excludes external rebinding; it does not assume that straw preserves aliases.** The model explicitly lets cleanup unlink names and checks that all accepted bindings survive. The implementation must protect every traversed binding, apply the check to partial files too, reject uncertain/escaping candidate identities, and unlink the selected lexical name rather than its resolved target. Successful identity/trace caches last only for one inspection. A trace failure forbids deletion; it is not interpreted as absence of ownership.

Use `python verification/runners/environment_manifest.py --contract ENV-OFFLINE-BINDINGS` or `--model OfflineAliases` to review this boundary. The environment negative intentionally omits intermediate bindings and produces `AcceptedReadable` failure. This supplied finite trace is not a POSIX resolver proof. Python path expansion, graph/WAL decoding, report comparison and syscall correspondence remain unproved. Hard-link/bind-mount identity and external rebinding are outside the existing namespace contract. The inspector's 40-symlink expansion cap is a rejection guard, not a visibility or latency bound.

The [four-host comparison](../verification/measurements/20261025-offline-alias.json) uses valid aliases created before queue acceptance, with no filesystem fault injection. This is an implementation omission under the allowed environment, not evidence that JuiceFS violated the contract.

`ENV-REPLAY-APPLY` now separates the proved catalog raw-history adapter from the remaining decoder obligation. `catalog_target::Apply` projects actual joint log history and preserves its invariant; `Decoder` has no assumed business semantics. Review raw JSON/run/path-to-operation meaning independently from Source freshness/sync, and from outer catalog locking/open. The actual rebuild/replay/barrier/confirmation composition is proved under Device. Journal Target remains unproved.

`ENV-REPLAY-APPLY` also requires complete application on success and unchanged context (the catalog confirmation floor) on every return. The actual catalog Target proves these clauses; `Apply::recover` proves rebuild/byte-recovery/confirmation composition. Native Decoder::reset/cache correspondence, outer lock/fresh-open and Source primitive meaning remain separate review boundaries.

The existing ENV-JSON-ENCODE/ENV-JSON-VALUE-CONSISTENCY laws now also support actual queue journal encoding and successful replay observations. straw proves validation-before-write and exact event-to-payload/frame binding; selecting the real recovered prefix, native File/run comparison and full business replay remain its own obligations.

`ENV-CROSS-COMPOSITION` separates the multi-request durable-effect abstraction from the native implementation. Filesystem deletion restoration and sync are external premises; complete immutable closure recipes, queue/staging/holder namespace separation, accurate live snapshots, receipt reconstruction and lower-effect linearization belong to straw. The model admits lost apply/reply and repeated recovery without combining queue/catalog durability into one transaction. Its Verus name-restoration/deletion predicates are in the shared environment module; TLA premises are in EnvironmentCrossComposition. A negative environment that can erase a protected durable name violates protection in both tools. No native mapping or liveness theorem follows merely from this inventory entry.

`ENV-GC-BATCH-EFFECTS` now separates actual `gc_receipt` encoding/append/reclaim correspondence from physical absence. The function preserves the exact canonical target set and permanent tombstones; it does not establish that native unlink/parent repair/barriers actually made those names durably absent. Review target-to-world binding and the already-recovered catalog context separately from receipt serialization.

`ENV-JSON-CANONICAL-CONTENT` separately records four pinned `serde_json::Value` library laws: canonical contents survive successful parsing of the library’s own serialization at corresponding tree nodes; array, string and unsigned-number constructors preserve equal canonical inputs. This boundary uses sorted maps and `float_roundtrip`. It does not equate Value equality or the weaker JSON observation model with serialized bytes: positive and negative zero are a counterexample. Actual handoff construction and recovery remain straw obligations; the finite codec regression does not verify serde internals.
