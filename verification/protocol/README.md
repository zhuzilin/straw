# 分布式协议 / Distributed protocol

[总入口 / Index](../README.md)

所有权交接、提交与重试、lease、队列和共享数据工作流。 / Ownership transfer, commit/retry, leases and queue/shared-data workflows.

模型与配置在 `tla/`，部署实验与配套注入程序在 `probes/`。如有 `checks/` 或 `verus/`，分别放可执行检查和数学证明。生产 Rust 的证明合约仍随实现在仓库 `rust/` 中。

Models/configurations live in `tla/`, deployment experiments and their shims in `probes/`. Optional `checks/` and `verus/` contain executable checks and mathematical proofs. Production Rust proof contracts remain with their implementations in the repository `rust/` directory.

从仓库根目录运行 / Run from the repository root:

```sh
python verification/runners/check_tla.py --area protocol
python verification/runners/environment_manifest.py --model CommitCleanup
```

不加 `--area` 运行完整模型集合。完整 Rust 证明共用 `verification/runners/check_verus.py`。 / Omit `--area` for the complete model suite. All Rust proofs use the shared Verus runner.

[控制器重试证明 / Controller retry proof](verus/retry_liveness.rs) derives completion from explicit environment and caller obligations.

| Model | Configuration |
|---|---|
| [CrossLogComposition](tla/CrossLogComposition.tla) | [CrossLogComposition.cfg](tla/CrossLogComposition.cfg) |
| [RequestReplay](tla/RequestReplay.tla) | [RequestReplay.cfg](tla/RequestReplay.cfg) |
| [CommitApplication](tla/CommitApplication.tla) | [CommitApplication.cfg](tla/CommitApplication.cfg) |
| [TrainingResume](tla/TrainingResume.tla) | [TrainingResume.cfg](tla/TrainingResume.cfg) |
| [CommitCleanup](tla/CommitCleanup.tla) | [CommitCleanup.cfg](tla/CommitCleanup.cfg) |
| [Completion](tla/Completion.tla) | [Completion.cfg](tla/Completion.cfg) |
| [DurableHandoff](tla/DurableHandoff.tla) | [DurableHandoff.cfg](tla/DurableHandoff.cfg) |
| [LeaseSelection](tla/LeaseSelection.tla) | [LeaseSelection.cfg](tla/LeaseSelection.cfg) |
| [Ownership](tla/Ownership.tla) | [Ownership.cfg](tla/Ownership.cfg) |
| [ProvisionalHold](tla/ProvisionalHold.tla) | [ProvisionalHold.cfg](tla/ProvisionalHold.cfg) |
| [PublicationHandoff](tla/PublicationHandoff.tla) | [PublicationHandoff.cfg](tla/PublicationHandoff.cfg) |
| [QueueRecordWorkflow](tla/QueueRecordWorkflow.tla) | [QueueRecordWorkflow.cfg](tla/QueueRecordWorkflow.cfg) |
| [ReactivationRace](tla/ReactivationRace.tla) | [ReactivationRace.cfg](tla/ReactivationRace.cfg) |
| [ReactivationScan](tla/ReactivationScan.tla) | [ReactivationScan.cfg](tla/ReactivationScan.cfg) |
| [SharedRecordWorkflow](tla/SharedRecordWorkflow.tla) | [SharedRecordWorkflow.cfg](tla/SharedRecordWorkflow.cfg) |
| [StagingConsumption](tla/StagingConsumption.tla) | [StagingConsumption.cfg](tla/StagingConsumption.cfg) |

证明边界和待补项见 [FORMAL_VERIFICATION](../../docs/FORMAL_VERIFICATION_zh.md)。 / See [formal verification](../../docs/FORMAL_VERIFICATION.md) for proof boundaries and outstanding work.

`protocol::execute_commit` 验证实际四步提交分派与 apply 隔离，原生 effect 语义另行证明。`CommitApplication` 覆盖捕获 panic 后的恢复／GC，`TrainingResume` 覆盖模型 checkpoint 与队列消费位置及 replay owner 的应用协议；后者不是自动续训实现。 / Actual commit dispatch is verified under lower-effect contracts; interrupted application and application-managed training recovery are checked separately.

实际请求回执查询／更新与任意有限 admitted 历史见 [request.rs](../../rust/request.rs) 和 [request_history](verus/request_history.rs)。`RequestReplay` 单独覆盖数据回收后的重试；完整 Coordinator admission／key／hash／WAL 组合仍未证明。 / Actual receipt lookup/update and finite-history induction are verified separately from the finite request/GC model; full coordinator and durability composition remains open.

