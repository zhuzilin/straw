# 离线恢复 / Offline recovery

[总入口 / Index](../README.md)

所有客户端停止后的检查与孤儿清理，包括控制文件、依赖闭包和别名。 / Inspection and orphan cleanup with all clients stopped, including control files, dependency closures and aliases.

模型与配置在 `tla/`，部署实验与配套注入程序在 `probes/`。如有 `checks/` 或 `verus/`，分别放可执行检查和数学证明。生产 Rust 的证明合约仍随实现在仓库 `rust/` 中。

Models/configurations live in `tla/`, deployment experiments and their shims in `probes/`. Optional `checks/` and `verus/` contain executable checks and mathematical proofs. Production Rust proof contracts remain with their implementations in the repository `rust/` directory.

从仓库根目录运行 / Run from the repository root:

```sh
python verification/runners/check_tla.py --area offline
python verification/runners/environment_manifest.py --model OfflineAliases
```

不加 `--area` 运行完整模型集合。完整 Rust 证明共用 `verification/runners/check_verus.py`。 / Omit `--area` for the complete model suite. All Rust proofs use the shared Verus runner.

| Model | Configuration |
|---|---|
| [OfflineAliases](tla/OfflineAliases.tla) | [OfflineAliases.cfg](tla/OfflineAliases.cfg) |
| [OfflineCleanup](tla/OfflineCleanup.tla) | [OfflineCleanup.cfg](tla/OfflineCleanup.cfg) |
| [OfflineRoots](tla/OfflineRoots.tla) | [OfflineRoots.cfg](tla/OfflineRoots.cfg) |

证明边界和待补项见 [FORMAL_VERIFICATION](../../docs/FORMAL_VERIFICATION_zh.md)。 / See [formal verification](../../docs/FORMAL_VERIFICATION.md) for proof boundaries and outstanding work.
