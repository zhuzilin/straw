---
name: straw-protocol-review
description: Review or change straw's persistence, ownership, recovery, GC, publication and read protocols. Use for protocol review, TODO acceptance, TLA+ changes, or Rust changes affecting durable ordering, crash/retry, request/attempt/epoch admission, reader roots or authentication. Compare the human protocol document to TLA+, then TLA+ to actual implementation; identify discrepancies and validate fixes with relevant regressions.
---

# straw 协议审查

以仓库根目录为工作目录。先读 `docs/DESIGN_zh.md`
（英文版去掉 `_zh`），再按三部分中的“模型与实现对应”定位模型、调用链和测试。
文档是人审查的规则入口；映射表只是导航，不能替代阅读当前源码。

## 1. 确定规则与范围

从用户要求、diff 和 `TODO.md` 确定受影响的规则：

- **分布式协议 C1/C2/C4**：跨日志 handoff、请求/attempt/epoch、checkpoint 回退。
- **WAL 与数据 I/O C3/R1–R4**：日志与派生状态恢复、准备与发布重试、认证读取、独立可见性与有限重试。
- **所有权与 GC G1–G4**：完整根、认证与缓存、删除屏障与回执、writer 退休和条件活性。

追踪一条完整流程，包括入口、成功返回、报错但已生效、崩溃恢复和重复调用。
区分 accepted、applied、protected、reclaimed。禁止通过扩大环境假设或把失败步骤
合成原子动作来消除反例。没有格式变化就保留现有 wire format 和 API。

## 2. 文档 → TLA+

逐条检查对应 `.tla`、`.cfg` 和 `verification/runners/check_tla.py` 的实际配置：

1. 文档的状态、前置条件、持久化点、错误路径分别对应哪些变量和动作？
2. 锁、write、sync、apply、release、unlink、目录屏障、receipt 的可观察顺序是否保留？
3. 关键性质是否在运行配置中启用？正例能否成功，避免“一律拒绝”假安全？
4. 负例是否因为指定不变量/时序性质失败？解析错误、超时和工具错误不是有效反例。
5. 有限实例抽象了什么？跨日志模型的完整事务不能替代 WAL 的碎片写入模型。
6. 活性是否单列公平性、锁/I/O 返回、环境稳定和实际调用者继续重试？永久 owner
   与永久无效输入应有独立安全/拒绝场景，不能假设它们最终消失或成功。

审查 `verification/environment_contracts.json` 和 `docs/DESIGN_zh.md#environment`：
环境保证与 straw 调用者义务分别列出。尤其不能把“已认证闭包”“正确根集合”或
“正确 callback”当成外部保证，跳过实际构造与调用。

## 3. TLA+ → 实现

从公开入口向下读真实 Rust/Python/FFI 调用路径，不能只核对同名 helper。
对每个关键动作记录：实际函数、guard、数据身份、持久化效果、错误后可用状态和恢复入口。
同时检查返回值和后续调用者是否真的遵守 guard。

重点反查这些越界：

- queue WAL 确认前释放 staging/根；旧 handoff 消耗新一代 staging。
- 部分 apply、I/O 错误或 caught unwind 后仍允许写入、缓存回复或 GC；恢复用了另一条日志。
- stale attempt/epoch 被新请求接受；旧成功请求因数据已回收而不能重放。
- reader/checkpoint/其他队列/传递依赖根遗漏；锁外认证后没有锁内重查。
- 缓存跨 pool/run/namespace/reference 复用；既有可变 owner 名不保护所请求的根。
- unlink/NotFound 后目录屏障未完成就记回执；回执删除永久 tombstone。
- writer fence 未排空旧 I/O；未认证结果就恢复 staging；release 与登记 settlement 混成一步。
- warm index cache 跳过 frame/envelope/payload 校验；失败读取不关闭重开；永久损坏被当成功。

对发现的实现错误，先保留可失败的实际回归，再修复并跑正常路径。
修改模型和文档时同步中英文、规则 ID、映射和环境登记；不添加重复的协议正文。
普通确定性 helper 优先编译器、Clippy 和有意义的边界测试，不重建局部证明体系。

## 4. 验证与交付

按风险运行 `CONTRIBUTING.md#checks` 中的命令。模型改动至少运行对应 area、
关联组合模型和环境登记检查；完整版本验收运行全部保留模型、Rust/FFI 回归及 crash campaign。
运行 Python 测试前确认加载的 native 扩展由当前源码构建，避免测试旧 wheel。
只有新的修改、失败或未解决疑点才需要重复已通过的检查。

输出须能让人复查：

- 结论及受影响规则；文档→TLA、TLA→实现分别有哪些一致点和差异。
- 每个问题的具体触发/崩溃窗口、影响、源码位置、复现及修复证据。
- 实际运行命令、输入/工具版本或哈希、结果，以及未运行的部分。
- 有意保留的抽象差异、环境前提和剩余风险；没有发现问题时也报告这些边界。
- 删除验证时说明原来防止的错误、保留证据、减少的保证与维护成本；性能只报可比实测。

有限模型检查、源码审查和具体回归是三种不同证据。不能把 CI 全绿写成完整实现等价证明，
也不能仅删除 TODO 或改标记来宣布完成。只有对应交付物与验收结果齐全才勾选。
