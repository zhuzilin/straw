# 协议依据与正确性验证

[English](PROTOCOL_AND_VERIFICATION.md)

状态：实验性基础设施，尚未获得生产环境持久性认证。Rust 实现、有限协议模型和真实文件系统测试提供不同层次的证据，任何单一层次都不能证明整个系统。本文说明当前实现及仍需验证的部分。

## 参考工作与 straw 的取舍

| 领域 | 一手参考 | 借鉴内容与边界 |
|---|---|---|
| 恢复顺序 | [Mohan 等，ARIES，TODS 1992](https://research.ibm.com/publications/aries-a-transaction-recovery-method-supporting-fine-granularity-locking-and-partial-rollbacks-using-write-ahead-logging) | 用持久化日志作为权威事实，明确定义恢复。straw 使用不可变数据与元数据事务 redo，**没有**实现 ARIES 的 pageLSN、undo 或补偿记录。 |
| 打包不可变数据 | [Rosenblum 与 Ousterhout，The Design and Implementation of a Log-Structured File System，TOCS 1992](https://www.cs.cmu.edu/afs/cs/academic/class/15712-f08/www/readings/Rosenblum92.pdf) | 多个逻辑对象追加到 segment，并按 segment 回收。目前只删除全部失去引用的 pack，不搬迁存活记录。 |
| 共享引用 | [Birrell 与 Wobber，Distributed Garbage Collection for Network Objects，1993](https://www.microsoft.com/en-us/research/publication/distributed-garbage-collection-for-network-objects/) | 显式描述持有者，在释放源引用前完成引用转交。straw 用共享 catalog 锁下的持久化 owner 集合，没有实现论文中的分布式回收算法。 |
| 并发 reader | [Michael，Hazard Pointers，TPDS 2004](https://research.ibm.com/publications/hazard-pointers-safe-memory-reclamation-for-lock-free-objects) | 对应的义务是在回收前保护真实 reader。工作 lease 过期不能证明读取停止；内存 hazard pointer 本身也不能解决进程崩溃后的持久化所有权。 |
| 可观察一致性 | [Herlihy 与 Wing，Linearizability，TOPLAS 1990](https://www.cs.cmu.edu/~wing/publications/HerlihyWing90.pdf) | 为操作定义原子观察点，尊重真实时间顺序。响应丢失的操作可能已提交，重试需保留其身份。 |
| 协议模型 | [Newcombe 等，How AWS Uses Formal Methods，CACM 2015](https://www.amazon.science/publications/how-amazon-web-services-uses-formal-methods) | 在信任协议前，穷举包括恢复在内的小规模并发模型。模型有效性与实现符合模型，仍是两项独立义务。 |
| 实现仿真 | [Zhou 等，FoundationDB，SIGMOD 2021](https://www.foundationdb.org/files/fdb-paper.pdf) | 确定性调度与故障仿真使困难故障可复现。straw 尚无 FoundationDB 式的完整实现仿真。 |
| 文件系统崩溃行为 | [Pillai 等，OSDI 2014](https://www.usenix.org/conference/osdi14/technical-sessions/presentation/pillai)、[Mohan 等，CrashMonkey/Ace，OSDI 2018](https://www.usenix.org/conference/osdi18/presentation/mohan) | 进程崩溃测试通过不能证明掉电安全，需要针对实际文件系统约定验证持久化顺序和小型穷举崩溃负载。 |

这些是设计和验证参考，不代表组合熟悉的技术就自动继承了相应证明。当前实现并非由某一篇论文形式化推导而来。

## 范围与假设

- 每个队列只有一个由外部 fencing 保证唯一的协调器。`exclusive_owner` 记录外部保证，非空字符串本身不执行 fencing。未实现自动选主、共识或容忍网络分区的故障切换。
- 多个队列可通过 `Coordinator(..., queue_id=..., namespace=True)` 共享不可变存储池/run，各有独立 journal，池共用一个保存持久化 owner 集合与 pack 状态的 `storage.log`。从创建池时启用 GC，尚不支持迁移已有的未跟踪队列。队列/staging owner 名称属于内部协议，应用不能直接覆盖或释放。
- Catalog 用跨客户端 advisory file lock 串行化元数据修改与回收，全部 writer/collector 都必须参与。同一 inode 不重命名、不替换。持锁后重新打开日志执行 I/O，不假设旧句柄会自动消除远端缓存影响。
- 支持 Linux/POSIX。文件 fsync 使之前的字节持久化并可见，目录 fsync 使新目录项持久化；底层服务必须真正实现这些保证。错误会中止操作，但 sync 失败不意味着没有字节被持久化。
- JuiceFS 部署需要在实际客户端/服务上验证分布式锁、fsync 和 close-to-open 行为，并禁用 writeback/open-cache。见 [JuiceFS 缓存约定](https://juicefs.com/docs/community/guide/cache/)。一次四客户端负载成功，只能证明该次运行，不能认证存储服务掉电安全。
- 载荷与引用不可变，pack 路径使用新 UUID，不复用已回收路径，不原地覆盖张量。COW 目前复制整个**被修改张量**，不是 chunk/page 级 COW。
- Manifest 依赖图描述可达性。序列化后的引用字节本身不是生命周期所有者。Reader 必须由任务、未完成 batch、显式 `pin`、其他队列或保留的 checkpoint 保护。

客户端可能仍缓存另一个客户端已删除 pack 的属性。持久化 tombstone 之后，`unlink` 返回 `ENOENT` 视为幂等成功，不再重复计算回收文件/字节。其他 unlink 错误仍必须报错。`verification/gc_cross_client.py` 在 JuiceFS 上复现过实际的 stat 成功后 `unlink(ENOENT)`。确定性故障测试也覆盖这一边界，并确认无关 unlink 错误会向上传播。

## 持久化顺序与确认

Writer 在创建/追加 pack 字节**之前**，先登记 open pack。发布带外部依赖的 manifest 前，先以操作 owner 持久化保留这些依赖；然后写入并 sync 载荷，持久化返回 publication 所对应的传递性 pack 集合的 staging 所有权，最后释放操作 owner。轮换/关闭会封存 pack。不能根据年龄或超时把不确定/崩溃 writer 的 pack 当成已封存。

队列接收顺序：

```
retain destination queue roots in storage catalog; fsync
append queue transaction; fsync
apply queue state
consume staging ownership
reply
```

队列 journal 的持久化事务是结果被接收的时刻。Catalog 与队列日志**不是分布式原子事务**。该顺序让中断转交倾向于多保留数据，而非让已接收引用失去保护。恢复重放队列日志，之后 GC 按恢复状态裁剪该队列的根。缺少可信完成信号的 in-progress/staging 泄漏会被保守保留。

GC 使用此前为不可变队列根持久化的传递性 pack 集合，检查每个存活根都有有效 catalog 所有权，再裁剪失效 owner 并回收 pack。不会持有协调器/catalog 锁重新打开每个存活载荷图；GC 不是完整数据完整性扫描。所有权缺失会在裁剪/unlink 前报错。这依赖包括恢复路径在内的 retain-before-WAL 顺序，并避免冷缓存元数据扫描阻塞 lease 和 continuation 写入。

持有 lease 的任务保留输入及已发布 continuation 版本，直到 completion、显式 failure 或 yield 确认本次尝试读取结束。取消、超时和协调器 epoch 变化只撤销授权，仍保留旧 reader 根。`release_task_reads([lease])` 可确认实际读取完成，包括过期 lease；只有宿主系统确认 worker 已停止，才能调用 `retire_worker`。`outstanding_reads()` 向监督程序暴露这些 lease，以便明确确认旧任务已停止，而不是根据时间判断死亡。

丢弃的张量结果也必须释放 publication staging。`release_tensor_publications` 释放完整的源 publication，包括同次发布的其他张量；已接管的队列/checkpoint owner 独立保留。应用可以先释放被拒输出的 staging，再结束任务对已接管 continuation 的所有权。

AI 训练消费者用 `processed_cursor` 加稀疏 processed 位置确认原始结果，用 `finished_batches` 确认**所有训练 rank** 已完成 batch 读取。Pending/未确认 batch 保留 plan 和 output 依赖。额外 reader 需要显式根，游标不代表任意历史载荷都会保留。

显式保存 training consumer 可以回退 processed 前缀、稀疏位置或 finished batches。Native 在提交队列 WAL 之前，为重新变为存活状态、但独立 owner 已被裁剪的 receipt/batch 重新登记引用。持有 catalog 锁时必须校验完整可达数据，且每个 pack 仍由 checkpoint 等持久 source owner 保护；缺失、损坏、已回收或失去保护的数据会在 consumer WAL 改变前拒绝恢复。已有存活 owner 直接复用，不重新扫描载荷。重新开放的重放范围与 consumer 状态一起写入日志。GC 不自行修复 owner。

Checkpoint 是独立 owner。发布外部 checkpoint 指针前先保存根；覆盖时，在 owner ID 中使用独立的物理引用版本，避免新版本持久化之前旧版本失去保护。应用 checkpoint manifest 记录该 owner，只有显式丢弃该版本时才释放。被替换版本和崩溃 staging writer 可能保守泄漏。模型/优化器/队列联合 checkpoint 的最终提交，仍需单独端到端验证。

## 回收与不变量

回收先将已确认消费的历史重放保留终止写入日志。然后根据存活任务、已发放 reader、未处理结果、当前消费者、未完成 batch 和 checkpoint，仅协调**自己的队列**根；其他队列和显式应用 owner 保持独立。

持有共享 catalog 锁时执行：

```
candidates = sealed packs - union(all owners' transitive pack closures)
append durable tombstones for candidates; fsync
unlink candidates; fsync parent directories
```

Retain 拒绝 tombstone 路径。Collector 在标记后、unlink 前死亡，后续 GC 会重试 unlink。任何存活 extent 都会保留整个 pack。不会根据目录扫描猜测未登记 pack 已死亡。离线 orphan 检查器拒绝共享 catalog/多队列根。

核心安全义务：

1. 每个已经确认、尚未释放的引用，同时拥有持久化载荷与持久化可达性保护，包括所有传递性张量依赖。
2. 释放一个 owner 不会使其他队列、活跃 reader 或保留 checkpoint 失效。工作 lease 过期不等于读完。
3. Open 或可达 pack 不能被 tombstone。持久化 tombstone 之前不能 unlink，也不能通过新 owner 使其路径复活。
4. 完整但损坏的日志帧必须报错，不能被当作不完整尾部。仅有效持久前缀加最后一个未完整追加的尾部可恢复。Header 长度/sequence、body/trailer 均被 checksum 覆盖。
5. 不确定响应通过操作身份确定结果。恢复保留已接收事实并撤销过期授权，但不保证每次物理计算或优化器更新只执行一次。

安全优先于回收进展。永久丢失的 writer/pin 可能无限期保留存储。目前没有 TTL 回收、存活记录整理或有界元数据/日志历史。

## 可执行证据与后续工作

| 层次 | 当前提供 | 尚不能证明 |
|---|---|---|
| 有限协议模型 | `verification/ownership_model.py`：两个队列、一个不可变 pack、一个 reader、一个 checkpoint、一次崩溃的固定点探索；五种不安全修改（包括回退游标却未重新登记引用）必须产生反例。 | 不是 TLA+/TLAPS、无界或 Rust refinement 证明。不含多 pack 依赖环、任意重试、文件系统缓存模型或活性证明。 |
| Native 日志故障注入 | `verification/crash_campaign.py`：两种 WAL 中新帧的每字节前缀、每字节一次 XOR 修改；catalog/队列 WAL 边界子进程突然退出后恢复/GC。 | 前缀持久化只是一种存储故障模型。进程退出后内核仍存活，不模拟设备扇区撕裂、块持久化重排或服务器故障。 |
| 回归/集成 | Rust 测试、驱动 native 核心的 Python 测试、可执行记录/张量/队列示例。 | 有限示例不能证明完备性。 |
| 四客户端存储负载 | `python -m straw.benchmark run --online-gc ...`：并发读写/回收，reader 确认后测量实际回收。 | 新路径不代表冷缓存，系统调用/应用载荷速率不等于后端设备吞吐。 |
| 属性缓存与根规模检查 | 在不同文件系统客户端运行 `verification/gc_cross_client.py` 和 `verification/gc_scale.py`。 | 有界运行不能认证任意挂载配置、服务故障或无界历史。 |

训练/恢复测试属于使用 straw 的应用。它们提供有用的集成证据，但不是核心正确性证明。详见[应用边界](APPLICATIONS_zh.md)与[验证命令](VERIFICATION_zh.md)。

复现成本较低的检查：

```bash
cargo test
cargo clippy --all-targets --features python -- -D warnings
python -m pytest
python verification/ownership_model.py --output /tmp/straw-model.json
python verification/crash_campaign.py --output /tmp/straw-crashes.json
```

Native 故障注入还接受 `--hosts <four hosts> --parent <shared mount>`，将交接用例分配到四个独立客户端。私有临时 root 自动删除，调用方把输出报告放在其外。

进程 fork 后创建新的 store/coordinator；继承可变 writer、mutex 和文件锁句柄后继续操作不在支持范围内。

模型与 native 故障注入已接入 CI。本地执行不代表托管 CI 矩阵已经运行。

在宣称生产可用之前，应继续：

- 扩大模型，包含多 pack/依赖、重试身份、协调器 epoch 和 checkpoint 替换；把公平性条件下的回收进展与安全分开检查。经过评审的 TLA+/PlusCal 规范与 TLC 运行可使模型更容易独立审查。
- 在 Rust 核心引入小范围文件系统/时钟/调度器接口，以独立顺序参考模型为基准进行带 seed 的确定性仿真，重放失败 seed 并缩减为短历史。
- 记录真实客户端请求/响应，按队列规范检查包含未完成操作的历史。[Knossos](https://github.com/jepsen-io/knossos) 可检查给定的线性化对象模型，但不会自动推断队列应该提供的约定。
- 在隔离认证环境中中断客户端/存储服务、延迟或丢失 RPC 响应、注入 ENOSPC/EIO/short write，并测试 VM/设备掉电。不要给共享训练集群断电做实验。
- 长时间运行，统计泄漏/orphan、反复保留/释放 checkpoint，验证恢复时间有界。在启用自动 failover 或 pack 整理前，独立评审协议及文件系统假设。

若未来要求高可用元数据或严格跨队列事务，应评估成熟事务/共识元数据服务，不要悄悄把单所有者日志扩展成新的共识实现。将嵌入式数据库放到 JuiceFS 上，也需要认证文件系统锁与崩溃持久性，不能自动修复正确性问题。
