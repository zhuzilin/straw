# Filesystem deployment

[中文版](FILESYSTEM_zh.md)

straw uses a **mounted Linux/POSIX filesystem**, with real files, directories,
append writes, file/directory fsync and advisory file locks. It does not send
object-store API requests or replace a filesystem client. Local filesystems
work for one machine; several machines need the same shared pool contents.

## 0.1.4 candidate performance comparison

The [release comparison](../verification/measurements/20261026-release-comparison.json)
ran 0.1.3 and the 0.1.4 candidate on one host and the same JuiceFS mount, in
baseline/current/current/baseline order with a fresh root each time and online
GC enabled. Each round published and authenticated 1 GiB (32 × 32 MiB), appended
500 WAL frames, replayed a 2,000-frame catalog, and exercised task acceptance,
retirement and collection. WAL and catalog recovery each ran five times.

| Metric | 0.1.3 | 0.1.4 candidate | Change |
|---|---:|---:|---:|
| Publication throughput | 134.93 MiB/s | 107.00 MiB/s | −20.7% |
| Authenticated read throughput | 314.18 MiB/s | 322.91 MiB/s | +2.8% |
| 500 synchronous WAL appends | 8.395 s | 8.094 s | −3.6% time |
| Acceptance of 64 tasks | 2.843 s | 3.832 s | +34.8% time |
| Collection of 24 packs | 153.84 ms | 161.28 ms | +4.8% time |

Values are medians across two process rounds per build; the GC value first
takes the median of three collections per round. All phases passed. Caches
were not evicted, so immediate reads and repeated recovery may use warm client
and server caches. These samples establish neither peak multi-host bandwidth
nor a latency bound. The publication and task-acceptance regressions are visible
in both current rounds. This compares complete builds, including changed
durability and writer-fencing work; no profiling was performed to attribute
the differences. The linked record retains every sample and the exact wheel,
native-library and probe hashes. Historical measurements below keep their own
build and workload scope.

## 0.1.4 candidate: one 100 GB record across four nodes

A separate [100 GB run](../verification/measurements/20261026-release-100gb.json)
used one writer node to publish a record containing 100,000,000,000 position-dependent payload
bytes. Four readers divided the record and jointly checked every byte against
an independent expected-value calculation. This is a different workload from
the 32 MiB publication comparison above; it is not a version-to-version result.

| Stage | Observed result |
|---|---|
| Checksum preparation before publication | 113.501 s, excluded from publication timing |
| Single-node publication | 358.855 s; 265.755 MiB/s |
| Four-node read and byte-oracle wall time | 55.275 s; 1,725.333 MiB/s combined |
| Per-node native full-read throughput | 593.8–625.8 MiB/s |

Publication still includes regenerating the positioned payload, authentication
and synchronization. Read wall time includes orchestration, boundary slices
and the byte oracle; per-node native timings exclude the subsequent oracle.
Caches were not evicted, so these logical rates are not cold-read or backend
bandwidth ceilings. One publication supplies no publication p99.

After ownership release, normal GC reported one reclaimed file and
100,003,195,567 bytes, including framing and metadata. This confirms durable
deletion from the filesystem namespace; it does not measure when an underlying
object store physically releases that space. The linked record identifies the
exact probe source and native binary.

## Four-node visibility sample

The [visibility probe](../verification/measurements/20261026-release-visibility.json)
used one writer and three remote readers, with 1,000 publications of 64 KiB per
mode. All 6,000 reads succeeded without failed attempts. The empirical p99 from
fsync return to validated reply was 9.515 ms while keeping the writer open and
9.035 ms after closing it. This includes notification, fresh open, read, hash
validation and reply; it is neither a pure filesystem visibility delay nor a
guaranteed latency bound. Both modes used the same mounted deployment, without
cache eviction. All probe workers exited and their temporary data was removed.

## Required contract

| Operation | Required meaning |
|---|---|
| File fsync | A successful acknowledgment makes preceding bytes durable under the deployment's stated failure model |
| Directory fsync | New names and deletions can be made durable; failures are not ignored |
| Reopen after another client's sync | A reader can obtain the committed extent/log prefix |
| Cross-client advisory lock | All catalog writers/collectors serialize on the same stable inode |
| Single queue owner | The supervisor prevents two coordinators from writing one queue WAL |
| Immutable file identity | Pack names are unique, never reused or modified after sealing |

The catalog is one fixed `storage.log` inode. Do not replace/rename it while
clients exist. A transaction holds its lock while reopening a descriptor for
fresh I/O. Locks alone do not invalidate every filesystem cache.
Stale attributes after a remote unlink are permitted: deleting an already
tombstoned file can return ENOENT and is idempotent. Other errors are fatal.

All coordinators should use the same canonical absolute pool mount path;
queue owner identities currently include their control-root path. Relative
record references can be read through another mount root, but this is not
a supported way to move a live coordinator's ownership namespace. Rebuild
local `TensorRef` descriptors against the reading store when rebasing.

For defensive handling of deployments that expose stale or inconsistent views,
extent reads tolerate independent delays in length, header, index and data using
bounded reopen/revalidation. This extends beyond the default JuiceFS contract;
it is not a claim that every correct deployment exhibits these delays. Success requires a correct view within
that budget; eventual visibility without a bound does not suffice. WAL/catalog
recovery still requires a complete committed prefix on reopen under the lock;
do not extend the extent-read cache model to log-tail truncation. See the
[TLA+ assumptions](FORMAL_VERIFICATION.md).

## JuiceFS and other distributed filesystems

Use a direct client mount and qualify its lock, sync, recovery and visibility
semantics on the actual client version and storage service. In the explicit
JuiceFS profile, provide a deployment declaration:

```python
from straw.backend import FilesystemBackend
from straw import SharedFilesystemStore

root = "/shared/new-job"
backend = FilesystemBackend(root, profile="juicefs", declaration={
    "direct_mount": True,
    "writeback": False,
    "open_cache": 0,
    "readdir_cache": False,
    "client_version": "your deployed version",
    "durability_description": "your verified metadata/object-store durability contract",
})
store = SharedFilesystemStore(root, "new-job", backend=backend, online_gc=True)
```

These values are **operator declarations**, not discovered mount settings.
Replacing a placeholder string with a version does not qualify the service.
The default `local` profile uses the same POSIX calls without a JuiceFS
declaration; its name is not evidence of the mounted filesystem type.

Run the [multi-client crash/GC checks](VERIFICATION.md) on an isolated directory
before trusting a deployment. Process SIGKILL leaves the kernel and storage
service alive; it does not establish power-loss, metadata-service failover,
disconnected-client or reordered-device-write guarantees. Filesystem/vendor
qualification must cover those failures separately.