实际任务目标解析见 [queue_event.rs](../../rust/queue_event.rs)，同一 JSON／WAL 恢复后的完整有序目标对应见 [queue_event_equivalence.rs](verus/queue_event_equivalence.rs)；完整任务状态、缓存及 stale attempt／epoch 准入仍未完成。[证据 / Evidence](../measurements/20261025-queue-event.json)。 / Actual target parsing is verified and connected to ordered WAL recovery; complete task-state/cache/admission refinement remains open.

实际 consumer／batch／checkpoint／retention 及末尾回执更新见 [queue_metadata.rs](../../rust/queue_metadata.rs)，嵌套路径更新与错误无副作用见 [json_update.rs](../../rust/json_update.rs)。完整 coordinator 和 GC 组合仍未闭合。 / Actual metadata edits and receipts are verified with exact partial-error prefixes; full coordinator/GC composition remains open.

[多队列／多请求归纳证明 / Multi-queue/request induction](verus/cross_log.rs) 组合共享根、恢复、持久回执、重新 staging 与 GC，包含可达历史见证。模型以独立持久效果为边界，原生调用链、WAL 碎片细化和活性仍单列；[环境](tla/EnvironmentCrossComposition.tla) 可独立审查。 / The proof covers arbitrary finite histories at explicit durable-effect boundaries, not complete native refinement.

实际 [catalog_transaction](../../rust/catalog_transaction.rs) 验证入口绑定、持锁 fresh 身份检查、恢复后回调、末尾 context 校验和退出错误优先级；原生锁／refresh／callback 对应仍单列。 / The production transaction dispatcher is verified under explicit primitive/context/callback contracts; see the [exact scope](../../docs/FORMAL_VERIFICATION.md#actual-catalog-transaction-entry-and-exit).

实际任务记录的创建、租约、失败、终态与续采样字段更新见 [queue_task.rs](../../rust/queue_task.rs)；完整事件分派、缓存和准入仍是未闭合边界。[证据 / Evidence](../measurements/20261025-queue-task.json)。 / Actual task-record updates and local error preservation are verified; full dispatch/cache/admission refinement remains open.

实际 reader 根登记／追加／有序释放见 [queue_reader.rs](../../rust/queue_reader.rs)，完成回执及 producer continuation 更新见 [queue_receipt.rs](../../rust/queue_receipt.rs)。[证据 / Evidence](../measurements/20261025-queue-reader.json)。 / Actual reader-root preservation, ordered release/error prefixes, submission-before-commit and producer updates are proved; worker-retirement filtering is now proved; full native admission, history and root/ownership correspondence remain open.

[queue_state.rs](../../rust/queue_state.rs) 将实际任务更新绑定到 queue 中的目标，证明其他根／任务保持及 epoch 批次的精确有序错误前缀。[证据 / Evidence](../measurements/20261025-queue-state.json)。 / Actual task-table selection, leaf dispatch and uniform-change batch traces are verified; outer event selection and full mixed history remain open.

[queue_transition.rs](../../rust/queue_transition.rs) 验证任务／reader／回执的实际顺序及错误前缀；[queue_transition_roots.rs](verus/queue_transition_roots.rs) 从实际 progress 合约推导旧根／新输入和 peer reader 保持，显式依赖 String 内容身份。[证据 / Evidence](../measurements/20261026-queue-transition.json)。 / Ordered compound replay and derived continuation-root preservation; full event admission, history and catalog ownership remain open.

[queue_admin.rs](../../rust/queue_admin.rs) 验证有界排队序号、有序提交／producer continuation、epoch／撤销及 reader／回执根绑定。[证据 / Evidence](../measurements/20261026-queue-admin.json)。 / Bounded order allocation and administrative replay preserve exact completed prefixes. Full dispatch/history, caches and ownership remain open.

[queue_replay.rs](../../rust/queue_replay.rs) 验证实际事件分派、完整租约／失败批次错误前缀、sealed 和回执收尾；worker 退休保留其他 reader 的全部根。[证据 / Evidence](../measurements/20261026-queue-replay.json)。 / Actual business dispatch and reader filtering are proved; preparation and the business/WAL history fold are now composed by queue_history; native cache semantics and ownership remain open.

[queue_history.rs](../../rust/queue_history.rs) 验证实际初始化、目标准入、事件／事务循环和精确错误前缀；`State::apply_business` 绑定 journal 解码结果，`physical_prefix_business` 连接字节前缀与完整业务 fold。缓存／消费记账内容和原生上下文仍单列边界。[证据 / Evidence](../measurements/20261026-queue-history.json)。

[queue_application.rs](../../rust/queue_application.rs) 验证实际隔离、回调结果顺序、成功确认、入口拒绝及全新恢复结果；`ENV-QUEUE-APPLICATION-HOOKS` 将回调接口和原生适配义务分开。[证据 / Evidence](../measurements/20261026-queue-application.json)。
