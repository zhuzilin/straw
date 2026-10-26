# Shared verification support / 共享验证基础

[总入口 / Index](../README.md)

`tla/` contains environment modules shared by several models; single-area environments stay beside their consumers. Shared location does not establish an assumption's truth. Review external guarantees separately from straw caller obligations in the [environment review](../../docs/DESIGN.md#environment).

`tla/` 保存多个模型共用的环境模块；单个领域的环境随模型放置。共享不代表假设已被证明。按[环境审查](../../docs/DESIGN_zh.md#environment)区分外部保证和 straw 调用者义务。

```sh
python verification/runners/environment_manifest.py --check
```

`probes/runtime_performance.py` compares installed builds using durable publications, checked reads, WAL appends and catalog replay. Use the same arguments with each environment's Python, alternate versions, and choose fresh private roots. `--phase`, `--online-gc`, `--catalog-encoding`, `--catalog-paths` and `--cpu` select independent controls. Reports bind the native library hash and preserve raw operation latencies. The probe removes only the private workload root it created after successful checks. It does not evict global caches or establish a performance SLA; see the [recorded comparison](../../docs/BENCHMARKS.md#published-results).

`probes/runtime_performance.py` 对安装版本分别测量持久发布、校验读取、WAL 和 catalog 重放。交错运行两版、保持参数一致并使用新私有目录；报告绑定 native 哈希、保留原始操作延迟，成功核对后删除本轮创建的私有负载目录。它不清空全局缓存，也不建立性能 SLA；见[已记录的对照](../../docs/BENCHMARKS_zh.md#published-results)。

`runtime_performance.py --phase commits --commit-tasks 64` measures the complete retain/WAL/apply/cleanup dispatch with online GC, task retirement and final reclamation of one shared input. / 该模式测量真实提交调用链、任务退休及最终回收，始终开启在线 GC。

`runtime_performance.py --phase gc --gc-writers 3 --gc-packs 8 --gc-rounds 3` measures actual collection of multiple packs sharing each writer directory. Fixture publication, sealing and release happen outside the timer; collection includes refresh, mark, unlink, barriers and receipts. / 该模式测量共享父目录的多文件回收；准备负载不计时，实际 collection 全流程计时，保留逐轮 wall／CPU 样本。
