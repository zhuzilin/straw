# 共享验证基础 / Shared verification support

[总入口 / Index](../README.md)

`tla/` 只放被多个功能消费的环境模块；单一功能的环境模块随其模型放置。`verus/verus.rs` 是统一证明入口，导入实际生产 Rust、`protocol/verus/control_plane.rs` 、`protocol/verus/retry_liveness.rs` 和 `gc/verus/gc_liveness.rs`。`verus/environment.rs` 保存共享的 Verus 环境契约。

`tla/` contains environment modules shared across feature areas. Environments used by one area stay beside that area's models. `verus/verus.rs` is the combined entry point, importing production Rust, protocol model theorems and GC liveness proofs. `verus/environment.rs` contains shared Verus contracts.

Run from the repository root:

```sh
python verification/runners/environment_manifest.py --check
python verification/runners/check_verus.py --verus /path/to/verus
```

[环境契约 / Environment contracts](../../docs/ENVIRONMENT_MODELS.md) distinguish external assumptions from straw obligations. Shared location does not imply these assumptions have been proved.

`verus/serde_json.rs` 单列锁定 serde_json 的八个基础接口和三个不透明类型规范；这是显式可信库边界，运行器对白名单和输入哈希单独检查。straw 的 `rust/catalog_json.rs` 解码函数体直接验证；不假设路径解析或完整业务 replay 正确。

`verus/serde_json.rs` is the audited external library specification surface: eight primitive methods and three opaque types. The runner builds the locked real library with its pinned compiler and records its hashes. Actual straw wire decoding is verified; path resolution and complete business replay remain separate obligations.

`probes/runtime_performance.py` compares installed builds using durable publications, checked reads, WAL appends and catalog replay. Use the same arguments with each environment's Python, alternate versions, and choose fresh private roots. `--phase`, `--online-gc`, `--catalog-encoding`, `--catalog-paths` and `--cpu` select independent controls. Reports bind the native library hash and preserve raw operation latencies. The probe removes only the private workload root it created after successful checks. It does not evict global caches or establish a performance SLA; see the [recorded comparison](../../docs/FILESYSTEM.md#runtime-comparison-during-verification-work-2026-10-25).

`probes/runtime_performance.py` 对安装版本分别测量持久发布、校验读取、WAL 和 catalog 重放。交错运行两版、保持参数一致并使用新私有目录；报告绑定 native 哈希、保留原始操作延迟，成功核对后删除本轮创建的私有负载目录。它不清空全局缓存，也不建立性能 SLA；见[已记录的对照](../../docs/FILESYSTEM_zh.md#验证工作中的运行性能对照2026-10-25)。

`verus/collections.rs` 单列两个标准 HashMap 消费迭代原语及一个不透明类型，runner 对独立白名单进行审计。 / Two trusted HashMap consuming-iterator primitives and one opaque type are audited separately from the verified straw loops. `ENV-SET-BUILDER` retains the native bulk-set adapter obligation.

`runtime_performance.py --phase commits --commit-tasks 64` measures the complete retain/WAL/apply/cleanup dispatch with online GC, task retirement and final reclamation of one shared input. / 该模式测量真实提交调用链、任务退休及最终回收，始终开启在线 GC。

`runtime_performance.py --phase gc --gc-writers 3 --gc-packs 8 --gc-rounds 3` measures actual collection of multiple packs sharing each writer directory. Fixture publication, sealing and release happen outside the timer; collection includes refresh, mark, unlink, barriers and receipts. / 该模式测量共享父目录的多文件回收；准备负载不计时，实际 collection 全流程计时，保留逐轮 wall／CPU 样本。
