# Changelog

[中文版](CHANGELOG_zh.md)

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
[protocol guarantees](docs/PROTOCOL_AND_VERIFICATION.md).
