# 数据存储 / Data storage

[总入口 / Index](../README.md)

发布与读取、可见性、路径／编码、tensor 及大数据带宽测量。 / Publication and reads, visibility, paths/encoding, tensors and large-data throughput measurements.

模型与配置在 `tla/`，部署实验与配套注入程序在 `probes/`。如有 `checks/` 或 `verus/`，分别放可执行检查和数学证明。生产 Rust 的证明合约仍随实现在仓库 `rust/` 中。

Models/configurations live in `tla/`, deployment experiments and their shims in `probes/`. Optional `checks/` and `verus/` contain executable checks and mathematical proofs. Production Rust proof contracts remain with their implementations in the repository `rust/` directory.

从仓库根目录运行 / Run from the repository root:

```sh
python verification/runners/check_tla.py --area storage
python verification/runners/environment_manifest.py --model DependencyTraversal
```

不加 `--area` 运行完整模型集合。完整 Rust 证明共用 `verification/runners/check_verus.py`。 / Omit `--area` for the complete model suite. All Rust proofs use the shared Verus runner.

| Model | Configuration |
|---|---|
| [DependencyTraversal](tla/DependencyTraversal.tla) | [DependencyTraversal.cfg](tla/DependencyTraversal.cfg) |
| [DirectoryDurability](tla/DirectoryDurability.tla) | [DirectoryDurability.cfg](tla/DirectoryDurability.cfg) |
| [ExtentArithmetic](tla/ExtentArithmetic.tla) | [ExtentArithmetic.cfg](tla/ExtentArithmetic.cfg) |
| [ExtentVisibility](tla/ExtentVisibility.tla) | [ExtentVisibility.cfg](tla/ExtentVisibility.cfg) |
| [IncompletePublication](tla/IncompletePublication.tla) | [IncompletePublication.cfg](tla/IncompletePublication.cfg) |
| [NamespaceEncoding](tla/NamespaceEncoding.tla) | [NamespaceEncoding.cfg](tla/NamespaceEncoding.cfg) |
| [NamespaceIdentity](tla/NamespaceIdentity.tla) | [NamespaceIdentity.cfg](tla/NamespaceIdentity.cfg) |
| [PayloadStream](tla/PayloadStream.tla) | [PayloadStream.cfg](tla/PayloadStream.cfg) |
| [PendingPublication](tla/PendingPublication.tla) | [PendingPublication.cfg](tla/PendingPublication.cfg) |
| [PublicationFinalization](tla/PublicationFinalization.tla) | [PublicationFinalization.cfg](tla/PublicationFinalization.cfg) |
| [PublicationPreparation](tla/PublicationPreparation.tla) | [PublicationPreparation.cfg](tla/PublicationPreparation.cfg) |
| [PublicationRetry](tla/PublicationRetry.tla) | [PublicationRetry.cfg](tla/PublicationRetry.cfg) |
| [RangeRead](tla/RangeRead.tla) | [RangeRead.cfg](tla/RangeRead.cfg) |
| [ReadAttempt](tla/ReadAttempt.tla) | [ReadAttempt.cfg](tla/ReadAttempt.cfg) |
| [ReadLookup](tla/ReadLookup.tla) | [ReadLookup.cfg](tla/ReadLookup.cfg) |
| [TensorGranularity](tla/TensorGranularity.tla) | [TensorGranularity.cfg](tla/TensorGranularity.cfg) |
| [TypedRead](tla/TypedRead.tla) | [TypedRead.cfg](tla/TypedRead.cfg) |
| [WriterAccess](tla/WriterAccess.tla) | [WriterAccess.cfg](tla/WriterAccess.cfg) |

证明边界和待补项见 [FORMAL_VERIFICATION](../../docs/FORMAL_VERIFICATION_zh.md)。 / See [formal verification](../../docs/FORMAL_VERIFICATION.md) for proof boundaries and outstanding work.

实际 [closure::collect/resolve](../../rust/closure.rs) 已证明完整可达 pack、缓存保持、错误不写部分缓存及传递 tombstone 拒绝；原生逐节点 Reader／摘要／namespace 与缓存上下文仍有适配义务。[确切证据](../measurements/20261025-dependency-closure.json)。

Production closure traversal and tombstone checking now prove exact reachability and cache preservation under a finite immutable per-node source. Native source/context and ownership/GC refinement remain open; the linked record separates proof and runtime evidence.

实际 [closure_node](../../rust/closure_node.rs) 进一步证明字段优先级、数组校验、完整有序后继和摘要／引用绑定；图上下文、原生认证读取／摘要／路径适配仍未闭合。[逐节点证据](../measurements/20261025-closure-node.json)。 / Actual node decoding proves field selection and complete ordered child binding under Digest and Value contracts; native read/hash/namespace and graph-context refinement remain open.
