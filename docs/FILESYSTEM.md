# Filesystem deployment

[中文版](FILESYSTEM_zh.md)

straw uses a **mounted Linux/POSIX filesystem**, with real files, directories,
append writes, file/directory fsync and advisory file locks. It does not send
object-store API requests or replace a filesystem client. Local filesystems
work for one machine; several machines need the same shared pool contents.

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
