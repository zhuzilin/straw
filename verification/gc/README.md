# gc: models and experiments / 模型与实验

[总入口 / Index](../README.md) · [三条协议主线 / Protocol](../../docs/DESIGN.md) · [审查映射 / Review map](../../docs/DESIGN.md#review)

规则以协议文档为准；本目录仅列可执行模型和实验。运行器保留每个 case 的配置、哈希与反例。

The protocol document defines the rules. This directory contains executable models and experiments; the runner records each case configuration, hashes and counterexample.

```sh
python verification/runners/check_tla.py --area gc
```

## Models / 模型

- [GCBatchDeletion](tla/GCBatchDeletion.tla) · [config](tla/GCBatchDeletion.cfg)
- [GCDeletion](tla/GCDeletion.tla) · [config](tla/GCDeletion.cfg)
- [GCDiscovery](tla/GCDiscovery.tla) · [config](tla/GCDiscovery.cfg)
- [GCParentRepair](tla/GCParentRepair.tla) · [config](tla/GCParentRepair.cfg)
- [GCReceipt](tla/GCReceipt.tla) · [config](tla/GCReceipt.cfg)
- [WriterLifetime](tla/WriterLifetime.tla) · [config](tla/WriterLifetime.cfg)
- [WriterRetirement](tla/WriterRetirement.tla) · [config](tla/WriterRetirement.cfg)

## Checks and probes / 检查与实验

- [catalog_discovery_probe.py](probes/catalog_discovery_probe.py)
- [gc_cross_client.py](probes/gc_cross_client.py)
- [gc_history_probe.py](probes/gc_history_probe.py)
- [gc_parent_probe.py](probes/gc_parent_probe.py)
- [gc_receipt_probe.py](probes/gc_receipt_probe.py)
- [gc_scale.py](probes/gc_scale.py)
- [gc_stat_probe.py](probes/gc_stat_probe.py)

环境前提与实现义务见[环境审查 / Environment review](../../docs/DESIGN.md#environment)。实验结果只支持实际测试的部署与故障类型；不保证任意文件系统或应用。
