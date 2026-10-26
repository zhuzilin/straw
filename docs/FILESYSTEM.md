# Filesystem deployment

[中文版](FILESYSTEM_zh.md)

straw uses a mounted Linux/POSIX filesystem. Multiple clients need the same
pool, stable names and compatible durability/lock semantics; straw does not
call object-store APIs directly.

## Deployment identity and durability

The [spec environment contract](DESIGN.md#environment) defines the protocol premises. This guide explains how to configure and qualify a deployment.


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
[spec environment assumptions](DESIGN.md#environment).

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

Run the [multi-client crash/GC checks](#qualification) on an isolated directory
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

<a id="qualification"></a>

## Several filesystem clients

On a shared checkout and mount, with the same package/interpreter available
on four hosts and SSH configured:

```sh
python verification/wal/checks/crash_campaign.py \
  --hosts HOST_A HOST_B HOST_C HOST_D --parent /shared/test-tmp \
  --output /tmp/straw-four-client-crashes.json
```

The launcher runs on the first host. The harness moves child crash/handoff
cases among four clients and checks recovered ownership before source release.
It uses the shared verification script; application users running installed
wheel benchmarks do not need this source checkout.

The [multi-host benchmark](BENCHMARKS.md) adds concurrent packed I/O, durable
acceptance, independent readers and online collection. Bound its storage budget
and use a fresh root. Verify the returned byte counts, exit status and cleanup
record, not just throughput or the launcher's success message.

## Focused GC probes

`verification/gc/probes/gc_scale.py` measures ownership cost with tiny records. Start one
copy on each client, with a distinct queue name and the same fresh root:

```sh
python verification/gc/probes/gc_scale.py --root /shared/new-gc-scale \
  --queue HOST_A --count 1024 --participants 4
```

Repeat on HOST_B/C/D concurrently. Each process submits 1,024 roots in its
namespaced queue, joins a four-client barrier, reopens its coordinator, times
GC, reads sampled live records and verifies deletion of a dead pack. Increase
to `--count 16384` for 65,536 total roots. Timing excludes coordinator/catalog
replay and does not measure tensor bandwidth or cold storage. The caller owns
cleanup **after all four processes stop**. To continue an interrupted fixture,
confirm its old workers stopped before using `--resume` on the same queues.

`verification/gc/probes/gc_cross_client.py` isolates cached positive attributes after a
remote deletion. Prepare a unique root on one client:

```sh
python verification/gc/probes/gc_cross_client.py --root /shared/new-gc-cache
```

Start the following on each of the four clients with distinct `--worker` names:

```sh
python verification/gc/probes/gc_cross_client.py --root /shared/new-gc-cache --worker HOST_A
```

Each client primes its stat cache and writes one `ready-<worker>` marker. After
all four markers are present, the supervisor creates `/shared/new-gc-cache/go`.
The workers collect concurrently; each must exit successfully and their total
`reclaimed_files` must be one. Only then remove the private root. The 60-second
barrier deadline bounds a failed setup. This fixture has one pack and a fixed
number of markers, not a file per sample.

## Failure scope and reports

Preserve failing source/build identity, operation IDs and a small reproducer.
Process exit leaves the kernel and storage service alive; qualify actual locks,
cache settings, service interruption, device/VM power loss and ENOSPC/EIO
separately. A background failure is not rollback, and a lost RPC reply does not
authorize releasing uncertain ownership.

Public performance results and methods live in
[benchmarks](BENCHMARKS.md#published-results). Keep raw deployment logs in
controlled local/CI artifacts. Never remove data still used by readers or
uncertain processes.
