# protocol: models and experiments / 模型与实验

[总入口 / Index](../README.md) · [三条协议主线 / Protocol](../../docs/DESIGN.md) · [审查映射 / Review map](../../docs/DESIGN.md#review)

规则以协议文档为准；本目录仅列可执行模型和实验。运行器保留每个 case 的配置、哈希与反例。

The protocol document defines the rules. This directory contains executable models and experiments; the runner records each case configuration, hashes and counterexample.

```sh
python verification/runners/check_tla.py --area protocol
```

## Models / 模型

- [CommitApplication](tla/CommitApplication.tla) · [config](tla/CommitApplication.cfg)
- [CommitCleanup](tla/CommitCleanup.tla) · [config](tla/CommitCleanup.cfg)
- [Completion](tla/Completion.tla) · [config](tla/Completion.cfg)
- [CrossLogComposition](tla/CrossLogComposition.tla) · [config](tla/CrossLogComposition.cfg)
- [DurableHandoff](tla/DurableHandoff.tla) · [config](tla/DurableHandoff.cfg)
- [Ownership](tla/Ownership.tla) · [config](tla/Ownership.cfg)
- [ProvisionalHold](tla/ProvisionalHold.tla) · [config](tla/ProvisionalHold.cfg)
- [PublicationHandoff](tla/PublicationHandoff.tla) · [config](tla/PublicationHandoff.cfg)
- [QueueRecordWorkflow](tla/QueueRecordWorkflow.tla) · [config](tla/QueueRecordWorkflow.cfg)
- [ReactivationRace](tla/ReactivationRace.tla) · [config](tla/ReactivationRace.cfg)
- [ReactivationScan](tla/ReactivationScan.tla) · [config](tla/ReactivationScan.cfg)
- [RequestReplay](tla/RequestReplay.tla) · [config](tla/RequestReplay.cfg)
- [SharedRecordWorkflow](tla/SharedRecordWorkflow.tla) · [config](tla/SharedRecordWorkflow.cfg)
- [StagingConsumption](tla/StagingConsumption.tla) · [config](tla/StagingConsumption.cfg)
- [TrainingResume](tla/TrainingResume.tla) · [config](tla/TrainingResume.cfg)

## Checks and probes / 检查与实验

- [ownership_model.py](checks/ownership_model.py)
- [commit_cleanup_probe.py](probes/commit_cleanup_probe.py)
- [consumption_probe.py](probes/consumption_probe.py)
- [handoff_probe.py](probes/handoff_probe.py)
- [lease_selection_probe.py](probes/lease_selection_probe.py)
- [provisional_hold_probe.py](probes/provisional_hold_probe.py)
- [reactivation_peer_probe.py](probes/reactivation_peer_probe.py)
- [handoff_extent_probe.rs](probes/handoff_extent_probe.rs)
- [queue_record_probe.rs](probes/queue_record_probe.rs)
- [reactivation_probe.rs](probes/reactivation_probe.rs)
- [workflow_extent_probe.rs](probes/workflow_extent_probe.rs)

环境前提与实现义务见[环境审查 / Environment review](../../docs/DESIGN.md#environment)。实验结果只支持实际测试的部署与故障类型；不保证任意文件系统或应用。
