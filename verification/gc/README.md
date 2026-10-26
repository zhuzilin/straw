# GC

[总入口 / Index](../README.md)

在线发现、所有权释放后的回收、删除屏障、持久回执和条件活性。 / Online discovery, reclamation after ownership release, deletion barriers, durable receipts and conditional liveness.

模型与配置在 `tla/`，部署实验与配套注入程序在 `probes/`。如有 `checks/` 或 `verus/`，分别放可执行检查和数学证明。生产 Rust 的证明合约仍随实现在仓库 `rust/` 中。

Models/configurations live in `tla/`, deployment experiments and their shims in `probes/`. Optional `checks/` and `verus/` contain executable checks and mathematical proofs. Production Rust proof contracts remain with their implementations in the repository `rust/` directory.

从仓库根目录运行 / Run from the repository root:

```sh
python verification/runners/check_tla.py --area gc
python verification/runners/environment_manifest.py --model GCBatchDeletion
```

不加 `--area` 运行完整模型集合。完整 Rust 证明共用 `verification/runners/check_verus.py`。 / Omit `--area` for the complete model suite. All Rust proofs use the shared Verus runner.

| Model | Configuration |
|---|---|
| [WriterRetirement](tla/WriterRetirement.tla) | [WriterRetirement.cfg](tla/WriterRetirement.cfg) |
| [GCBatchDeletion](tla/GCBatchDeletion.tla) | [GCBatchDeletion.cfg](tla/GCBatchDeletion.cfg) |
| [GCDeletion](tla/GCDeletion.tla) | [GCDeletion.cfg](tla/GCDeletion.cfg) |
| [GCDiscovery](tla/GCDiscovery.tla) | [GCDiscovery.cfg](tla/GCDiscovery.cfg) |
| [GCParentRepair](tla/GCParentRepair.tla) | [GCParentRepair.cfg](tla/GCParentRepair.cfg) |
| [GCReceipt](tla/GCReceipt.tla) | [GCReceipt.cfg](tla/GCReceipt.cfg) |
| [UsageCache](tla/UsageCache.tla) | [UsageCache.cfg](tla/UsageCache.cfg) |
| [WriterLifetime](tla/WriterLifetime.tla) | [WriterLifetime.cfg](tla/WriterLifetime.cfg) |

证明边界和待补项见 [FORMAL_VERIFICATION](../../docs/FORMAL_VERIFICATION_zh.md)。 / See [formal verification](../../docs/FORMAL_VERIFICATION.md) for proof boundaries and outstanding work.

实际 catalog 的索引与 replay 状态现由 [catalog_log.rs](../../rust/catalog_log.rs) 统一验证，详见[联合状态与有类型历史](../../docs/FORMAL_VERIFICATION_zh.md#catalog-联合状态与有类型-frame-历史)。这尚未建立完整逻辑 owner、closure cache 或物理 GC 的调用链对应。

[gc_batch.rs](../../rust/gc_batch.rs) 是实际调用的批量删除执行体：Verus 检查每个目标的 mark／unlink、全部共享父目录屏障和回执顺序，包括 NotFound 与报错前副作用。`ENV-GC-BATCH-EFFECTS` 保留原生路径／目录修复／catalog 持久回执的适配义务。实际 [gc_io](../../rust/gc_io.rs) 已证明 unlink／父目录分派，[directory_io](../../rust/directory_io.rs) 已证明 mkdir 的完整有序屏障及失败前缀；完整路径／祖先绑定、repair 聚合持久性、根集合到 mark 和完整活性尚未闭合。[I/O 证据](../measurements/20261025-gc-io.json)。见[本阶段证据](../measurements/20261025-gc-batch.json)。

Production `gc_batch::collect` now composes every marked target, unlink, shared-parent barrier and receipt under explicit lower-effect contracts. Actual `gc_io` dispatch and `directory_io` ancestor barriers are also checked. Native target/path/handle mapping, complete ancestor enumeration and aggregate repair durability, roots-to-mark composition and progress remain open. [Formal scope](../../docs/FORMAL_VERIFICATION.md#actual-gc-batch-deletion-dispatch).

[gc_mark.rs](../../rust/gc_mark.rs) 将实际根／候选扫描连接到精确 JSON mark、持久追加和完整 pending 快照，包括写错误隔离；原生锁／命名空间、逻辑闭包、删除适配与活性仍未闭合。[证据](../measurements/20261025-gc-mark.json)。

Production `gc_mark::mark` connects the exact scanned candidates to a durable catalog mark and the complete pending snapshot, preserving uncertain-write quarantine. Native lock/namespace/closure/deletion adapters and progress remain open. See the same evidence record above.

[gc_receipt.rs](../../rust/gc_receipt.rs) 将实际规范目标集合连接到持久回执及精确 pending／pack 移除，保留永久 tombstone；物理删除／路径／父目录对应仍单列。[回执证据 / Receipt evidence](../measurements/20261025-gc-receipt.json)。 The actual receipt encoder/append proves logical catalog effects, while physical absence remains a separate native caller obligation.

实际 `gc_collection::Batch::run` 已把 mark、完整 pending 快照、批量 unlink、共享父目录与祖先修复屏障及持久 receipt 接成经过证明的生产调用链。 / The actual mark-to-receipt chain now establishes exact final indexes and durable absence for the whole batch under named filesystem primitives. Outer locked refresh, queue retirement/root provenance and conditional liveness remain separate. [Scope and exact evidence](../measurements/20261026-gc-collection.json).
