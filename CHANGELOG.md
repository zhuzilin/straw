# Changelog

[中文版](CHANGELOG_zh.md)

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
