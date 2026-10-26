# wal: models and experiments / 模型与实验

[总入口 / Index](../README.md) · [三条协议主线 / Protocol](../../docs/DESIGN.md) · [审查映射 / Review map](../../docs/DESIGN.md#review)

规则以协议文档为准；本目录仅列可执行模型和实验。运行器保留每个 case 的配置、哈希与反例。

The protocol document defines the rules. This directory contains executable models and experiments; the runner records each case configuration, hashes and counterexample.

```sh
python verification/runners/check_tla.py --area wal
```

## Models / 模型

- [CatalogReadBuffer](tla/CatalogReadBuffer.tla) · [config](tla/CatalogReadBuffer.cfg)
- [CatalogReplay](tla/CatalogReplay.tla) · [config](tla/CatalogReplay.cfg)
- [CatalogTransaction](tla/CatalogTransaction.tla) · [config](tla/CatalogTransaction.cfg)
- [WalRecovery](tla/WalRecovery.tla) · [config](tla/WalRecovery.cfg)

## Checks and probes / 检查与实验

- [crash_campaign.py](checks/crash_campaign.py)
- [catalog_history_probe.py](probes/catalog_history_probe.py)
- [catalog_replay_probe.py](probes/catalog_replay_probe.py)

环境前提与实现义务见[环境审查 / Environment review](../../docs/DESIGN.md#environment)。实验结果只支持实际测试的部署与故障类型；不保证任意文件系统或应用。