See the primary [JuiceFS cache documentation](https://juicefs.com/docs/community/guide/cache/),
[POSIX compatibility notes](https://juicefs.com/docs/community/posix_compatibility/),
and [Linux fsync contract](https://man7.org/linux/man-pages/man2/fsync.2.html).

`FilesystemBackend` supplies deployment declarations and test fault hooks;
payload I/O runs in Rust. Subclassing it does not implement an arbitrary cloud
backend. A new I/O backend must implement and validate the same durability
contract in the native layer, or define a different protocol explicitly.

## Visibility contract and measurement

The official [community cache documentation](https://juicefs.com/docs/community/guide/cache/)
describes default close-to-open consistency and says fsync/fdatasync/close flush
writes for other clients. The [cloud/enterprise cache documentation](https://juicefs.com/docs/cloud/guide/cache/)
likewise describes close-to-open consistency and upload-before-metadata-commit
when writeback is disabled. Enabling open cache weakens immediate cross-client
freshness; enabling writeback can expose metadata before upload, with reads
blocking or failing. Neither document specifies arbitrary zero-filled gaps after
a successful sync as normal default behavior. Treat the TLA+ region-gap model
as conservative fault tolerance; do not infer a JuiceFS implementation defect
from a simulated counterexample. Actual client version and mount configuration
must be recorded before attributing a failure to the documented contract.

A bounded raw-read probe is available (Python standard library only):

```sh
python verification/storage/probes/extent_visibility_probe.py \
  --parent /shared/test-tmp --samples 1000 --output /tmp/visibility.json
# Distinct SSH clients with the same shared path; uses configured SSH trust/auth.
python verification/storage/probes/extent_visibility_probe.py \
  --parent /shared/test-tmp --hosts HOST_B HOST_C \
  --samples 1000 --output /tmp/visibility-cross-client.json
```

It creates two private append-only packs, using straw's header/frame/index/footer
layout and per-publication fsync, and primes readers' tail/EOF caches before each
append. It compares a kept-open writer with a close/reopen writer. Notifications
travel over pipes/SSH **after** fsync; no filesystem marker hides propagation
latency, and native straw retry sleeps cannot quantize the measurements. Each
reader independently reopens and checks length, header, footer, index and the
full extent hash. There is no fault injection, cache eviction or mount change.
The default run writes approximately 128 MiB; `--max-write-bytes` defaults to
512 MiB and explicitly bounds the run. `--start-offset` places the actual extents
after a sparse hole (up to 8 TiB); it tests addressing, not full-volume I/O.
Only the fresh private directory is cleaned after workers stop; a failed remote
run retains its directory because SSH disconnect is not proof of remote exit.

`fsync_to_ack_ms` is measured using one coordinator clock, so it needs no host
clock synchronization. Its p99 is an **observed upper bound** on visibility:
it includes notification, scheduling, read, hashing and reply latency. Replies
are collected in reader order, which can inflate later readers' upper bounds.
`reader_probe_ms` excludes those control-channel delays. Failed attempts and
checksum/region failures are reported separately; successful-read quantiles do
not hide timeout counts. With no stale reads, report “not observed”, not an exact
zero propagation delay. Local processes share one mount/client; their p99 is not
a cross-client p99. Fresh paths are not cold backing storage. Neither measured
p99 nor maximum is a worst-case guarantee or a TLA+ liveness assumption.

## File-count and space planning

- A long-lived writer appends publications and embedded manifests to a pack.
  Default rotation target: **1 GiB**. No temporary file or sidecar manifest is
  created per sample. `publish_many` reduces transaction overhead too.
- Each writer incarnation has its own directory and current pack. Several
  writers increase partial-pack count. Creating a writer or sealing for every
  sample defeats packing; keep writers at process/worker scope.
- Rotation happens before the next publication would exceed the target. One
  oversized publication stays contiguous and may exceed it. Choose record and
  buffer limits as well as the target. The target is not a quota.
- Each queue has `run.json` and one `control/journal.log`. Namespaced queues
  share data packs and one pool `storage.log`. User owner names are log entries,
  not files. Optional traces add one append file per process.
- GC deletes only entirely dead sealed packs. One live record can retain the
  whole pack. Separate very different retention lifetimes into writer streams
  where useful; there is no relocation/compaction of live records yet.
- Crashed writers can leave open packs/staging retained. Empty writer directories
  and WAL/task history also grow with job lifetime. No timeout proves them dead.

Estimate retained tensor bytes from shapes/dtypes and active retained versions,
then include framing, partial packs, checkpoints, readers, queued results and
WALs. Pack count is roughly data volume divided by pack target plus partially
filled writer packs; this is not a strict mathematical bound because publication
sizes and explicit sealing vary. Queue logical limits may count shared data
more than once and do not cap physical usage.

Set application admission/retention budgets and filesystem quotas. The benchmark
has explicit `--max-files` and `--max-gib` guards. Use one fresh pool per bounded
job; stop every participant before deleting that pool. Never delete pack files
directly to enforce a quota while references remain live.

## Throughput qualification and measured parameters

Correctness proofs do not establish bandwidth. Qualify small operations by
latency/operations per second, and large publications by sustained payload
throughput, memory and read amplification. Define a workload and target before
calling a deployment fast enough: record sizes, publication batching, number of
clients/streams, fsync boundary, read fanout and cache policy all matter.

The standard-library raw baseline separates synced writes, cross-client raw
reads, and checked reads (the latter can reuse the first read's caches):

```sh
python verification/storage/probes/filesystem_bandwidth_probe.py \
  --parent /shared/test-tmp --hosts HOST_A HOST_B HOST_C HOST_D \
  --mib-per-host 2048 --streams 4 --sync-mib 16 --output /tmp/raw.json
python -m straw.benchmark run --root /shared/new-queue-benchmark \
  --hosts HOST_A HOST_B HOST_C HOST_D --writers 4 --readers 4 \
  --record-bytes 16777216 --gib 8 --max-gib 10 --max-files 128 \
  --segment-mib 256 --report /tmp/queue.json
```

The queue benchmark includes publication, coordinator acceptance, and overlapping
checked ring reads. Its write/read rates use the same end-to-end duration; they
are not isolated device write/read bandwidth and should not be added as a claim
of storage capacity. Fresh-path tests without cache eviction do not certify cold
object-store performance.

The [2026-10-23 measurement manifest](../verification/measurements/20261023.json)
records source/binary/report hashes and archived raw-report locations. Four
JuiceFS clients performed 10,800 synced publications and 32,400 cross-client
checked visibility probes, with no stale view or retry observed. The initial
64 KiB matrix rotated all four writer hosts; additional size/offset cases used
one writer and three remote readers. Actual client version and writeback/open
cache settings were not discoverable, so this is not vendor certification.

| Payload / starting offset | Publications per mode | fsync-to-checked-reply p99, open / close-reopen |
|---|---:|---:|
| 2 KiB / 0 | 1,000 | 10.38 / 8.16 ms |
| 1 MiB / 2 GiB − 5 | 200 | 15.52 / 13.46 ms |
| 16 MiB / 4 GiB − 5 | 100 | 57.73 / 46.05 ms |
| 2 KiB / 2 TiB − 5, sparse boundary only | 100 | 3.38 / 14.04 ms |

These numbers calibrate test cases and performance expectations. They do **not**
set `VisibleBy`, prove a maximum delay, or justify shortening retry sleeps.
The documented fresh-open scenario remains `VisibleBy=0`; defensive delayed-view
scenarios remain explicit fault assumptions independent of these observations.
The 100 GB run below replaces a proposed full-volume TB test; the sparse TiB
boundary check is retained solely for addressing coverage.

| Workload, four machines, 8 GiB total payload | Observed aggregate rate |
|---|---:|
| Raw synced write, 1 stream per host, fsync every 16 MiB | 982 MiB/s |
| Raw synced write, 4 streams per host, fsync every 16 MiB | 3,683 MiB/s |
| Queue + checked ring read, 16 MiB publication, 1 writer/reader per host | 369 MiB/s in each direction |
| Same queue workload, 4 writers/readers per host | 430 MiB/s in each direction |
| Queue + checked ring read, 64 MiB publication, 1 writer/reader per host | 739 MiB/s in each direction |

These are short calibration runs. The smaller gain from extra queue writers,
compared with larger publications, motivates profiling batching and serialized
control operations. It does not prove the bottleneck or authorize skipping
fsync/authentication. The raw and queue workloads differ, so their ratio is not a
controlled measurement of library overhead alone.

A separate native probe (`cargo build --release --example large_extent_probe`)
published **100,000,000,000 payload bytes in one real record**, then three remote
clients each checked the entire record using 256 MiB range results and at most
1 MiB verification scratch. Publication included envelope/segment hashing and
fsync: 277.20 s, 344 MiB/s, 10.9 MiB process peak RSS. Readers took 153.75–159.16 s
(599–620 MiB/s each); aggregate including SSH dispatch was 1,792 MiB/s. Peak
reader RSS was 260.6–260.8 MiB. All checks passed and the private data root was
removed. This is one publication sample, **not a 100 GB publication p99**.
The payload repeated a random 8 MiB pattern; generation and checksum-metadata
preparation were excluded. There was no cache eviction. The probe exercises
native storage APIs, not the Python tensor constructor or coordinator RPC.

### Filtered acquire latency on four clients (2026-10-23)

`verification/protocol/probes/lease_selection_probe.py` measured the fixed selection code on the
same four JuiceFS clients. Each client used its own fresh queue, 1,024 pending
tasks, batches of 16, and 250 acquisitions for each of two cases. The cases
alternate: a unique-ID filter and a shuffled filter containing four copies of
every ID. All 2,000 acquisitions returned 32,000 distinct-in-batch, current
leases; heartbeat, release and WAL recovery checks passed. Native hashes were
checked on every host. The private roots (13,268,772 total WAL bytes) were removed.

| Filter | Pooled samples | p50 acquire | p99 acquire | Maximum |
|---|---:|---:|---:|---:|
| 1,024 unique IDs | 1,000 | 17.09 ms | 41.62 ms | 71.70 ms |
| 4,096 entries / 1,024 distinct IDs | 1,000 | 18.45 ms | 44.07 ms | 155.18 ms |

Times include local Python/native control processing, filtering and durable
lease WAL append/fsync. They exclude network RPC transport, heartbeat/release,
queue preparation and recovery. This is metadata control-plane latency, not
payload bandwidth or an isolated measurement of deduplication CPU time. The
larger filter also changes serialization and sorting costs. The pooled p99 is
an observation under this workload; it supplies no hard deadline, storage
convergence bound, or fair-scheduling guarantee. Evidence and input hashes are
in [`20261023-selection.json`](../verification/measurements/20261023-selection.json).

### Historical tombstone cost before batching (2026-10-23)

The archived pre-batching `verification/gc/probes/gc_history_probe.py` grows each client's separate pool to 0, 32,
then 128 deleted packs and samples 250 subsequent GC calls per level per host.
Each pack held a 1 KiB record. Creation, release and first deletion are excluded;
these are repeated warm calls, each reclaiming zero additional files/bytes.
A separate untimed callback check confirms one directory sync per tombstone.

| Historical tombstones | Pooled samples | p50 GC | p99 GC | Maximum |
|---|---:|---:|---:|---:|
| 0 | 1,000 | 10.77 ms | 29.85 ms | 43.48 ms |
| 32 | 1,000 | 23.46 ms | 47.52 ms | 124.06 ms |
| 128 | 1,000 | 64.57 ms | 103.48 ms | 119.08 ms |

Timing includes catalog locking/refresh, stat and repeated parent-directory
sync; it does not isolate individual syscall costs. Levels run in increasing
order and cache/backend load are uncontrolled. The measured scaling motivates
GC history/barrier optimization, but skipping a barrier merely because a name
is absent breaks interrupted-deletion durability. No such optimization was
made in this measurement. There is no latency bound, TB-scale history claim,
or payload-bandwidth result here. All four hosts loaded the same verified native
hash; private data and temporary environments were removed. Full samples and
input hashes are referenced by
[`20261023-gc-progress.json`](../verification/measurements/20261023-gc-progress.json).

### Batched directory barrier comparison (2026-10-23)

A fresh comparison ran both the preceding per-pack-barrier binary and the new
batched binary on all four hosts, with separate pools/environments and native
hash checks. Each pool contains one writer directory. At each of 0, 32 and 128
historical tombstones, each host samples 250 repeated GC calls per binary.
Two hosts run the baseline first and two run the batched binary first. The same
probe validates the expected barrier policy (`--expected-barriers per-pack` or
`per-directory`); this option validates the benchmark, not a runtime GC setting.

| Tombstones in one directory | Baseline p50 / p99 | Batched p50 / p99 | Barriers per call, before → after |
|---|---:|---:|---:|
| 0 | 11.15 / 28.40 ms | 11.10 / 27.27 ms | 0 → 0 |
| 32 | 21.99 / 46.77 ms | 19.69 / 42.53 ms | 32 → 1 |
| 128 | 57.46 / 97.54 ms | 48.42 / 84.45 ms | 128 → 1 |

Each table cell uses 1,000 pooled samples. At 128 tombstones, pooled p50/p99 fell
15.7%/13.4%, but per-host p99 worsened on two hosts and the pooled maximum rose
from 127.09 to 261.71 ms. There is no demonstrated universal tail improvement.
The count reduction is deterministic for this directory layout; latency remains
sensitive to cache and backend load. Levels run in increasing order, and the
comparison is not simultaneous on the same host. Creation, first deletion and
probe callbacks are outside timing. Catalog refresh, historical stat/scanning
and local Python/native calls are included; no individual syscall cost is
isolated. Batching still scans every tombstone and does not solve unbounded
history growth or establish a latency SLA.

All 6,000 timed calls passed; 1,024 small packs were created/deleted during setup.
Private data and temporary environments were removed. Baseline and batched
binaries, per-host samples and hashes are retained through
[`20261023-gc-batch.json`](../verification/measurements/20261023-gc-batch.json).

### Durable receipt comparison (2026-10-23)

The next comparison uses the preceding batched binary and a new binary that
records durable reclamation receipts. Four hosts each run 250 repeated calls at
0, 128 and 1,024 historical tombstones, with separate pools/environments and
verified native hashes. Two hosts run each version first. Each pool has one
writer directory and one 1 KiB record per pack; levels accumulate in order.

| Completed tombstones | Batched p50 / p99 | Receipt p50 / p99 | Payload-parent barriers per repeated call |
|---|---:|---:|---:|
| 0 | 11.17 / 24.78 ms | 11.19 / 24.71 ms | 0 → 0 |
| 128 | 55.32 / 111.90 ms | 11.30 / 27.80 ms | 1 → 0 |
| 1,024 | 51.75 / 379.55 ms | 11.07 / 20.81 ms | 1 → 0 |

Each cell pools 1,000 samples. Separate untimed callback checks confirm zero
payload stat attempts, directory barriers and receipt appends for completed
work in the new binary. The old binary lacks the stat callback, so its stat
count is not reported as a measurement. Both versions still lock/refresh/sync
the catalog. These are warm control-call timings, excluding pack creation,
first deletion and probe callbacks, not payload bandwidth or cold replay.

The first nonempty collection adds a receipt append/sync. Four first-collection
samples per version are too few for a p99 or an isolated overhead estimate:
for 128 new packs, baseline/new ranges were 589–763 / 557–739 ms; for the next
896 packs, 3.81–4.46 / 3.68–4.15 s. Catalog growth during these calls increased
from 10,377 to 20,756 bytes and 71,817 to 143,636 bytes respectively. This is
GC-event growth, not the entire catalog. Empty repeated calls add no receipts.
Permanent tombstones and the log remain unbounded; cold replay and collection
capacity costs are not eliminated by removing completed paths from the indexes.

All 6,000 timed calls passed, after creating/deleting 8,192 small packs. A separate
four-host sequence passed new-write → old-read/collect/write → new-recover/collect
→ new-reopen checks with three additional packs, preserving a live reference
across the first three stages. It covers these two binaries, not all historical
versions. Private data and temporary environments were removed. Raw samples,
first-call costs, interoperation results and hashes are linked in
[`20261023-gc-receipt.json`](../verification/measurements/20261023-gc-receipt.json).
Cache/backend load was uncontrolled; improved measured tails are not a bound or
a guarantee for arbitrary history sizes. No new 100 GB/TB bandwidth run is
claimed for this control-path change.

### 100 GB rerun with checked native stream lengths (2026-10-23)

After correcting the native payload length/callback contract, the same
`large_extent_probe` source ran with the new core. All four hosts verified the
same executable hash. A 64 MiB pilot passed, followed by one real
100,000,000,000-byte record, then three concurrent remote full reads. This is a
new run, separate from the earlier bounds experiment.

| Operation | Observed result |
|---|---|
| 100 GB native publication, including envelope hashing/framing/fsync | 271.37 s; 351.43 MiB/s; peak RSS 10.75 MiB |
| Full authenticated read on each of three remote clients | 149.31–157.97 s; 603.70–638.71 MiB/s; peak RSS 260.63–260.83 MiB |
| Three-reader aggregate, including SSH dispatch | 1,806.29 MiB/s over 300 GB of returned and checked payload |
| Complete 256 MiB range requests, pooled across readers | 1,116 samples; p50 393.86 ms; p99 537.96 ms; max 592.58 ms |

The shorter final request on each reader is excluded from range percentiles.
These are sequential requests in three concurrent native Reader sessions;
samples are correlated, the authenticated index can be reused, and no caches
were evicted. Payload repeats a random 8 MiB pattern. Pattern generation and
chunk-metadata preparation are outside write timing. File length was
100,000,051,365 bytes including framing/manifest metadata; this is not a device
or object-store byte counter. There is only one large publication, so no 100 GB
publication p99, controlled before/after overhead estimate, Python/RPC bandwidth
or TB physical-I/O claim follows. The read percentile does not bound stale-view
convergence or total retry time. Both private test roots were removed.

Exact sources, binary, per-range samples and gates are recorded in
[`20261023-stream-length.json`](../verification/measurements/20261023-stream-length.json).

### Dependency depth and control latency (2026-10-23)

Four clients each created separate pools at depths 128, 2,048 and 16,384. Each
pool contained a 1 KiB leaf publication in one sealed pack and a valid chain of
small inline manifests in another. There were 50 validation samples per host
and depth, using the verified current wheel and a 512 KiB process stack. Each
call starts a new native Reader; its index is reusable within that call.

| Manifest-chain depth | Pooled samples | Validation p50 | p99 | Maximum |
|---|---:|---:|---:|---:|
| 128 | 200 | 49.01 ms | 96.57 ms | 170.27 ms |
| 2,048 | 200 | 810.28 ms | 1,178.05 ms | 1,262.77 ms |
| 16,384 | 200 | 6,273.88 ms | 8,010.08 ms | 8,222.98 ms |

All 600 calls passed. After sampling, each pool retained its chain root, released
staging, checked that GC kept both reachable packs and the leaf stayed readable,
then released the final owner and reclaimed both packs. Each depth has only four
first-root-retain observations: 87.75–100.92 ms, 533.34–1,107.95 ms and
5,605.34–7,147.86 ms respectively. Those are not retain p99s; the store already
has raw-reference/leaf closure caches and the preceding validations warm the
filesystem. The requested chain root has not previously been retained.

Pools are independent, levels ascend, four hosts run concurrently, and backend
load/cache state is not controlled. Validation times include local Python/native
calls, graph parsing, per-node file I/O and metadata authentication; they exclude
pool construction and GC and do not measure leaf-payload throughput. Removing
recursive call-stack growth does not make deep graph validation cheap, establish
constant heap use, or bound operation latency. Per-read visibility retry sleeps
also do not bound a traversal containing thousands of separate reads. Private
pools and remote environments were removed. Exact inputs, crashes from the
preceding binary, samples and gates are in
[`20261023-dependency-traversal.json`](../verification/measurements/20261023-dependency-traversal.json).

### Payload scans during checkpoint reactivation (2026-10-23)

`reactivation_probe` verifies the same native binary hash on four hosts. Each
host/size uses an independent pool with a durably retained checkpoint depending
on the payload publication, whose staging owner is released. Each host calls
the real `Catalog::retain_reactivated` 50 times, releasing the restored owner
between samples. The 800 timings include catalog locking/refresh, dependency
metadata validation, full payload SHA-256 and owner append/sync. Release/GC is
excluded, as are Python/RPC and the full Coordinator rewind API.

| Payload | Pooled samples | p50 | p99 | Maximum |
|---|---:|---:|---:|---:|
| 2 KiB | 200 | 37.36 ms | 82.76 ms | 91.41 ms |
| 1 MiB | 200 | 40.29 ms | 135.69 ms | 149.14 ms |
| 16 MiB | 200 | 51.64 ms | 100.16 ms | 140.31 ms |
| 64 MiB | 200 | 89.07 ms | 192.77 ms | 227.30 ms |

The first call opens a fresh catalog instance; later calls reuse catalog/closure
caches. Each scan creates a Reader. Filesystem caches are not evicted. Hosts
run concurrently on independent pools, so this does not measure same-pool lock
contention. Sizes increase over the run and backend load is uncontrolled;
percentiles need not grow monotonically. Peak measured process RSS was
3.65 MiB. That is an observation; only the 1 MiB payload scratch cap is
an algorithm contract.

Then the first host published one real **100,000,000,000-byte** `bytes.v1`
record and the second host performed one complete reactivation:

| Operation | Observation |
|---|---|
| Native publication including envelope hashing, framing, fsync and staging | 284.45 s; 335.27 MiB/s; peak RSS 10.63 MiB |
| Remote complete reactivation including verification and durable ownership | 107.45 s; 887.52 MiB/s; peak RSS 3.68 MiB |

The payload repeats a random 8 MiB pattern. Pattern generation and checkpoint
preparation are outside publication timing. Reactivation reads the entire
100 GB payload without allocating a returned payload. One large observation
does not establish p99. Verified bytes count logical data, not device/object
traffic. This differs from earlier tensor-range workloads and is not a
controlled performance comparison, cold-cache guarantee or TB I/O test.

In every pool, releasing the checkpoint lets GC reclaim its pack while the
restored owner protects the payload pack. Releasing the restored owner then
reclaims that pack. All 34 packs across 17 pools followed this lifecycle; private
directories and remote binaries were removed.

**In this measurement revision, complete verification still holds the catalog lock.** The memory fix removes
whole-payload allocation, not payload-dependent I/O or lock duration; a large
rewind can block other catalog operations in its pool. These timings are no
hard bound on lock waiting, recovery or visibility. Exact samples, binaries,
crash regressions and formal checks are linked from
[`20261023-reactivation.json`](../verification/measurements/20261023-reactivation.json).

### Unlocked reactivation with same-pool catalog contention (2026-10-23)

Four hosts checked identical native-probe and wheel-native hashes. The first
host published, the second reactivated, and the first/third/fourth ran peers.
Each peer durably retained a 1 KiB publication and repeatedly timed a retain
followed by release, including both catalog transactions. Each of three
experiments used its own pool; all five packs per pool were eventually GC'd.

The same binary and 2 KiB payload first compared the lower-level locked
`Catalog::retain_reactivated` path with `Config::retain_reactivated`, injecting
a two-second pause at verification start. This is a diagnostic pause, not an
observed filesystem delay:

| Path | Range of peers' first retain/release pair | Pairs observed before finish marker | Total reactivation including pause |
|---|---:|---:|---:|
| Locked compatibility path | 2247.32–2294.13 ms | 0 | 2.12 s |
| Unlocked path | 348.88–392.80 ms | 18 | 8.68 s |

Other clients made progress during unlocked verification. This does not show
that total recovery is always faster: that unlocked sample took longer.
The probe did not separately time initial catalog access, scanning and final
lock reacquisition, so the difference cannot be assigned to a single stage.
These small controls do not establish stable p99. Marker visibility can lag;
markers coordinate measurements, not synchronized clocks or temporal proofs.
Deterministic cross-process regressions separately check lock release.

The next experiment used **no injected pause**: publish one real
100,000,000,000-byte record, fully authenticate it, recheck ownership and commit
the restored owner while three peers operate on the same catalog:

| Operation | Observation |
|---|---|
| 100 GB native publication | 271.63 s; 351.09 MiB/s; peak RSS 10.77 MiB |
| 100 GB reactivation including both catalog transactions | 106.79 s; 893.00 MiB/s; peak RSS 3.64 MiB |
| Peer retain/release pairs | 1361 samples; p50 211.67 ms; p99 386.68 ms; maximum 433.56 ms |

Peers wait 10 ms between pairs and stop after observing the finish marker;
first catalog instances and subsequent warm calls are pooled. Load/cache state
is uncontrolled and caches are not evicted. This is not the complete Python/RPC
restore API. Payloads repeat a random 8 MiB pattern; pattern generation and
checkpoint preparation are outside publication timing. Throughput uses logical
payload bytes, not device traffic. One 100 GB restore supplies no restore p99,
cold-cache guarantee or TB physical-I/O result.

Coordinator's full payload scan now runs outside the catalog critical section.
Closure metadata walks, refresh, final checks and owner append still lock, and
the operation adds a catalog access. Loss of a source during scanning may
reject the restore. Measurements establish neither lock fairness nor a latency
bound. Protection/GC checks passed for fifteen packs in three pools; private
directories and remote environments were removed. Exact samples, binaries,
sources and checks are linked from
[`20261023-unlocked-reactivation.json`](../verification/measurements/20261023-unlocked-reactivation.json).

### Reactivation phase controls (2026-10-23)

To investigate the 8.68-second sample above, the native probe now records local
monotonic events for candidate inspection, full verification, final catalog
access, append, sync and return. Experimental marker writes and the injected
pause have separate intervals. Library code and models are unchanged. Four
hosts checked the same new probe binary and existing wheel/native hashes. Each
of sixteen fresh pools contained a 2 KiB payload. Contended trials alternated
locked/unlocked order; each operation still included an injected two-second
pause.

| Scenario | Samples per path | Locked total duration | Unlocked total duration |
|---|---:|---:|---:|
| No concurrent peers | 2 | 2.087–2.091 s | 2.107–2.137 s |
| Three same-pool retain/release peers | 6 | 2.073–2.140 s | 2.115–2.181 s |

Across the six contended unlocked calls, verification took 2.55–8.24 ms,
final catalog access 25.70–87.98 ms, and the owner append's sync 14.00–25.37 ms.
Final catalog access combines lock reacquisition, refresh, rechecking and append
preparation; it is **not pure lock waiting**. The locked path already owns the
transaction at this boundary, so those intervals represent different work.
Peers completed 239 retain/release pairs in unlocked trials, 224 before their
local observation of the finish marker; locked trials had 18 pairs and zero
such observations. Markers remain client observations.

The 8.68-second sample was **not reproduced** and remains unattributed. These
small controls establish no stable p99, lock-wait bound or new large-payload
throughput result. Numeric policy and environment premises are unchanged.
Protection/reclamation checks passed for 68 packs in sixteen pools; private data
and all four remote environments were removed and independently checked absent.
Raw events, samples, exact binaries and this run's verification scope are linked
from [`20261023-reactivation-timing.json`](../verification/measurements/20261023-reactivation-timing.json).

### Same-process recovery after catalog replay failure (2026-10-23)

This experiment deliberately injects a fault. A second catalog client writes a
complete owner frame and fails before sync; when the original client replays it,
a Linux `LD_PRELOAD` shim fails its next real catalog fsync once with EIO. The
probe then explicitly keeps or removes that unconfirmed tail and retries on the
same object. Every confirmed prefix byte is preserved. The shim targets only the
armed private log and is not linked into the library. This is not a JuiceFS
service power cut or an observation of real deployment data loss.

All four hosts checked old/new isolated wheels and actual native hashes. Each
trial created a fresh pool, published/sealed 2 KiB, and checked ownership and GC.
On every host, the old version falsely reported `UnsafeRecovery` after the tail
was lost; a new object could finish. Its surviving-tail controls worked. The new
version ran 25 lost-tail and 25 surviving-tail trials per host, **200 total**;
all retries completed on the original object. A surviving uncertain owner was
retained after sync and prevented GC until explicit release.

| New-version tail outcome | Samples | Retry p50 | Retry p99 | Maximum |
|---|---:|---:|---:|---:|
| Lost | 100 | 29.20 ms | 81.20 ms | 162.54 ms |
| Survived | 100 | 26.92 ms | 46.87 ms | 52.09 ms |

Timing covers only retrying `release_publications` on the same object: refresh,
any full replay, sync and release. It excludes injection, rewriting the tail and
subsequent GC. Hosts used independent pools; caches/background load were not
controlled. These samples establish no long-history recovery bound, normal fault
rate, bandwidth result or p99 SLA. All 208 old/new trial pools/packs passed their
checks and reclamation; private shared data and remote environments were removed.
Sources, exact binaries, samples and checks are linked from
[`20261023-catalog-replay.json`](../verification/measurements/20261023-catalog-replay.json).

### Empty-refresh failures and catalog history size (2026-10-23)

The previous cache fix rebuilt after every refresh error, including an empty
refresh that changed no cache state. Four-host comparisons use that already
corrected lost-tail binary as baseline. The new version requires rebuilding only
after possible tentative changes. Each host/frame count has an independent pool:
one native protected 2 KiB pack, followed by an offline legal alternating
retain/release history while no clients operate, then one final fsync. Both
binaries read identical history, with order alternated across hosts and sizes.
This is not per-event commit throughput.

| Added history frames | Catalog size | Baseline retry p99 after empty-sync failure | New retry p99 |
|---|---:|---:|---:|
| 0 | 1,170 B | 30.47 ms | 22.93 ms |
| 4,096 | 0.73 MiB | 37.07 ms | 19.33 ms |
| 65,536 | 11.66 MiB | 122.18 ms | 17.40 ms |
| 262,144 | 46.63 MiB | 449.53 ms | 17.68 ms |

Each cell pools 25 samples per host, 100 total. Timing includes only the retry
after a one-shot injected empty-refresh fsync EIO, excluding injection. New
catalog instances and ordinary warm calls also have 100 samples per version/size,
for 2,400 timed samples overall. At the largest size, first catalog load had
p50 411.16 ms baseline / 415.51 ms new; normal warm calls were about 11 ms in
both. **Necessary full replay was not accelerated**; redundant replay after an
empty failure was removed. A new catalog object is not a cold filesystem cache.
Caches were not evicted and background load was uncontrolled.

A separate local strace comparison used 4,096 added frames plus five native base
frames. Total catalog `read` calls fell from 36,915 to 24,612, removing exactly
`3 × 4,101 = 12,303`; actual successful fsync calls remained four. The traced
session includes first load, warm-object priming, checksum checks and one retry,
so these totals are not one retry's syscall count. The injected EIO returns from
the shim without a kernel fsync; traced timings are not used in the table. This
supports removing one full parse without removing successful sync barriers.

The new binary also passed 40 lost/surviving uncertain-tail trials across the
four hosts, including ownership and final GC. Sixteen history pools, forty fault
trial pools and the separate trace pool were reclaimed and removed. A single
live pack does not characterize large owner/tombstone scans, huge individual
frames, payload bandwidth or a latency bound. Counts are workload sizes, not API
quotas. Evidence: [`20261023-catalog-history.json`](../verification/measurements/20261023-catalog-history.json).

## GC stale stat and reclamation receipts (2026-10-23)

The [JuiceFS Community cache documentation](https://juicefs.com/docs/community/guide/cache/) describes configurable negative-lookup caching, disabled by default. This does not establish this deployment's mount settings or equate stat and unlink semantics. No mount options were changed.

Each of four rotating writers created 100 fresh 2 KiB files. The other three readers first primed a negative stat; the writer completed file/directory fsync and closed, then persistent SSH pipes instructed readers to query. All 1,200 first observations saw the file. Query latency was p50 0.255 ms, p99 9.245 ms, maximum 21.424 ms. Timing starts at the reader after notification, not at a synchronized cross-host publication instant. This yields neither a negative-cache convergence bound nor a throughput result.

A separate native-publication probe injected **one stat ENOENT** on a sealed, unowned pack; unlink and directory sync used the real filesystem. All eight old-binary cases across four hosts skipped unlink and wrote a receipt, leaving the file after repeated GC and fresh-client replay. The probe explicitly removed those private orphans afterward. All 100 fixed-binary cases actually deleted the file and passed repeated/reopened checks. First-GC timing, including tombstone, fault callback, unlink, parent sync and receipt, was p50 82.68 ms, p99 163.11 ms, maximum 203.34 ms. The incorrect old path performs less work and is not a performance baseline.

Stat now supplies diagnostic accounting; pending paths still require unlink. Successful deletion with unknown size increments `reclaimed_files` and adds zero to `reclaimed_bytes`, which is not a capacity audit. Trustworthy unlink NotFound remains an assumption. The fix does not establish unlink freshness under every cache configuration or repair existing erroneous receipts. Private data and remote environments were removed. Exact inputs, native hashes, individual samples and complete verification are recorded in [`20261023-gc-stat.json`](../verification/measurements/20261023-gc-stat.json). No 100 GB/TB payload throughput was rerun in this change.

## Bounded catalog read-ahead comparison (2026-10-23)

Compare the preceding stat-fix binary with the final 64 KiB read-ahead binary.
Each of four hosts prepares its own private pool with 0/4,096/65,536/262,144
legal offline retain/release history frames. Each version/host/size has 25
samples per cold/warm/failed-empty-refresh retry operation, 2,400 timed calls
total. Both binaries read identical logs in alternating host/size order. Cold
means a new catalog object; OS/JuiceFS caches are not evicted. This is neither
cold-storage bandwidth nor per-event commit throughput.

First GC on a new object (ms, 100 pooled samples per cell):

| Extra history frames | Baseline p50 | Buffered p50 | Baseline p99 | Buffered p99 |
|---:|---:|---:|---:|---:|
| 0 | 17.97 | 19.05 | 68.65 | 41.10 |
| 4,096 | 23.14 | 20.19 | 36.36 | 258.36 |
| 65,536 | 118.12 | 72.31 | 163.47 | 169.50 |
| 262,144 | 417.05 | 232.32 | 485.81 | 396.09 |

At 262,144 frames, p50 falls by about 44.3%. Tail latency does not improve
universally: the buffered 4,096-frame p99 is 258.36 ms with a 278.18 ms maximum;
the 65,536-frame p99 also worsens slightly. Warm/retry paths allocate no new
buffer but still show timing variability. No general speedup or latency SLA is
established. Full replay remains linear in history; 64 KiB is a policy, not a
searched or proved optimum.

A separate syscall control performs two full parses of 4,101 total frames
(about 0.73 MiB). Read calls fall from 24,612 to 30; both versions still execute
11 seeks and four successful kernel fsyncs. Totals include common streaming
checksum checks. The injected failure returns inside the test shim without a
kernel fsync. Traced durations are excluded from the table.

For attribution, each host additionally traces 200 final-binary new-object
calls. One 252.32 ms call spends 236.76 ms inside `flock(LOCK_UN)`; across 800
calls, all syscalls taking at least 10 ms are open/flock. There are no competing
clients in these pools, so flock time must not be equated with lock contention.
This locates delay in that traced call, without attributing the earlier
untraced 258–278 ms samples or identifying filesystem/network internals.

Logs remain byte-identical, the protected 2 KiB payload remains readable, and
40 lost/surviving-tail regressions pass. GC removes 56 packs from the main
comparison, one syscall-control pack and four attribution packs; private data
and remote environments are removed. Adding regressions changed the rebuilt
native hash, so the complete comparison was rerun with the final binary. The
preliminary run is retained separately and not pooled with final samples.
[Exact samples, identities and proof scopes](../verification/measurements/20261023-catalog-buffer.json)
are retained. JSON frames, owner/closure indexes and process memory remain
unbounded; no 100 GB/TB payload bandwidth was rerun.

## GC discovery and handoff faults (2026-10-23)

Each of four hosts creates private native 2 KiB publications. The source
explicitly enables GC, seals and establishes a source owner. A default-config
destination uses automatic discovery, either fresh or after using its catalog.
One stat NotFound or EIO is armed on its `storage.log`. All 32 old-binary cases
return retain successfully without a destination owner; releasing the source
then lets GC delete the actual file. New discovery uses fresh open, so the 160
armed stat faults remain unused; real retention, post-source-release protection
and payload reads pass.

Another 40 fixed cold-discovery cases inject read-only open EIO. First retain
returns `StorageUnavailable` while source ownership and payload survive. The
caller retries successfully before releasing the source, and all 40 handoffs
remain protected. All 200 fixed handoffs pass; explicit destination release
finally permits reclamation. Old false-success calls do less work and cannot
serve as a performance baseline.

Fixed first-retain latency (ms; stat labels denote unused armed faults, while
open-error measures **failed return**):

| Scenario | Samples | p50 | p99 / maximum |
|---|---:|---:|---:|
| cold-stat-missing | 40 | 48.71 | 276.66 |
| cold-stat-error | 40 | 45.97 | 82.79 |
| warm-stat-missing | 40 | 40.02 | 82.35 |
| warm-stat-error | 40 | 33.95 | 95.54 |
| cold-open-error | 40 | 14.95 | 41.46 |

With only 40 samples per cell, nearest-rank p99 equals the maximum. These are
not discovery timeouts, retry settings or environmental delay bounds. The
276.66 ms sample was untraced and cannot be attributed using another run's
flock trace. Setup/seal/initial ownership are excluded; timing includes the
Python/native boundary, discovery, lock/fresh refresh, closure reads and owner
append/sync. No payload bandwidth or 100 GB/TB I/O was rerun.

Initial automatic discovery still trusts fresh-open NotFound, with pool mode
established before participants start. Live legacy-pool migration is not
verified. Clients requiring GC should explicitly use `online_gc=True` to avoid
selecting legacy from initial absence. Rust `Config::gc_enabled` now returns
`Result<bool>`; full shared-atomic/I/O refinement remains unproved.

All 232 private packs were reclaimed and remote data/environments removed. No
production data was faulted. [Samples, binary identities, old failing
regressions, new verification and environment counterexamples](../verification/measurements/20261023-catalog-discovery.json)
are retained. Injection is not evidence that the deployment naturally produced
stale/erroring stat responses.

## Directory durability and publication cost (2026-10-23)

Four hosts exercised 25 independent retry pools and 25 healthy pools per binary.
Two hosts ran the prior binary first and two ran the fix first. Each publication
contains a native 2 KiB payload. Test fixtures establish durable parent
directories before each pool is created; setup and GC are outside timing.

The retry probe fails one actual fsync of the private `pool/raw` directory and
requires the caller to receive StorageUnavailable. The directory already exists
after that failure. All 100 old-binary retries acknowledge publication without
syncing `pool` after creation of `raw`. All 100 new-binary retries complete the
resolved chain. Readback and subsequent ownership release/GC pass on both
binaries. This observes a missing or repaired barrier, not actual crash/data
loss; loss is an allowed consequence in the TLA+ environment.

Every measured path has 13 directories through the filesystem root. The fixed
retry issues 14 successful directory fsyncs: the complete 13-directory chain
plus the final publication barrier. The old retry issues five and omits the
required `pool` barrier. Some older ancestors are already durable fixtures;
their omission alone is not the defect. The measured depth supplies a Depth=12
TLC configuration (root is the anchor), not an algorithm limit.

Healthy timing disables fault injection and directory logging; the shim remains
loaded and forwards directly to fsync. Cold means the first publication by a new
store, including catalog initialization; hot is the next publication by that
same writer. Each cell has 100 observations, pooled over four clients:

| Publication | Old p50 ms | Fixed p50 ms | Old p99 ms | Fixed p99 ms |
|---|---:|---:|---:|---:|
| cold | 218.09 | 219.16 | 325.99 | 331.94 |
| hot | 128.08 | 128.12 | 224.05 | 245.63 |

The medians are nearly unchanged in this run. Hot p99 is higher with the fix;
this is not evidence of a universal tail-latency improvement or a causal estimate
of barrier cost. Caches and background load were not controlled. Traced retries
are separate raw evidence: their logs write to JuiceFS for every directory sync,
so their timing includes instrumentation and is not a production performance
comparison. Percentiles are observations, not timeouts or environmental bounds.

400 actual packs were reclaimed by GC, and private roots and remote environments
were removed. No 100 GB/TB bandwidth rerun or storage-service power-cut test was
performed. The fix requires sync access to all resolved ancestors, including
those outside the pool, with stable aliases/mounts/namespace. Source, exact
binaries, raw samples and formal inputs are in
[`20261023-directory-durability.json`](../verification/measurements/20261023-directory-durability.json).

## Publication retry and GC (2026-10-23)

[Summary](../verification/measurements/20261023-publication-retry.json) and
[public-API probe](../verification/storage/probes/publication_retry_probe.py) compare the
preceding directory-durability wheel with the publication-hold fix. All four
hosts use independent private pools under `/shared/straw-test`.
Two hosts run old first, two fixed first. Each source payload is 2 KiB. The
probe injects callback EIO at `after_data_sync`, after the internal dependency
hold is durable; it does not crash JuiceFS or certify power-loss behavior.

Both `publish` and `record_set` are exercised with a same-ID retry that removes
or replaces dependencies. After the original source releases, GC initially
keeps the dependency. On the old binary, a rejected retry removes protection
and subsequent GC actually deletes the original pack in **32/32 cases**. These
are dependencies needed by an uncertain destination that has not acknowledged
success. This is not an observation of spontaneous JuiceFS data loss.

On the fixed binary, **160/160 cases** retain the original dependency, finish a
correct retry, read the result/source, release extra rejected roots and reclaim
all packs after the final publication release. Each of the four API/conflict
groups has 40 samples. The table shows p50 / nearest-rank p99 in milliseconds;
with n=40, this p99 is the sample maximum, not an environmental time bound.

| API / changed roots | Rejected retry p50 / p99 (ms) | Correct retry p50 / p99 (ms) |
|---|---:|---:|
| `publish_remove` | 15.04 / 34.98 | 100.74 / 139.50 |
| `publish_replace` | 40.02 / 85.80 | 92.93 / 142.33 |
| `record_set_remove` | 18.04 / 63.27 | 143.17 / 263.68 |
| `record_set_replace` | 34.30 / 68.44 | 154.30 / 259.93 |

These timings include catalog and barrier work, with uncontrolled caches and
background load. The older binary's rejection timings are archived separately
(n=8 per group); its broken recovery outcome is not a useful successful-retry
performance baseline. No throughput, physical 100 GB/TB or universal latency
claim follows from this small-payload experiment. Model pack counts are finite
exploration configurations, not maximum supported dependency counts; the Rust
union proof has no such cardinality bound.

Across both binaries GC reclaimed **528 packs**. The old binary also left **48
private packs** (unresolved output and, for replacement retries, extra protected
roots); the supervisor removed these only after all remote commands returned,
separately from GC. Private test data and environments were confirmed absent on
all four hosts. Raw per-case events/timings, both wheels and their native hashes
are in `/shared/straw-experiments`; exact formal
inputs, traces, logs and source hashes are in the sibling
`publication-retry-verification` directory.

## Standalone finalization and GC (2026-10-23)

[Summary](../verification/measurements/20261023-publication-finalization.json),
[API probe](../verification/storage/probes/publication_finalize_probe.py) and
[native large-record probe](../verification/storage/probes/standalone_extent_probe.rs) check
`segment_target_bytes=0` across raw writes, record sets and publication.
The preceding wheel succeeds but never marks raw-write/record-set files sealed
in the catalog. Of 48 old-binary healthy cases, 32 remain unreclaimable after
explicit seal, release, close and fresh reopen; all 16 publish controls reclaim.
On the fixed wheel, all 240 healthy cases and 48 uncertain-seal cases reclaim.
Four hosts use independent private pools, two running old first and two fixed
first. Each API/size group has n=8 old and n=40 fixed samples.

Fault cases raise callback EIO before/after publication seal or catalog sync,
then run GC, retry the same identity, authenticate the returned small record
and release it. GC never deletes a protected file; retry creates no duplicate
file. An error before catalog sync can leave a complete frame which the next
refresh sync promotes. These injections do not demonstrate natural JuiceFS
failures or service/power-loss behavior.

Fixed-wheel timings are milliseconds, p50 / nearest-rank p99; with n=40,
p99 equals the sample maximum. For record_set, the source record is prepared
before timing and the timed call writes its manifest. For the other APIs the
call writes the indicated payload. GC timing excludes explicit release and
covers the complete collect call, including namespace deletion and receipts.

| API | Source payload | Publication p50 / p99 (ms) | GC p99 (ms) |
|---|---:|---:|---:|
| `write_records` | 2 KiB | 162.01 / 269.29 | 98.53 |
| `write_records` | 8 MiB | 225.28 / 379.13 | 93.86 |
| `record_set` | 2 KiB | 251.66 / 359.71 | 119.31 |
| `record_set` | 8 MiB | 251.65 / 401.25 | 98.56 |
| `publish` | 2 KiB | 185.87 / 285.71 | 83.18 |
| `publish` | 8 MiB | 260.33 / 352.80 | 105.91 |

Old timings are archived, but missing sealing and no-op GC do not perform the
same completed work. Caches/background load were uncontrolled, and the run
overlapped the large probe below. Repeated-byte small payloads and these short
series provide no cold-cache, object-store bandwidth or latency bound.

The separate native probe streamed **100,000,000,000 payload bytes** into one
standalone record. File length was **100,000,000,585 bytes**. It uses a repeating
8 MiB random pattern and sequential writes, with no sparse-file construction.
Publication including hashing, data sync, staging and sealing took
**283.616 s (336.26 MiB/s logical payload)**.
A fresh full-extent checksum scan on the same host took
**111.828 s (852.80 MiB/s)**;
no cache eviction was performed. Peak process RSS was **10,848 KiB**.
This is one observation, not p99, a cold-read result or a physical-device SLA.
Whole-record APIs that return an allocated payload still have separate memory
requirements; this probe uses streaming write and checksum-only inspection.

The file was catalog-sealed while staging protected it. After explicit release
(53.67 ms), GC reclaimed exactly one file and its reported
100,000,000,585 bytes in **5437.42 ms**. This measures unlink,
parent barriers and a durable catalog receipt, not completion of asynchronous
backend object reclamation. A 2 KiB native smoke case preceded the large run.
No physical TB record was written; the single-publication workload cap remains
100 decimal GB. The finalization controller is independent of file size.

Four-host GC removed 400 files; 48 old-binary retained fixture files were
manually removed after remote work returned, separately counted. The large
file was removed by GC. All shared private pools and remote environments were
cleaned. Artifacts are under
`/shared/straw-experiments`,
`publication-finalization-large` and `publication-finalization-verification`.
Historical open entries from earlier application runs are not automatically
sealed; they may belong to uncertain or live writers.

## Pending files and sync errors (2026-10-23)

[Summary](../verification/measurements/20261023-pending-publication.json),
[API probe](../verification/storage/probes/pending_publication_probe.py) and
[scoped fsync shim](../verification/storage/probes/publication_sync_fault.c) compare the
standalone-finalization wheel with the pending-description fix. Four hosts use
independent private JuiceFS pools; two run old first, two fixed first. Each of
write_records, record_set and publish is tested with 2 KiB / 8 MiB source records
and three failure modes:

- `before`: consume an arm file, skip one real payload fsync and return EIO.
- `after`: real payload fsync returns success, but the shim returns EIO to straw.
- `callback`: fsync returns success to straw, then after_data_sync raises EIO.

Only selected private payload paths are intercepted. Fsync traces/arm files live
under local /tmp, outside the measured pool. Trace entries distinguish calls
actually performed and their outcomes. No JuiceFS service crash, actual power
loss or natural fault frequency is inferred from these injections.

The old binary rewrites a complete extent in **72/72 cases**, then leaves its
original partial/catalog-open entry. The fixed binary reuses the original
UUID/file in **360/360 cases**, authenticates its bytes, returns readable records
and reclaims all released files. No retry payload-chunk callback occurs. Unknown
sync (`before` / `after`) is retried once on the same file. Confirmed sync followed
by callback failure needs no additional file barrier; checksum validation and
directory completion still run. Packed controls and corruption rejection are
covered by separate local regressions, not this standalone-only four-host run.

Fixed write_records retry timing (ms; n=20 per row, p99 equals sample maximum):

| Payload | Failure | p50 | p99 / max | Retry file-sync calls |
|---|---|---:|---:|---:|
| 2 KiB | `before` | 71.16 | 148.88 | 1 |
| 2 KiB | `after` | 70.40 | 147.36 | 1 |
| 2 KiB | `callback` | 76.96 | 134.48 | 0 |
| 8 MiB | `before` | 94.45 | 147.60 | 1 |
| 8 MiB | `after` | 99.30 | 171.80 | 1 |
| 8 MiB | `callback` | 99.50 | 142.67 | 0 |

The complete report retains all API/size/failure groups (old n=4, fixed n=20),
including publish and record_set. For record_set the source is prepared before
timing; its retried extent is a small manifest, not another source payload.
Timing includes input hashing, full retry inspection, ownership/catalog work,
barriers and tracing overhead. Caches and background load are uncontrolled.
It is not a filesystem-only latency measurement or a delay bound.

Old cases collectively rewrite **201,445,104 logical extent bytes** and leave
that same number of bytes in 72 original partial files. The count uses complete
file lengths and observed retry payload writes; it is not block-device or
backend physical traffic. Fixed cases write zero retry payload chunks; this
avoids file rewrite, not the input-hash pass or complete authentication scan.
No 100 GB/TB workload was rerun; the preceding 100 GB evidence has a different
binary and scope. No payload-size or time bound was added to the environment.

GC reclaims **576 completed files** across both binaries. The supervisor removes
72 old orphan partials only after all remote commands return, separate from GC.
All shared private pools and remote environments are confirmed removed. Exact
wheels, native/shim hashes, traces and per-case outcomes are under
`/shared/straw-experiments`; formal inputs,
logs, source snapshots and package checks are in `pending-publication-verification`.
Historical application orphan partials/open entries are not automatically repaired.

## Namespace identity and GC (2026-10-23)

The final [measurement record](../verification/measurements/20261023-path-identity.json)
identifies both exact wheels/native extensions. On four hosts, 48 of 64 old cases
lost data while an alias owner remained; the 16 canonical controls stayed readable.
All 320 final-wheel cases stayed readable through protected GC and reopened-catalog
checks, then reclaimed after owner release. File, directory and writer-directory
aliases were exercised with 2 KiB/8 MiB payloads and standalone/packed files.
Another 32 old-writer/new-GC cases preserved staging during migration and tested
legacy alias replay. Four-client handoffs assigned publication, retain, GC and read
to different hosts: all four old alias cases lost data, all eight repaired cases
kept it until release. All 440 data files were accounted for by GC; private pools
and remote environments were removed and absence checked on all four hosts.

For replay cost, both binaries read the same catalog bytes containing one real
owned 2 KiB file plus 0/128/4,096 offline completed-receipt histories for absent
paths. These histories are not measurements of payload write/delete throughput.
Each row pools 20 observations per binary. Cold means a fresh catalog object,
not evicted filesystem caches. Values below are p50 / p99 milliseconds:

| Historical retired paths | Old cold | Final cold | Old warm | Final warm |
|---|---:|---:|---:|---:|
| 0 | 30.83 / 97.72 | 19.13 / 39.16 | 12.85 / 29.95 | 17.21 / 47.12 |
| 128 | 22.32 / 48.61 | 25.31 / 114.01 | 14.73 / 23.79 | 15.42 / 32.83 |
| 4,096 | 26.23 / 75.41 | 106.15 / 137.29 | 13.08 / 26.79 | 15.05 / 58.88 |

Prefix caching avoids repeated resolution of shared ancestors, but distinct-name
lookup still adds cold replay cost. No controlled background load or cache eviction
was used. Nearest-rank p99 is the maximum at this sample count. Per-workload samples
number four for old and twenty for final; the record includes canonical-control
publication/retain/read/GC timings. Unsafe old alias deletion/read paths do different
work and are not a performance baseline. None of these values supplies a hard
latency bound. This run does not replace prior 100 GB bandwidth measurements, which
used different binaries; sparse 2 TiB regression offsets are not physical TB I/O.

The deployment must provide stable aliases/prefix bindings and truthful resolution
(`ENV-NAMESPACE`); cached identities do not detect alias retargeting. Bind-mount
aliases/inode equivalence and arbitrary external namespace edits remain outside
this policy. Old collectors must be upgraded before relying on alias ownership.

## Catalog path encoding and replay lookups (2026-10-23)

The [measurement record](../verification/measurements/20261023-catalog-encoding.json)
compares the preceding alias-safe binary with the new `path_encoding=1` version.
Both preserve alias ownership. Four hosts passed 64 normal/alias cases per binary,
64 bidirectional old-writer/new-reader and new-writer/old-reader cases, and eight
mixed-version four-client handoffs. GC reclaimed all 224 data files; private pools
and remote environments were removed and absence checked on each host.

Each history fixture contains one real owned 2 KiB file and 0/128/4,096 offline
completed-receipt entries. Setup is not a payload write/delete throughput measurement.
Both binaries read identical bytes per fixture: legacy entries lack markers,
resolved entries carry truthful version-1 markers. Binary order is reversed on
half the hosts. Each row pools 20 uninstrumented observations; p50 / p99 in ms:

| Encoding and historical entries | Old cold | New cold | Old warm | New warm |
|---|---:|---:|---:|---:|
| legacy-0 | 28.19 / 91.57 | 35.39 / 67.09 | 22.43 / 62.86 | 15.04 / 43.75 |
| legacy-128 | 22.76 / 45.89 | 21.17 / 53.40 | 11.44 / 38.45 | 12.22 / 61.21 |
| legacy-4096 | 92.26 / 184.26 | 83.62 / 145.65 | 21.68 / 79.10 | 18.44 / 47.29 |
| resolved-0 | 25.12 / 59.40 | 23.00 / 103.49 | 15.66 / 61.49 | 15.58 / 63.76 |
| resolved-128 | 25.33 / 141.54 | 24.49 / 71.64 | 14.44 / 37.74 | 16.63 / 66.44 |
| resolved-4096 | 98.93 / 217.10 | 30.13 / 54.11 | 18.61 / 79.49 | 13.54 / 116.82 |

A separate instrumented cold sample on each host counts 4,101 readlink API calls
for 4,096 resolved entries on the old binary and zero on the new; legacy entries
still count 4,101 on both. Counts for 0/128 entries are 4/133, becoming zero only
for new resolved replay. All warm counts are zero. These intercepted readlink/
readlinkat calls are not all filesystem operations, kernel lookups or metadata
RPCs. Instrumented timings are excluded from the table.

Cold means a fresh catalog object, with no filesystem cache eviction or controlled
background load. At n=20 nearest-rank p99 equals max; warm tails still vary, so this
is not an improvement claim for every latency. Old logs are not rewritten and
retain their resolution cost. The marker, version and normalized keys are protocol
choices; measurements supply no new hard time/retry bound. This cycle uses 2 KiB/
8 MiB data and existing sparse large-offset regressions, not a new 100 GB run or
physical TB transfer.

The compact JSON marker adds 18 bytes per event. The measured 4,096-entry legacy/resolved fixtures occupy 910,802 / 984,584 bytes respectively (4,099 events including the real publication history). Both compared binaries read the same resolved fixture; the timing gain is not from fewer catalog bytes.

## Incomplete writes and cleanup retries (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-incomplete-publication.json)
compares the preceding wheel with incomplete-write cleanup. Each binary runs 64
four-host cases: 2 KiB/8 MiB, standalone/packed, interrupted after header, first
payload chunk, record or before footer, twice per case. Normal GC leaves 64 old
partials totaling 167,835,504 bytes; the fixed writer leaves no unpublished garbage.
Old packed dead tails disappear only when the whole pack is finally reclaimed;
retry offsets already skip those bytes. The fixed writer restores the saved prefix
before retry and keeps old records readable.

For 8 MiB payloads stopped after the first 4 MiB chunk, two attempts leave
8,389,000 bytes including framing in the old binary, zero in the fixed one. The
table selects that failure phase: pooled n=8 failures and n=4 retries, milliseconds,
p50 / p99. Failure time includes preparation and cleanup, not isolated truncate/fsync.

| Payload / mode | Old failure return | Fixed failure + cleanup | Old successful retry | Fixed successful retry |
|---|---:|---:|---:|---:|
| 2 KiB / standalone | 69.00 / 103.46 | 117.51 / 173.32 | 186.65 / 218.31 | 159.11 / 176.46 |
| 2 KiB / packed | 34.53 / 80.52 | 63.72 / 73.44 | 94.51 / 196.89 | 61.00 / 186.76 |
| 8 MiB / standalone | 110.14 / 198.95 | 156.84 / 279.83 | 208.69 / 295.06 | 158.21 / 300.02 |
| 8 MiB / packed | 109.85 / 117.42 | 122.14 / 272.34 | 162.61 / 231.02 | 133.93 / 220.80 |

Cleanup adds required barriers and can increase failure-return latency. The old
binary performs less work, so this is not an equivalent-semantics speed comparison.
Nearest-rank p99 is the maximum at these small sample sizes; caches and background
load are uncontrolled. These are not bandwidth measurements. Each host independently
retries its own live writer on the shared filesystem; no cross-host writer recovery
is claimed.

Another 32 cases inject real syscall EIO before/after successful truncate, unlink,
file fsync and parent-directory fsync. Every case retains retryable cleanup state,
preserves the older record and completes publication/GC after recovery. Separate
traces are excluded from the table. These are deliberate faults, not naturally
observed errors or power-loss tests. Normal GC reclaims 240 files; the driver manually
removes the 64 recorded old private partials, without counting them as GC success.
All private pools and four remote environments are confirmed removed.

Real payloads are 2 KiB/8 MiB; local sparse-offset regressions approach 2 TiB. This
cycle performs no physical TB transfer or new 100 GB bandwidth run on the final
binary. Exact offsets and barrier ordering are protocol/implementation obligations;
measured durations never become safety premises.

## GC missing parents and 100 GB recheck (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-gc-parent-repair.json)
compares the preceding and repaired wheels. Each runs 120 four-host cases with real
2 KiB/8 MiB garbage plus a separately protected owner. Existing-parent controls pass.
All 80 old missing-parent cases fail three attempts and require explicit manual
directory creation to finish GC; fixed cases need no manual repair. Failed GC calls
may already unlink files, so observed removal and success-only return counters are separate.

Each size/parent group pools n=20, milliseconds, p50 / p99. Missing levels are observed
at collection time. **Old missing-parent times are errors; fixed times are successful
completion, not an equal-work speed comparison.**

| Real garbage payload | Missing levels | Old first attempt | Fixed first attempt |
|---|---:|---:|---:|
| 2 KiB | 0 | 60.58 / 125.72 | 63.38 / 101.73 |
| 2 KiB | 1 | 48.72 / 83.65 | 73.20 / 117.09 |
| 2 KiB | 2 | 44.87 / 124.51 | 74.37 / 123.88 |
| 8 MiB | 0 | 56.90 / 117.95 | 69.20 / 197.12 |
| 8 MiB | 1 | 44.25 / 72.18 | 65.36 / 103.55 |
| 8 MiB | 2 | 42.89 / 71.28 | 72.96 / 99.14 |

Repair executes 12 directory barriers including root, motivating a Depth=11 TLC
fixture. Existing-parent paths add no creation/ancestor barriers. Nearest-rank p99
is the maximum at n=20. Caches/background load are uncontrolled and the 100 GB run
overlaps the experiment's tail. Existing-path tail latency varies; no universal
improvement or SLA is established.

Another 24 syscall cases inject false directory-open ENOENT/EIO, fsync EIO before/after
real success, and mkdir EIO before/after real success. Post-success mkdir EIO is
reconciled by create_dir_all's exists check; required barriers still follow. Other
errors retain retry obligations. Protected records remain readable and fresh-client
replay skips completed payload GC I/O. Across successful and failed-call side effects,
GC calls remove 528 actual data files. Old fixtures need manual directory repair,
not manual data deletion. Private pools and remote environments are removed.

A separately compiled native probe from the same Rust core sources publishes one
**100,000,000,000-byte** record on `node-1`; the file is **100,000,000,585
bytes**. Publication including preparation/checksums/durability/staging takes
281.197 s (**339.15 MiB/s**). Full-extent
authentication takes 110.960 s (**859.47
MiB/s**). Peak RSS is 10.69 MiB. After release, GC takes
5.301 s, retires one file, leaves zero owners and confirms the path
absent; the private pool is removed.

This is one native-core run with a repeated random 8 MiB pattern, same-host read and
no cache eviction. It is not Python FFI throughput, a p99, physical-device bandwidth
or backend-object/trash completion. Probe and wheel binary hashes are separate;
core source correspondence is checked. No actual TB file is written; roughly-2-TiB
coverage remains sparse-offset tests and integer proofs.

## Publication preparation failures (2026-10-23)

[Exact binaries, raw samples and proof scope](../verification/measurements/20261023-publication-preparation.json)
cover 320 cases per wheel on four hosts, alternating binary order, plus 24 current
open/seek EIO injections before/after actual effects. Each failure case makes two
uncertain registrations then retries a real 2 KiB/8 MiB record. The old wheel leaves
480 open reservations after normal close/GC; the new wheel leaves zero. Normal
GC removes all 664 successful data files. Old catalog-only leftovers are counted
as manual private-fixture cleanup; no payload file was manually removed.

Healthy publication latency, milliseconds p50 / nearest-rank p99, pooled n=20
per row/binary (p99 is the sample maximum):

| Mode | Payload | Old | New |
|---|---:|---:|---:|
| standalone | 2 KiB | 138.19 / 878.15 | 147.14 / 211.54 |
| standalone | 8 MiB | 269.55 / 1004.50 | 282.05 / 1041.42 |
| packed | 2 KiB | 116.23 / 181.70 | 104.33 / 166.96 |
| packed | 8 MiB | 188.37 / 254.76 | 163.03 / 413.57 |

Failure timing includes different work: the repaired writer removes/syncs/seals
before reporting the original error, while the old one leaks a reservation.
Retry publication also starts from different directory/catalog states. Therefore
these are not equal-work failure speed comparisons. Healthy tails vary; no
uniform performance improvement is established. Caches/background traffic were
uncontrolled and the 100 GB run overlapped. No percentile is a hard bound.

Resolved parent depth is 11 edges (12 directory levels including root), matching
the existing `directory_gc_repair_measured_depth` TLC case. This is an observed
parameter, not a maximum supported depth or a composition proof. Hook counts in
the raw report include attempts before a missing parent is opened and are not
physical fsync counts.

The current Rust core published **100,000,000,000 payload bytes** in 284.076 s (**335.71 MiB/s**) and authenticated the full extent in 105.693 s (**902.30 MiB/s**). Peak RSS was 10.68 MiB; release followed by GC took 5.575 s for namespace deletion/receipt. The file held 100,000,000,585 bytes and the private pool was removed. This is one native-core sample with a repeated random 8 MiB pattern, same-host read and no eviction, not Python/FFI or cold physical bandwidth, a p99, TB I/O, or backend object-reclamation completion.

## Publication handoff failures and a 100 GB retry (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-publication-handoff.json) contains 400 cases per old/new binary across four hosts: write_records, publish and record_set; standalone/packed; 2 KiB/8 MiB inputs; response loss after staging and, for the latter two APIs, final hold release. Callbacks raise after real successful catalog sync: these are neither syscall EIO nor power loss. record_set writes a small manifest referencing the sized payload.

The old writer admits another request, overwrites the cache and rewrites the original on retry. The fix rejects it and retries without payload-write callbacks. After release/GC, old cases retain 400 owners, 120 open reservations and 640 files / 2,350,605,360 bytes including dependencies; fixed cases retain none. Normal GC deletes 960 old and 1,400 fixed files. Old leftovers are separately removed as fixtures, not counted as GC. All private pools and remote environments are cleaned. An initial aggregate assertion overlooked dependency files after all 800 cases passed; corrected per-case accounting reuses every sample and preserves the failed report.

write_records original-request retry latency, milliseconds p50/p99, n=20 per group (five per host); nearest-rank p99 equals the maximum. **The old retry rewrites; the fixed retry reuses the file, so work differs.**

| Mode | Payload | Old | Fixed |
|---|---:|---:|---:|
| standalone | 2 KiB | 109.25 / 118.34 | 38.04 / 47.15 |
| standalone | 8 MiB | 166.77 / 239.81 | 44.57 / 67.25 |
| packed | 2 KiB | 63.33 / 80.44 | 11.09 / 20.73 |
| packed | 8 MiB | 117.46 / 142.06 | 18.65 / 26.73 |

Caches/background traffic are uncontrolled and the 100 GB experiment overlaps. Other API/stage samples remain in JSON. These observations supply neither latency bounds nor modeled retry deadlines.

A separate native executable from the same Rust core writes **100,000,000,000 payload bytes** on node-1 (file 100,000,000,582 bytes), then loses the staging response. The first attempt takes 281.320 s (339.00 MiB/s by payload), **ending in error rather than successful-publication throughput**. Another ID is rejected in 0.001 ms without traversing payload. Original-request retry takes 69.253 s, hashing input once with zero payload-write callbacks and unchanged file identity/length/mtime. Total workflow, including diagnostic GC and rejected request, is 350.585 s (272.02 MiB/s), not one API's latency.

Full extent authentication takes 105.522 s (903.77 MiB/s); peak RSS is 18.71 MiB. Post-release GC takes 5.552 s, removes one file and leaves no owner/open reservation; the private pool is cleaned. This single native sample uses a repeated random 8 MiB pattern and same-host reads without cache eviction. It establishes no Python FFI/cold-cache physical bandwidth, p99, actual TB I/O or backend object-reclamation completion.

## Staging consumption, GC and retry (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-staging-consumption.json) contains 336 cases per old/new binary across four hosts: 320 publish/record_set scenarios and 16 write_records controls. Each loses a release response after real successful catalog sync, allows peer GC, then retries with fresh Config/catalog replay. An independent reader protects an unrelated publication throughout. All 320 old manifest retries fail on deleted files, while raw controls pass; all 336 fixed cases succeed without retry manifest-read/catalog-append hooks.

Real payloads are 2 KiB/8 MiB at depths 1/12. Reachable publication graphs measure 2/24 nodes and 1/23 direct edges, matching TLC Depth=1/12 (98,296 states at depth 12). Direct raw roots, including record_set internal raw-manifest consumption, add no graph cache entries. The independent reader control graph is counted separately. Search depth is not an API limit and quantiles are not hard environment bounds.

For **12 levels, 8 MiB per level**, retry milliseconds p50/p99 and median first-release catalog byte growth follow. Each group has n=20 (five per host); nearest-rank p99 is the maximum. **Old retries fail while fixed retries succeed, so this is not equivalent-work acceleration.**

| API | Mode | Old retry | Fixed retry | Catalog bytes: old/fixed |
|---|---|---:|---:|---:|
| publish | standalone | 186.22 / 238.73 | 20.45 / 57.59 | 1103 / 4659 |
| publish | packed | 192.08 / 265.08 | 22.45 / 56.38 | 1103 / 4659 |
| record_set | standalone | 177.94 / 216.67 | 23.88 / 61.20 | 2051 / 5607 |
| record_set | packed | 181.80 / 237.62 | 25.15 / 77.68 | 2051 / 5607 |

Direct edges are stored linearly rather than as every node's transitive closure; overall catalog memory/history is still unbounded. Byte deltas measure encoded log growth, not heap allocation. JSON retains all setup/first-failure/retry/metadata groups. Caches/background traffic are uncontrolled; the final run overlaps the separately archived intermediate experiment. Read hooks are new, so baseline zero hooks do not establish zero I/O.

Normal GC removes 2112 old and 2112 fixed data files; no payload leftovers require manual removal. Private pools and remote environments are cleaned. Maximum real payload per graph is 96 MiB. This run adds no 100 GB measurement for the current binary or physical TB transfer. Full regressions still include sparse TB-offset checks; no bandwidth SLA, cold-cache or physical backend-reclamation guarantee follows.

A separate final-wheel run consumes a 2,048-level inline-manifest chain with a 512 KiB main-thread stack. It reads 2049 manifests and adds 469,470 catalog bytes; the first lost-response call takes 0.672 s. After peer GC removes two files, fresh replay/retry completes in 21.11 ms without manifest reads or catalog appends. Peak RSS is 47.64 MiB for one Python-inclusive process, not a memory bound. Leaf payload is only 1 KiB; remaining bytes are real metadata. The private pool is cleaned.

## Preparation dependency-owner cleanup (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-provisional-holds.json) covers 480 cases per baseline/fixed binary on four hosts: publish/record_set, standalone/packed, 2 KiB/8 MiB records, and validation, lost post-sync hold response or payload-callback failure. Five samples per host/group yield pooled n=20, with nearest-rank p99 equal to the maximum. Binary order alternates by host; local regression/verifier work runs concurrently and caches are not evicted.

Both versions first release source staging, run GC and fully read the failed preparation dependency before close. After close and GC, the baseline retains 480 internal owners/480 files; the fix retains none. An independent reader remains readable and is released only before final GC. Normal GC removes 800/1280 baseline/fixed data files. Baseline leftovers totaling 3,021,279,300 bytes are then explicitly removed only inside newly allocated private fixtures, not counted as successful GC. GC file counts exclude unpublished output removed by live-writer rollback. The fix needs no manual payload cleanup. All private pools and remote environments are cleaned.

The table gives close latency for 8 MiB records at the writing cutpoint, in milliseconds. Fixed close calls include the additional durable owner release; old close calls leave an owner behind, so this is not an equal-work throughput comparison.

| API | Mode | Baseline p50 / p99 | Fixed p50 / p99 |
|---|---|---|---|
| publish | standalone | 0.00 / 0.00 | 33.34 / 80.24 |
| publish | packed | 34.63 / 77.85 | 72.03 / 135.47 |
| record_set | standalone | 0.00 / 0.01 | 44.89 / 96.58 |
| record_set | packed | 31.76 / 72.23 | 69.84 / 159.79 |

These observations check error/reclamation behavior, not hard deadlines or fairness premises. Full records include validation/hold cutpoints and close/GC timings. No current-binary 100 GB run, physical TB transfer, cold-cache bandwidth or power-loss durability claim is added; earlier large-file evidence remains bound to its exact binaries.

## Current Rust core: 100 GB four-host workflow (2026-10-23)

The [exact-binary record](../verification/measurements/20261023-shared-record-workflow.json) uses unchanged production Rust from the preceding 693-test Python run in a separately compiled native probe, not Python/FFI throughput. A 33,556,480-byte preflight precedes one successful **100,000,000,000-byte** tensor publication on node-1; its source file is 100,000,050,713 bytes. Publication takes **281.063 s, 339.31 MiB/s**, including input hashing, framing, writes and sync. Random repeated 8 MiB pattern/checksum-metadata preparation is excluded. Writer peak RSS is 11.04 MiB, one native-process observation.

A destination record_set fails before its footer while its established preparation hold protects the source. Source staging is released and GC runs before four hosts read disjoint slices. All 373 chunks together authenticate and return exactly 100 GB without overlap or gaps. Aggregate time is **41.071 s, 2322.01 MiB/s**, including supervisor SSH startup; host rates are not simply summed.

| Host | Authenticated/returned bytes | Native read seconds | MiB/s | Peak RSS MiB |
|---|---:|---:|---:|---:|
| node-1 | 25,106,507,776 | 40.567 | 590.22 | 260.86 |
| node-2 | 24,964,497,408 | 39.982 | 595.46 | 260.93 |
| node-3 | 24,964,497,408 | 40.145 | 593.05 | 260.87 |
| node-4 | 24,964,497,408 | 40.091 | 593.85 | 260.89 |

Each call returns at most 256 MiB plus bounded authentication scratch. RSS includes the returned buffer and is distinct from the 8 MiB scratch setting. For 372 full 256 MiB chunks, sample p50/p95/p99 are **417.86/457.87/525.69 ms**, excluding the final short chunk. Samples are correlated within one file, concurrent hosts and one run; they are not independent whole-file quantiles or visibility deadlines. Caches are not evicted, and one publication supplies no publication p99 or bandwidth SLA.

After every reader returns, original handoff retry takes 339.73 ms; explicit close cleans another failed preparation in 61.48 ms. Consumption loses its response after catalog sync. Peer GC normally removes the source and a 1,152-byte manifest, 100,000,051,865 bytes total, in 1.678 s. Fresh-client release retry after manifest removal takes 20.00 ms with zero manifest-read/catalog-append hooks. Final owner/open-pack counts are zero. Private data and all remote program directories are cleaned; this does not establish immediate physical backend reclamation.

At the first observed writer wait for readers, cumulative Linux user/system CPU are 207.70/38.87 s. These counters also include pattern preparation/control work and monitoring starts late in publication; they are not exact per-API CPU timings or sufficient CPU/filesystem bottleneck attribution. Raw samples are archived. No actual TB transfer, injected 100 GB reader failure or physical power loss is tested, and general recovery/memory/bandwidth bounds remain unproved.

## Writer callback access on four hosts (2026-10-24)

[Exact binaries and complete evidence](../verification/measurements/20261024-writer-access.json) compare the preceding wheel with nonblocking writer access. Each host exercises metrics/seal/close, callback reentry or a dependent caller on another thread, 2 KiB/8 MiB records and standalone/packed output. Baseline runs once per combination (96 total); the final fix runs five times (480). The outer write is paused before its footer, after writing real payload and index bytes. Already committed prefix data must remain intact.

Every baseline child reaches the accessor and remains live without a response for the two-second observation window; its supervisor explicitly kills and reaps it. That finite observation is not a proof of infinite deadlock; the separate TLA+ waiting-cycle trace supplies the model-level liveness counterexample. Baseline cleanup manually removes 144 private data files / 402,853,536 bytes, not counted as successful GC.

All 480 fixed cases report ResourceLimitExceeded, then finish and authenticate the original publication, write another record, release ownership and close. Normal GC removes 960 files; final owner/open/retained counts are zero and no fixed data file requires manual cleanup. All shared private roots and remote test environments are removed.

| Accessor | Caller mode | Samples | p50 ms | p99 ms |
|---|---|---:|---:|---:|
| metrics | callback | 80 | 0.011 | 0.027 |
| metrics | thread | 80 | 0.014 | 0.027 |
| seal | callback | 80 | 0.012 | 0.030 |
| seal | thread | 80 | 0.014 | 0.031 |
| close | callback | 80 | 0.011 | 0.104 |
| close | thread | 80 | 0.013 | 0.052 |

These timings cover Python-visible busy exception responses, not file reads/writes. Each group mixes two sizes and two layouts across four hosts and five repetitions; n=80 nearest-rank p99 equals its maximum. Repetitions, scheduling and background load are not independent/controlled. Neither the two-second watchdog nor the measured microsecond-scale response enters the environment as a numeric deadline. No new 100 GB/TB, payload-bandwidth or power-loss claim follows from this binding change; prior native 100 GB measurements retain their own binary identities.

## Queue post-commit cleanup on four hosts (2026-10-24)

[Exact binaries and complete evidence](../verification/measurements/20261024-commit-cleanup.json) cover 2 KiB/8 MiB, standalone/64 MiB pack targets, errors before release/after a partial catalog frame/after a durable receipt, and live retry/fresh recovery. Each of four hosts runs one baseline and three fixed repetitions per combination: 96 baseline and 288 fixed cases.

The 64 baseline pre-release/partial-frame cases leak owners and files; the other 32 durable-receipt cases reclaim normally. Manual baseline fixture cleanup removes 64 files / 268,579,360 bytes, separately from normal GC's 32 files / 134,289,680 bytes. All 288 fixed cases finish with zero owners/open packs; normal GC removes 288 files / 1,208,607,120 bytes without manual data deletion. Every case first checks an independent reader after queue pruning, then releases that owner. All shared fixture roots and remote environments are removed.

| Failure cut | Retry mode | Samples | p50 ms | p99 ms |
|---|---|---:|---:|---:|
| after_storage_catalog_part | live | 48 | 44.055 | 106.006 |
| after_storage_catalog_part | reopen | 48 | 92.596 | 142.292 |
| after_storage_catalog_sync | live | 48 | 15.459 | 64.744 |
| after_storage_catalog_sync | reopen | 48 | 62.907 | 120.300 |
| before_staging_consume | live | 48 | 36.381 | 119.436 |
| before_staging_consume | reopen | 48 | 85.228 | 344.026 |

Each n=48 group mixes four hosts, two sizes, two layouts and three repetitions; nearest-rank p99 equals the maximum. Reopen timing also includes close/open/epoch work, not just release. Cache/background state is uncontrolled and samples are not independent. These observations describe this fault-recovery workflow, not visibility bounds or new 100 GB/TB bandwidth evidence. Only Black formatting followed the remote probe copy; the archive preserves exact executed source, final-source hashes and an AST-equivalence check.

## A 100 GB queue record and historical recovery (2026-10-24)

[Exact program, source and complete evidence](../verification/measurements/20261024-queue-record-workflow.json) exercise the current native core: a durable record/manifest, four tasks sharing its input, an error after accepting WAL but before staging release, fresh cleanup recovery, real accepted/cancelled history, four input leases, cancellation and another recovery. GC retains the readers. Four hosts authenticate disjoint shards; the supervisor acknowledges only after successful remote exit, matching shard identity and verified byte coverage. The first three acknowledgement/GC steps reclaim nothing; the fourth reclaims data and manifest. Later recovery reads no manifests and appends no catalog frames.

Payload is **100,000,000,000 bytes**, in a 100,000,050,717-byte data file. Publication takes 271.268 s (351.561 MiB/s); manifest creation takes 0.247 s. Pending-cleanup recovery takes 0.091 s; constructing 128 historical handoffs takes 14.341 s and post-cancellation recovery 0.095 s. Writer peak RSS before readers is 11,444 KiB.

All 100 GB are authenticated/returned across 373 chunks in 40.182 s (2373.393 MiB/s aggregate), including SSH startup/supervision. Final GC removes two files / 100,000,051,877 bytes, with zero owners/open packs. First subsequent recovery takes 0.082 s. No manual data deletion is needed; every shared fixture root and remote program is removed.

Separate 2 KiB workloads construct real accepted/cancelled history. WAL/catalog bytes below are measured before reader acknowledgements; recovery samples follow GC. Each group runs twenty fresh coordinators on each of four hosts sequentially, always after the preceding process exits. Every recovery appends an epoch, so the log is not static across samples.

| Historical handoffs | Setup s | WAL bytes | Catalog bytes | Recovery p50 ms | Recovery p99 ms | Peak RSS KiB |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 0.000 | 7,166 | 3,850 | 54.261 | 99.166 | 3,484 |
| 128 | 15.499 | 184,920 | 35,338 | 63.585 | 114.786 | 6,380 |
| 1024 | 104.910 | 1,433,168 | 255,754 | 95.187 | 158.703 | 26,420 |

For n=80, nearest-rank p99 is the maximum. Fresh instances do not evict storage caches; hosts/repetitions/background load are uncontrolled and correlated. Setup timings describe individual submit/cancel calls, not batch-admission capacity. 0/128/1024 are experimental/model configurations, not supported-scale limits. Metadata still grows without compaction or a proved memory bound.

This is one native 100 GB record and queue-input reading workflow. An 8 MiB random pattern repeats; publication timing excludes pattern/chunk-checksum metadata preparation and includes actual data hashing/framing/sync. Returned ranges are at most 256 MiB and every requested byte is authenticated. Formal checkers also ran on the writer host without CPU isolation; process/cgroup observations are archived without bottleneck attribution. No new Python/FFI, physical TB transfer or hard bandwidth SLA claim follows; prior sparse-TB-offset evidence remains separate.

## Position-distinguishable 100 GB range reads (2026-10-24)

The [complete record and exact binary](../verification/measurements/20261024-positioned-range.json) uses global-word-position data instead of a repeated 8 MiB pattern. Every checksum chunk has a different digest; four native readers compare every returned byte after production authentication. An independent Python implementation checks every chunk digest of the 32 MiB workload and each remote request's prefix/suffix. One actual write remains **100,000,000,000 bytes**; earlier sparse TB-offset evidence is separate.

Data generation and chunk-checksum metadata preparation take 113.308 s, reported outside publication. Publication and close take 363.514 s (262.349 MiB/s), including stream regeneration, two content-processing passes, framing and durability. The producer supplies callback pieces of at most 1 MiB, an experiment choice. Writer peak RSS is 5,044 KiB; the data file contains 100,000,050,725 bytes.

Four hosts read 373 complete chunks covering 100 GB, plus 96 boundary intervals, of which 25 are empty. Samples cover word, 1 MiB buffer and 256 MiB chunk boundaries, 2 GiB/4 GiB, the midpoint and tail. Nonempty small slices contain 1–4099 bytes. Empty slices authenticate zero payload bytes but may still read/authenticate metadata.

| Metric | This run |
|---|---:|
| Total returned bytes including slices | 100,000,094,695 |
| Total authenticated payload bytes | 120,953,550,848 |
| Four-host elapsed time including SSH, slices and byte oracle | 62.131 s |
| 100 GB divided by that complete workflow time | 1534.941 MiB/s |
| Full 256 MiB native-call p99 (n=372) | 575.895 ms |
| Nonempty small-slice native-call p99 (n=71) | 552.673 ms |
| Four reader peak RSS values, KiB | 268376 / 268348 / 268204 / 268268 |

A one-byte request can authenticate 256 MiB; a 17-byte request across a chunk boundary can authenticate 512 MiB. Checked bytes count logical payload authentication, not physical backend I/O. Bounded scratch does not imply small read amplification; optimization must preserve authentication of the touched chunks.

After successful remote exits and complete coverage/oracle checks, normal GC deletes one file and 100,000,050,725 bytes, leaving zero owners/open/retained packs, with no manual data deletion. The 32 MiB four-host boundary workload also passes and is reclaimed normally. Production code is unchanged; no new range-read implementation defect was reproduced.

Caches and background load are uncontrolled; TLC/Verus also run on the writer host. No performance SLA is inferred. Quantiles mix positions, and nearest-rank p99 is the maximum for the fewer-than-100 small-slice samples. Added generation, extra authentication and content comparison mean that comparison with the earlier repeated-pattern workflow cannot attribute a difference to storage speed. Native-call timing excludes the subsequent oracle; whole-workflow timing includes it. This adds no Python/FFI, physical TB-throughput or complete I/O-adapter proof.

## 100 GB validation with fixed 4 MiB checksums (2026-10-24)

The [complete record](../verification/measurements/20261024-tensor-granularity.json) keeps new tensor checksum chunks at 4 MiB, removing the former 512-digest budget. A 100 GB record has 23,842 digests. Readers retain encoded sizes for legacy records. The actual Rust span helper is Verus-proved; Python production and complete I/O refinement remain unproved. See [FORMAT](FORMAT.md#typed-tensors) for metadata growth and cross-policy same-ID retry compatibility.

This run writes 100,000,000,000 actual position-dependent payload bytes and compares every returned byte on four hosts. There are 373 full-read requests of at most 256 MiB and the same 96 small intervals as before (25 empty). Request positions and lengths match; seeds/binaries differ and caches/background load are uncontrolled, so timing changes cannot be wholly attributed to granularity. TLC/Verus/integration checks overlap on the writer host.

| Metric | Previous 256 MiB checksums | Current 4 MiB checksums |
|---|---:|---:|
| Small-interval returned bytes | 94,695 | 94,695 |
| Small-interval authenticated payload bytes | 20,953,550,848 | 330,158,080 |
| Nonempty small-interval native-call p99, n=71 | 552.673 ms | 25.391 ms |
| Full 256 MiB native-call p99, n=372 | 575.895 ms | 533.180 ms |
| 100 GB / complete four-host workflow time | 1534.941 MiB/s | 1756.167 MiB/s |
| Data-file bytes | 100,000,050,725 | 100,003,195,567 |

The same small requests authenticate 1/63.465 as many logical payload bytes. This count follows from interval/chunk geometry; it is not physical backend I/O. One-byte requests authenticate at most 4 MiB. Nonempty N-byte requests authenticate at most `N + 2 * (4 MiB - 1)` bytes, and empty requests authenticate zero payload. Metadata, retries and physical object prefetch are outside this bound.

Metadata preparation/generation takes 111.553 s. Publication/close takes 354.223 s (269.230 MiB/s). Writer peak RSS is 18,064 KiB; the four readers use 280796 / 280804 / 280724 / 280792 KiB. Four-host elapsed time is 54.304 s including SSH, extra slices and the byte oracle. Native-call timing excludes the subsequent oracle. Nearest-rank p99 for n=71 is the sample maximum, not an environment deadline or SLA.

The final binary also handles 32 MiB + 37 bytes with 8193 digests on four hosts; independent Python regenerates every small-workload digest and all request prefixes/suffixes. Two installed-wheel Python producers each publish **2 GiB + 17 bytes**: the baseline uses 257 digests at 8 MiB, while the new version uses 513 at 4 MiB. Slices, same-policy retries and GC pass. Repeating Python data tests the actual API threshold and supplies no physical bandwidth claim. The 100 GB source streams natively rather than allocating a 100 GB Python tensor.

All measured data is reclaimed by normal GC after readers exit, followed by removal of private metadata and remote programs. The 100 GB run deletes one file and 100,003,195,567 bytes, leaving zero owners/open/retained packs. Fresh validation covers 761 Python and 38 native tests, 282 TLC cases, 116 Verus checks and 96 expected semantic rejections, with separate model/binary evidence archives. This supplies no new physical-TB or complete-recovery proof and no claim that the whole system is bug-free.

## Four-host typed-reference preflight measurements (2026-10-24)

The [complete record](../verification/measurements/20261024-typed-preflight.json) reproduces and fixes payload allocation/read before rejecting a mismatched typed descriptor, and unconditional success for bad empty slices. With only 16 MiB additional reader address space, the old 64 MiB bad reference raises allocation `ValueError`; the fix reports typed `CorruptData` first. Checksums remain required and empty slices may still read metadata.

Four hosts each run baseline/fixed installed wheels, creating private 4 KiB, 4 MiB and 64 MiB records and separate child readers. This compares four clients on the deployment, not cross-host visibility of one shared record. Each size/version/host runs 50 bad full loads, 100 bad small slices, 100 bad empty slices and 100 valid small slices: 8,400 calls. Repeating bytes test API/order behavior and provide no physical-bandwidth claim.

| Metric on a 64 MiB record | Baseline | Fixed |
|---|---:|---:|
| Bad full-load process rchar delta p99, n=200 | 67,112,143 bytes | 3,277 bytes |
| Bad full-load latency p99, n=200 | 199.476 ms | 4.995 ms |
| Valid small-slice latency p99, n=400 | 17.483 ms | 35.519 ms |

Across sizes, the baseline accepts 1,200 bad empty slices and the fix rejects all of them. Fixed readers perform metadata work before rejecting large bad descriptors. rchar includes metadata and `/proc/self/io` instrumentation, not exact authenticated payload or physical backend I/O. Worker RSS is a high-water observation; no increase above post-import high water does not imply zero allocation. Normal GC deletes 24 files and 570,510,800 bytes, with zero owners/open/retained packs, before removal of private metadata and remote programs.

The first campaign overlaps formal/integration work and has worse valid-read tail latency; that observation is retained. After full-check processes terminate, a local control uses **one immutable 64 MiB record, two persistent readers and serial ABBA batches**, with 400 valid slices per version. Baseline/fixed p50 is 3.804/3.838 ms; p95 is 9.574/11.775 ms; p99 is 15.891/21.608 ms. This reduces fixture/startup/order differences but does not isolate system-wide load or guarantee cold caches. It cannot rule out every valid-read performance regression, and no p99 becomes an environment deadline.

No new 100 GB or physical-TB run is performed. Earlier throughput retains its original binary identity. Verus proves the actual layout match loop; Python shape products, JSON/FFI, call placement and full I/O/recovery refinement remain explicit gaps.

## Valid small-slice tail-latency attribution (2026-10-24)

The [measurement record](../verification/measurements/20261024-read-latency.json) investigates the valid-read tail difference above. Baseline/fixed wheels remain the fixed-4-MiB and typed-preflight versions; production code is unchanged. Each experiment shares one immutable record between two persistent children, alternating ABBA batches of first-32-byte reads, with 800 calls per version/mode. Three experiments use 64 MiB and 4 KiB JuiceFS records and a 64 MiB local overlay record: 14,400 calls.

| Original API path, caller-boundary timing only | Baseline p50 / p95 / p99, ms | Fixed p50 / p95 / p99, ms |
|---|---:|---:|
| JuiceFS, 64 MiB record | 4.337 / 12.211 / 20.748 | 4.225 / 11.502 / 18.113 |
| JuiceFS, 4 KiB record | 0.535 / 1.129 / 5.774 | 0.541 / 1.597 / 6.648 |
| Local overlay, 64 MiB record | 3.415 / 3.450 / 3.907 | 3.417 / 3.443 / 3.566 |

Two diagnostic modes wrap Python stage boundaries and then additionally preload a libc interposer selecting only the private directory. Their timing is not pooled with the original API path. Every call checks returned bytes and records actual native/Python/shim hashes, wall/thread CPU times and raw events. No delays or errors are injected. Two earlier stage-only experiments comprising 6,400 calls are archived separately with their original script/hash and are not included in this table.

Every libc sample has two `open64` calls and two closes in both versions. The 64 MiB case authenticates 4,194,304 payload bytes; nine reads return 4,197,460 bytes including metadata. The 4 KiB case authenticates 4,096 bytes; six reads return 5,222 bytes. In the implementation, `Config::frame` first calls `inspect_once` to open and inspect the index, then `frame_with_index` opens the same reference's file to check its envelope and read payload. These are not physical backend I/O or RPC counts.

For the eight slowest diagnostic calls per version on the 64 MiB JuiceFS record, summed open time accounts for 86.3%/78.1% of summed API time; the 4 KiB case gives 96.6%/95.7%. The slowest baseline call takes 70.694 ms, including one 65.655 ms open, with 3.554 ms total thread CPU. On overlay, summed-open p99 is 0.011/0.010 ms. Python encoding p50 in stage samples rises from about 20.3 to 25.1 µs, insufficient to explain the tens-of-milliseconds tail here.

This locates these slow samples at the file-open call boundary; it does **not** identify JuiceFS internals, network or server behavior. Wall minus thread CPU includes scheduling and other waits. The interposer covers selected paths/descriptors and listed libc functions, not arbitrary syscalls or other threads. The baseline/fixed p99 ordering changes between experiments, so this does not establish a fixed typed-preflight tail cost or rule out regressions on every workload.

Reusing the inspected descriptor within an attempt is a concrete optimization candidate. Any implementation must retain fresh-open retries, index/envelope binding and payload authentication. This round implements no such optimization and claims no new correctness defect. `VisibleBy` remains an attempt count and 3.85 seconds remains the sum of retry sleeps; neither becomes these p99 values. The 48 relevant read TLC cases and 118 positive Verus checks are rerun; full integration suites and Verus negative controls are not reported as fresh executions.

Normal GC after reader exits deletes three files totaling 134,230,390 bytes, leaving zero owners/open/retained packs, before private metadata removal. Repeated data, one position and unevicted caches limit the evidence. This round's formal checks follow all measurements, but there is no host-wide load isolation. No new four-host, 100 GB or physical-TB throughput, latency bound or bandwidth SLA is claimed.

## Reusing the inspection descriptor: four-host control and 100 GB (2026-10-24)

The [complete record](../verification/measurements/20261024-single-open.json) passes the inspection descriptor into ordinary and cold-session frame reads, avoiding a second successful open. Warm sessions still open fresh files; sessions cache only authenticated indexes. Failed attempts close the data file before retry. Actual frame-position arithmetic has a Verus proof; the complete descriptor/I/O adapter does not.

Each of four hosts creates private 4 KiB/64 MiB records. Persistent baseline typed-preflight and new-wheel processes alternate ABBA batches reading the same record's first 32 bytes. There are 800 calls per size/version/mode/host, totaling 38,400. The table uses original-API caller-boundary timing only, n=3,200 per pooled group; Python stage and libc instrumentation are recorded separately.

| Record size | Baseline p50 / p95 / p99, ms | New p50 / p95 / p99, ms |
|---|---:|---:|
| 4 KiB | 0.521 / 1.086 / 5.439 | 0.361 / 0.656 / 4.494 |
| 64 MiB | 3.775 / 8.930 / 17.015 | 3.602 / 7.982 / 14.977 |

Individual-host original-path p99 values, n=800 per group, preserve differences hidden by pooling:

| Host | 4 KiB baseline/new p99, ms | 64 MiB baseline/new p99, ms |
|---|---:|---:|
| client-1 | 4.327 / 2.910 | 21.175 / 14.191 |
| client-2 | 5.652 / 4.703 | 14.695 / 16.477 |
| client-3 | 4.812 / 5.007 | 17.434 / 13.751 |
| client-4 | 6.361 / 4.483 | 16.661 / 14.847 |

Every libc sample reduces opens/closes from 2/2 to 1/1 and statx from 5 to 3. Six reads still return 5,222 bytes for the 4 KiB record; nine return 4,197,460 for 64 MiB, with 4,096/4,194,304 authenticated payload bytes respectively. These samples have no name/visibility retry: `open_committed` separately permits five File::open attempts on NotFound, so these counts are not a one-syscall guarantee under arbitrary faults.

All individual-host p50 values decrease, but two host/size combinations have higher p99. For 64 MiB, pooled Python-stage p99 rises from 20.795 to 22.538 ms while libc-mode p99 falls from 24.913 to 21.685 ms. Instrumentation changes timing; caches and cluster-wide load are uncontrolled. This cannot promise every tail improves. These runs follow this round's formal, integration and 100 GB jobs. Each host reads its own record; this is not a cross-client publication-visibility experiment.

A native probe built from the same production source separately publishes **100,000,000,000 position-dependent bytes**, with 23,842 checksums of 4 MiB each. Four hosts compare every returned byte across 373 full windows of at most 256 MiB and 96 boundary intervals (25 empty). Total returned/authenticated bytes are 100,000,094,695/100,330,158,080. Preparation takes 112.335 s; publication/close takes 352.091 s, 270.860 MiB/s. Four-host completion takes 55.239 s including SSH, extra intervals and the oracle, or 1726.467 MiB/s. Writer RSS high water is 18,080 KiB; readers use 280736/280776/280720/280808 KiB.

Full-256-MiB native p99 is 546.046 ms (n=372). Nonempty small-range p99 is 66.614 ms (n=71, thus the sample maximum), above the earlier granularity run's 25.391 ms. The slowest range is local `[0, 4099)`, authenticating 4 MiB; this large-file run has no syscall-stage timing, so its cause remains unattributed. It uses sessions, so reuse only removes the initial second open per reader; later calls already had one open. Different seeds, caches, background load and binaries prevent attributing cross-run bulk speedups or a fixed regression. Verification work overlaps part of the writer lifetime; publication throughput is not a raw-filesystem maximum.

The same binary also passes a four-host 32 MiB + 37-byte workload with 8,193 digests. Independent Python regenerates every smoke digest and checks every remote prefix/suffix; the native oracle compares all returned bytes. Normal GC reclaims 100,003,195,567 bytes for the large file, 34,653,048 for the smoke fixture and 268,471,976 across eight latency fixtures, leaving zero owners/open/retained packs. Private metadata and remote programs are removed afterward, with no manual data deletion. No new physical-TB throughput is claimed; sparse large offsets remain separately covered by full regressions.

Full checks pass: 782 Python / 40 native tests, 302 TLC cases, 119 Verus checks and 105 expected semantic rejections. Successful descriptor acquisition count is algorithm policy; observed quantiles and throughput do not become environment time bounds.

## Combined name lookup and data retries (2026-10-24)

The [record](../verification/measurements/20261024-read-lookup.json) checks name retries in `open_committed` together with outer visibility retries. Production Rust/Python and the wheel match the descriptor-reuse round. This round adds models/probes and finds no new counterexample requiring a production read-policy change.

Each of four hosts runs 84 actual straw API injection cases on private 4 KiB/64 MiB records, covering ordinary reads and cold/warm sessions. An exact-path libc shim supplies one/four/five ENOENT responses, EIO, permission/quota/space errors, and interleaved payload authentication failures. Payload faults flip one returned buffer byte without modifying disk. Every case is followed by an unarmed full-byte comparison. All 336 cases match expectations; failed descriptors close before subsequent opens. Normal GC reclaims eight files totaling 268,456,360 bytes.

The maximum-budget cases place four ENOENT responses before each successful open, then fail data authentication five times before a sixth success/failure. Both paths show thirty libc opens and six acquired/closed descriptors. With no EINTR in these samples, this agrees with the model's thirty Rust File::open calls. Requested sleeps total 4,750 ms. The 64 MiB recovery cases take 5,255.787–5,373.256 ms (n=12), also including reading, authentication, allocation and scheduling; the sleep sum is not an API timeout.

A separate four-host supplement runs 120 finite-EINTR cases. Forty EINTR responses before success produce forty-one libc opens; adding four ENOENT responses before success produces forty-five. This matches [Rust 1.90's open implementation](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/fs/unix.rs#L1185) and internal [`cvt_r` retry](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/pal/unix/mod.rs#L310). **Thirty bounds straw's File::open invocations, not libc/kernel calls or RPCs.** The shim returns EINTR without sending signals; it proves no termination under infinite interruption. The supplement normally reclaims another eight files. Local development checks separately cover 84/30 cases and normally reclaim four files. Each probe version and binary is archived separately rather than labeling an earlier run with later source.

Natural observations are separate. Four writers take turns creating new raw files. Before creation, the other three clients open the name and receive NotFound. After writing and syncing the file and containing directory, the writer replies over SSH and peers are told to open, read fully and verify SHA-256. Each writer creates fifty names per size: four hundred files and 1,200 remote reads. Every first post-publication open succeeds.

| File size, n=600 per group | First open p50 / p95 / p99 / max, ms | Full read and verification p50 / p95 / p99 / max, ms |
|---|---:|---:|
| 2 KiB | 0.248 / 0.804 / 4.528 / 7.403 | 0.935 / 3.999 / 7.716 / 9.988 |
| 4 MiB | 0.271 / 3.543 / 6.773 / 71.509 | 12.416 / 18.521 / 26.998 / 86.566 |

Open timing starts at the reader's call; full verification includes open/read/hash. Separate notification-to-reply measurements include SSH, scheduling and coordinator reply collection. No cross-host clocks are subtracted. Background load and caches are uncontrolled; some formal checks overlap measurements. These raw files are not straw publications, and timings are not end-to-end straw performance or pure propagation latency. Once all workers exit, four hundred private raw files totaling 839,270,400 bytes are removed; this cleanup is recorded separately from normal straw GC.

No 100 GB or physical TB workload is rerun. The preceding 100 GB evidence for the same production source retains its original run identity. Finite observations establish neither a guaranteed `NameBy`/`VisibleBy` nor absence of name delays or full-system bugs.

## Offline cleanup bypassing shared ownership (2026-10-24)

The [record](../verification/measurements/20261024-offline-guard.json) reproduces actual deletion under a controlled boundary fault. `inspect_run` used `Path.exists()` for `storage.log`/`queues`. A false stat NotFound during both initial inspection and final reinspection let the root queue's journal classify an independently protected pack as an orphan. Even with all participants stopped, the old cleanup deleted checkpoint-owned or peer-queue-accepted data. This is injected behavior, not naturally observed stale JuiceFS stat.

Each of four hosts runs sixteen scenarios per installed old/new wheel: catalog/queues layouts, 2 KiB/8 MiB payloads, and stat NotFound, stat EIO, open EIO or open EACCES. Catalog fixtures are managed from initialization, with an independent checkpoint owner and no root-queue reference; there is no live migration. Queue fixtures use a legacy root queue and a separate namespace queue whose WAL accepted the input. Every queue and writer is closed before inspection.

| Four-host result | Baseline | Fixed |
|---|---:|---:|
| Scenarios | 64 | 64 |
| Protected packs actually deleted | 16 | 0 |
| Unsafely deleted bytes | 67,144,696 | 0 |
| Catalog owner or peer WAL remains valid | All | All |

All sixteen baseline losses follow false stat absence. Eight leave a live checkpoint owner; eight leave an accepted peer WAL, while subsequent data reads fail. Python 3.12 already raises stat EIO, so those cases do not lose data; this defect must not be described as swallowing every stat error. Baseline guards reject normally visible markers before reaching open, leaving open-fault arms unused.

The fixed guard performs read-only fresh opens without reading catalog contents. Present markers and non-FileNotFoundError failures raise `UnsafeRecovery`; cleanup still reinspects. Fixed stat arms remain unused. Open EIO/EACCES arms fire and are refused. Each fixed scenario tries both inspection and cleanup with an unvalidated report; both refuse, and full-byte reads still match. Both wheels contain the identical native binary; only the Python inspector changes in production.

After protection checks, catalog fixtures explicitly release their checkpoint and run normal GC. Baseline GC reclaims twenty-four files/100,717,044 bytes; the eight already deleted catalog files are not counted as GC. Fixed GC reclaims thirty-two files/134,289,392 bytes. Catalog-free namespace fixtures require private fixture teardown, separately removing twenty-four/thirty-two surviving files and 100,717,044/134,289,392 bytes. Sixteen local cases per version and the initial Python false-stat reproduction are separate from the table. All private pools, fault arm/trace directories and remote environments are removed.

This fixes offline administrative safety, with no new bandwidth or p99 guarantee. No 100 GB/physical TB workload is rerun; Rust read paths retain the prior native binary. `ENV-OFFLINE` still requires truthful fresh-open absence and actual quiescence/stable namespace during final inspection/deletion. The environment control exposes the fresh-open dependency; actual quiescence remains a caller obligation. Previously deleted data is not repaired, and full Python/I/O refinement remains unproved.

## Offline alias cleanup comparison (2026-10-25)

The [record](../verification/measurements/20261025-offline-alias.json) compares the preceding fresh-open-guard wheel with an alias-aware inspector on the four hosts in `the configured host list`. Every fixture is a private, stopped, catalog-free single queue under `/shared/straw-test`; all aliases exist before acceptance. No syscall faults or namespace rebinding are injected. Each host runs twenty-eight scenarios per version: seven reference shapes, 2 KiB/8 MiB payloads, and presence/absence of a second accepted canonical reference.

| Observation across four hosts | Baseline | Fixed |
|---|---:|---:|
| Scenarios | 112 | 112 |
| Accepted references unreadable after cleanup | 88 | 0 |
| Accepted physical files deleted | 56 | 0 |
| Accepted file survives but required alias is deleted | 32 | 0 |
| Unsafely deleted bytes | 235,006,436 | 0 |
| Independent orphan packs reclaimed | 112 | 112 |

Shapes cover a standalone manifest alias, a packed chain through a partial file, partial/sealed directory aliases, a directory traversed before `..`, an explicit aliased record and a transitive dependency. The original journal remains unchanged in every case. The fix preserves readable payloads byte-for-byte while still deleting independent orphan packs and dangling/orphan partial aliases. A separate unreferenced alias target outside raw enumeration must survive, proving cleanup unlinks the selected name rather than substituting its target. Both versions use the identical native binary; this is a Python administrative change.

The report's `retained_paths` now contains resolved physical identities. Candidate lists still contain lexical names. The inspector protects the full reference-resolution trace, not only final canonical ancestors, and applies that check to partials. Unknown/escaping candidate identities abort inspection; failed retained-reference tracing disables deletion. All private pools and remote environments are removed after validation. Fixture teardown is separate from intended offline orphan cleanup and the baseline's intentional unsafe deletion; none is described as catalog GC. Twenty-eight local cases per version and the two original four-case reproductions are separate evidence.

No 100 GB workload or physical TB run is repeated for this Python change. The prior native throughput measurements still apply only to their measured conditions, without a new bandwidth or p99 claim. Quiescence, truthful resolution, immutable data and external namespace stability remain assumptions; the new environment contract explicitly does not assume that straw preserves the aliases it is being checked for deleting.

## Runtime comparison during verification work (2026-10-25)

The [measurement record](../verification/measurements/20261025-runtime-performance.json) compares the v0.1.3 commit with the release wheel from the catalog-event verification checkpoint. It covers all intervening implementation fixes and refactors, so it does not isolate the cost of proof annotations. Proof functions, specifications and ghost histories are excluded from ordinary builds. Actual changes such as write admission, descriptor/length checks and catalog operation construction still execute and can affect performance.

Both wheels use the same release toolchain and run in separate installed environments on one host. Each filesystem/workload group uses four trials per version in B C C B C B B C order. The first campaign has sixteen trials with 1 GiB of data, 500 synchronous WAL appends and five replays of a 2,000-frame catalog per trial. Forty-eight further trials pin each process to the same CPU and separately measure 2 GiB local data, 1 GiB JuiceFS publication with online GC enabled, and five replays of a 20,000-frame catalog in both legacy and normalized path encodings. Catalog fixtures are byte-identical between versions for each encoding.

| Measurement | v0.1.3 median | Current median | Change |
|---|---:|---:|---:|
| JuiceFS publication, GC disabled, MiB/s | 214.98 | 217.22 | +1.04% |
| JuiceFS checked reads, GC disabled, MiB/s | 371.58 | 366.76 | −1.30% |
| JuiceFS publication, GC enabled, MiB/s | 108.93 | 109.21 | +0.26% |
| Pinned local publication, GC disabled, MiB/s | 219.53 | 215.56 | −1.81% |
| Pinned local checked reads, GC disabled, MiB/s | 504.47 | 503.32 | −0.23% |
| JuiceFS synchronous WAL, mean ms/frame | 15.455 | 15.676 | +1.43% |
| JuiceFS legacy catalog, 20,000 frames, ms | 53.567 | 58.100 | +8.46% |
| JuiceFS normalized catalog, 20,000 frames, ms | 57.899 | 60.949 | +5.27% |
| Local legacy catalog, 20,000 frames, ms | 40.278 | 34.909 | −13.33% |
| Local normalized catalog, 20,000 frames, ms | 43.154 | 37.257 | −13.67% |

These are medians across trials; replay first takes the median of five fresh-store calls in each trial. Higher throughput is better, whereas higher latency is worse. The record retains every trial and range. In particular, GC-enabled baseline publication ranges from 31.89 to 111.33 MiB/s, versus 103.03 to 111.30 for the current version. The unusually slow baseline trial is retained; its CPU time is close to the other trials. Pinned local write CPU time increases by 1.98%, and normalized JuiceFS catalog CPU time by 7.55%. The measurements do not establish which implementation change or external wait accounts for each difference.

The data workload reuses a seeded random 32 MiB payload, checks every returned byte and times durable publication separately from final writer close. Reads and replay use warm caches; there is no global cache eviction or cluster-wide background-load isolation. Catalog timing includes replay, root/candidate snapshots and a recovery barrier, with no physical GC deletions. These are logical application rates, not isolated device/object-store traffic, queue RPC throughput or a repeat of the four-client 100 GB workload. Four trials per version do not establish stable tail latency or a universal absence of regressions.

The shared-filesystem catalog slowdown remains visible in this comparison; passing formal checks is not a performance acceptance result. No production source changes during this campaign. Exact historical probes, wheels, source, raw reports and hashes are retained separately. The reproduction probe now records raw latency samples and nearest-rank percentiles; the original probes' offset percentile indexing is documented in the record, and none of those percentile fields is used in the table above.

## Catalog operation construction comparison (2026-10-25)

The [construction checkpoint](../verification/measurements/20261025-catalog-decode.json) compares its release wheel with the preceding catalog-event wheel, not with v0.1.3. One host ran 64 trials, alternating versions on the same CPU with fresh private fixtures. Each row has four trials per version and three warm replay/snapshot calls per trial; values are medians of trial medians. The fixture hash matches across versions. Multiple paths per owner exercise bulk set construction.

| Filesystem | Encoding | Frames | Paths per owner | Before ms | After ms | Time change |
|---|---|---|---|---|---|---|
| local | legacy | 20,000 | 1 | 35.442 | 35.154 | -0.81% |
| local | legacy | 2,000 | 64 | 59.064 | 58.589 | -0.80% |
| local | normalized | 20,000 | 1 | 36.721 | 36.802 | +0.22% |
| local | normalized | 2,000 | 64 | 50.031 | 49.904 | -0.25% |
| shared | legacy | 20,000 | 1 | 51.739 | 50.727 | -1.95% |
| shared | legacy | 2,000 | 64 | 75.996 | 75.563 | -0.57% |
| shared | normalized | 20,000 | 1 | 51.824 | 53.779 | +3.77% |
| shared | normalized | 2,000 | 64 | 70.380 | 67.625 | -3.92% |

The shared normalized one-path case increased 3.77%; the other changes range from −3.92% to +0.22%. These small warm-cache samples do not establish zero overhead or a throughput/latency guarantee. They do not repeat the payload bandwidth or 100 GB workload. All completed comparison samples are retained and their private roots removed. A preliminary setup attempt was stopped by the installed-binary guard before measuring the new build, because pip had reused a global package of the same version; its unmatched baseline sample and failure log are retained separately and excluded from the balanced campaign.

## Commit dispatcher comparison (2026-10-25)

The [commit/recovery checkpoint](../verification/measurements/20261025-commit-recovery.json) compares its release build with the preceding catalog-decode build. `runtime_performance.py --phase commits` uses online GC, one shared immutable input, task acceptance/cancellation and final physical reclamation. Sixteen alternating trials use one CPU: four per version/filesystem, 128 tasks per local trial and 64 per shared trial. All roots are private and removed after validation.

| Filesystem | Operation | Unit | Before | After | Time change |
|---|---|---|---|---|---|
| local | accept | ms/task | 0.270 | 0.255 | -5.74% |
| local | retire | ms/task | 0.061 | 0.070 | +15.31% |
| local | collect | ms/collection | 0.311 | 0.305 | -2.05% |
| shared | accept | ms/task | 61.957 | 63.409 | +2.34% |
| shared | retire | ms/task | 30.750 | 29.980 | -2.51% |
| shared | collect | ms/collection | 112.816 | 99.808 | -11.53% |

The short local retirement sample increased about 9 µs/task (+15.31%). A separately recorded follow-up ran six alternating trials per version with 2,048 tasks each, retaining the original samples. Local acceptance changed −0.48% wall/−0.36% CPU; retirement changed +0.66% wall/+1.76% CPU. The initial retirement increase was not reproduced at that size. Shared acceptance increased 2.34%. These warm-cache, single-host observations do not prove zero overhead or a latency SLA and do not repeat the 100 GB payload workload.

## Catalog recovery composition comparison (2026-10-25)

Compare this recovery composition against the preceding commit-recovery release wheel. Alternate versions on one fixed CPU/host, with four trials per filesystem/encoding/version (32 trials total). Each private fixture has 4,096 catalog frames and four paths per owner; each trial performs seven recoveries with root/candidate snapshots. The table summarizes trial medians. No cache eviction is performed; these are neither 100 GB bandwidth measurements nor p99 bounds.

| Filesystem | Path encoding | Before → after (ms/recovery and snapshot) | Wall change | CPU change |
|---|---|---|---|---|
| local | legacy | 11.712 → 11.759 | +0.41% | +0.33% |
| local | normalized | 12.548 → 12.400 | -1.18% | -1.18% |
| shared | legacy | 28.944 → 29.045 | +0.35% | -0.60% |
| shared | normalized | 28.741 → 28.933 | +0.67% | -0.39% |

All trials, raw samples and binary hashes are retained in the [checkpoint record](../verification/measurements/20261025-catalog-recovery.json). This sample does not show a substantial regression, but does not establish zero runtime cost; proof coverage and performance acceptance remain separate.

## GC batch dispatch comparison (2026-10-25)

The [batch deletion checkpoint](../verification/measurements/20261025-gc-batch.json) compares the preceding catalog-recovery wheel with the verified batch dispatcher. Sixteen alternating trials use one host and fixed CPU, four trials per version/filesystem. Each trial performs three collections on fresh fixtures with three writer directories. Local fixtures have 64 packs per writer; shared filesystem fixtures have eight. Fixture publication, sealing and staging release are outside the timer. Collection includes refresh, candidate selection, durable marks, unlink, parent barriers and receipts.

| Filesystem | Packs per collection | Before → after (ms/collection) | Wall change | CPU change |
|---|---:|---:|---:|---:|
| Local | 192 | 9.649 → 9.827 | +1.84% | +0.71% |
| Shared | 24 | 156.157 → 167.855 | +7.49% | +1.31% |

Values are medians of the four trial medians. Shared trial medians range from 151.833–209.022 ms before and 161.688–187.643 ms after. All samples and exact native hashes are retained. The measured slowdown remains visible; this sample neither attributes its cause nor establishes no regression. The refactor adds no filesystem calls and uses checked accounting, but source inspection alone does not establish equal runtime cost. Caches and background load are uncontrolled. This is collection latency, not a repeated 100 GB bandwidth test or a p99 bound.
