# storage: models and experiments / 模型与实验

[总入口 / Index](../README.md) · [三条协议主线 / Protocol](../../docs/DESIGN.md) · [审查映射 / Review map](../../docs/DESIGN.md#review)

规则以协议文档为准；本目录仅列可执行模型和实验。运行器保留每个 case 的配置、哈希与反例。

The protocol document defines the rules. This directory contains executable models and experiments; the runner records each case configuration, hashes and counterexample.

```sh
python verification/runners/check_tla.py --area storage
```

## Models / 模型

- [DependencyTraversal](tla/DependencyTraversal.tla) · [config](tla/DependencyTraversal.cfg)
- [DirectoryDurability](tla/DirectoryDurability.tla) · [config](tla/DirectoryDurability.cfg)
- [ExtentVisibility](tla/ExtentVisibility.tla) · [config](tla/ExtentVisibility.cfg)
- [IncompletePublication](tla/IncompletePublication.tla) · [config](tla/IncompletePublication.cfg)
- [NamespaceIdentity](tla/NamespaceIdentity.tla) · [config](tla/NamespaceIdentity.cfg)
- [PendingPublication](tla/PendingPublication.tla) · [config](tla/PendingPublication.cfg)
- [PublicationFinalization](tla/PublicationFinalization.tla) · [config](tla/PublicationFinalization.cfg)
- [PublicationPreparation](tla/PublicationPreparation.tla) · [config](tla/PublicationPreparation.cfg)
- [PublicationRetry](tla/PublicationRetry.tla) · [config](tla/PublicationRetry.cfg)
- [RangeRead](tla/RangeRead.tla) · [config](tla/RangeRead.cfg)
- [ReadAttempt](tla/ReadAttempt.tla) · [config](tla/ReadAttempt.cfg)
- [ReadLookup](tla/ReadLookup.tla) · [config](tla/ReadLookup.cfg)
- [TypedRead](tla/TypedRead.tla) · [config](tla/TypedRead.cfg)
- [WriterAccess](tla/WriterAccess.tla) · [config](tla/WriterAccess.cfg)

## Checks and probes / 检查与实验

- [catalog_encoding_probe.py](probes/catalog_encoding_probe.py)
- [dependency_probe.py](probes/dependency_probe.py)
- [directory_sync_probe.py](probes/directory_sync_probe.py)
- [extent_visibility_probe.py](probes/extent_visibility_probe.py)
- [filesystem_bandwidth_probe.py](probes/filesystem_bandwidth_probe.py)
- [incomplete_publication_probe.py](probes/incomplete_publication_probe.py)
- [path_identity_probe.py](probes/path_identity_probe.py)
- [pending_publication_probe.py](probes/pending_publication_probe.py)
- [preparation_probe.py](probes/preparation_probe.py)
- [publication_finalize_probe.py](probes/publication_finalize_probe.py)
- [publication_retry_probe.py](probes/publication_retry_probe.py)
- [read_latency_probe.py](probes/read_latency_probe.py)
- [read_lookup_probe.py](probes/read_lookup_probe.py)
- [tensor_granularity_probe.py](probes/tensor_granularity_probe.py)
- [typed_preflight_probe.py](probes/typed_preflight_probe.py)
- [writer_access_probe.py](probes/writer_access_probe.py)
- [writer_lifetime_probe.py](probes/writer_lifetime_probe.py)
- [large_extent_probe.rs](probes/large_extent_probe.rs)
- [positioned_extent_probe.rs](probes/positioned_extent_probe.rs)
- [standalone_extent_probe.rs](probes/standalone_extent_probe.rs)

环境前提与实现义务见[环境审查 / Environment review](../../docs/DESIGN.md#environment)。实验结果只支持实际测试的部署与故障类型；不保证任意文件系统或应用。
