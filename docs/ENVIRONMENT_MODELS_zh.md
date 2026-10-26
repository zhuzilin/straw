# 环境模型与假设审查

[English](ENVIRONMENT_MODELS.md) · [验证分层](FORMAL_VERIFICATION_zh.md)

环境模型描述 **straw 发出操作后，外部允许怎样响应**：哪些观察、返回值、部分副作用和其他参与者的交错是可能的。它与算法选择什么操作、希望证明什么安全／进展性质分开审查。模型可以数学上自洽，但与部署行为不符。

可审查的清单是 [`verification/environment_contracts.json`](../verification/environment_contracts.json)。每条契约有稳定的 `ENV-*` 编号、层级、状态、源码和证据路径。**显式模型**枚举行为；**假设**声明所需契约；**直接而未推导的前提**指出尚未补齐的证明；**仅测量**提供观察。这些状态都不是部署认证，缺失的模型也在清单中单独列出。

清单版本 2 为每条契约单列 `review.provided_by`（由谁提供保证）、`review.implementation_obligation`（straw 适配层必须做什么）和 `review.unproved`（尚未证明的对应关系）。例如，sync 成功保证持久化属于环境契约；**在调用 sync 前保存 pending 文件描述属于算法义务**，不能写成环境会替 straw 保证的前提。上述字段用于独立审查，并不新增证明。

实际调用者义务与外部保证的逐项对应见[实现／证明对应表](VERIFICATION_BOUNDARIES_zh.md)。`ENV-CROSS-LOG` 的全局回收另要求应用持有者结束并释放；永久 reader 应当保持保护，只有永久无主对象才适用条件回收。`ENV-AUTH` 的 WAL 调用者必须对正确的 header／payload 范围计算 digest；字节比较函数的证明不建立该调用者义务。

`ENV-JSON-LIBRARY` 单列二十一个 serde_json 方法契约，包括访问／克隆／映射／索引／数字／迭代／相等、解析与序列化；`ENV-JSON-PARSE` 独立提供成功反序列化关系，不承诺有效输入必定接受。实际 catalog 原始解码检查 run，并把字段／操作含义带入 Target 历史。实际 append 序列化／历史绑定在 ENV-JSON-ENCODE 下验证；原生命名空间／集合／缓存仍是单列的 straw 义务。

实际 queue 元数据编辑新增 Map `get_mut`／`remove` 契约：可变查询只借出已有值，值未改变则 map 未改变，删除只影响指定 key；Clone 保持规范内容及各有限深度观察。这些是外部库前提；按序元数据更新是已验证的 straw 代码。原生分派、错误隔离及完整 coordinator／GC 历史对应仍是单独义务。

`ENV-CATALOG-RESOLVER`、`ENV-SET-BUILDER`、`ENV-COLLECTION-ITERATOR` 分开命名空间身份、准确批量集合构造和 HashMap 消费迭代。实际字段到操作的构造已在这些契约下证明；原生路径／初始缓存有效性仍是 straw 义务；实际批量集合适配已在单列标准库定律下验证，两个库迭代原语仍属可信边界。原始解析／run 检查与 catalog Target 业务历史已在这些契约下组合；原生创建／append 对缓存历史前提的建立、Coordinator 业务 replay 仍未闭合。

`ENV-COMMIT-EFFECTS` 将原生单事务 retain／accept／apply／cleanup 义务与已证明分派器分开；`ENV-COMMIT-APPLICATION` 允许捕获绕过 Result 处理的 unwind。`ENV-TRAINING-CHECKPOINT` 由应用满足：一致 checkpoint 选择和 replay 保护不同于队列 processed 确认；这些契约不表示已有自动模型恢复能力。

`ENV-GC-BATCH-EFFECTS` 为实际批量删除执行体单列目标／父目录、unlink、目录屏障和回执契约。文件系统负责真实的名称缺失和同步持久性；straw 负责传入正确目标、保持持久 mark、处理 NotFound、修复缺失父目录并写入对应回执。Verus 已证明批次覆盖、`ENV-GC-PRIMITIVES` 下实际 `gc_io` 删除／父目录分派，以及 `ENV-DIRECTORY` 下实际祖先屏障循环。原生目标／路径／句柄绑定、祖先枚举及 repair 聚合持久性仍是独立义务。持续持久缺失还依赖封存对象不被其他 writer 重建，不能把暂停或崩溃 writer 的处理藏进同步假设。

`ENV-REQUEST-REPLAY` 将请求寿命与数据寿命分开：应用退休／GC 删除数据后，已接受回执仍可存在。环境允许重复 ID、冲突内容、丢失回复与反复崩溃；回执重建、冲突检查、授权及提前返回缓存是 straw 的义务。有限模型仍保留下层原子接受与完整跨日志细化边界。克隆／get／insert 的基础契约支撑实际回执代码，完整 key／hash 及 coordinator 调用组合尚未证明。


`ENV-JSON-LIBRARY` 另列借用 `str` 的 Index::index_into 基础接口：锁定版本调用 Map::get，缺失 key／非对象返回 None。实际 queue_event 目标选择及其 WAL／JSON 恢复对应已在此库契约下证明。另列 mutable index 基础接口，调用 Map::get_mut，约束选中字段的修改与其余字段保持，缺失字段时原值不变。实际 queue_task 的创建、retry、租约、终态、失败与续采样更新在这些库定律下验证；完整调用分派、reader 根、缓存对应及历史组合仍是 straw 义务。 实际 queue_state 已在既有库契约下证明嵌套任务查找、精确叶子分派、peer／非 task 根保持及有序统一更新批次／错误前缀；完整外层事件选择、计数器、缓存与混合历史仍是 straw 义务。 queue_admin 已证明有界序号分配、有序提交／producer 更新、epoch 赋值／撤销及 reader／回执表绑定；全局计数器／索引／历史对应仍未闭合。原生容量准入已测试缓存回复查询先于检查、检查先于写入，完整调用者对应尚未证明。 queue_transition 已证明任务／reader／回执的顺序效果和精确错误前缀；续读根保持推导另显式要求既有 String 内容身份前提，把复制的 attempt 绑定到同一 map key。完整根提取、准入及 catalog 所有权仍未完成。 实际 Value 相等另列为可信基础接口：相等保持有限观察，不保证序列化相同；固定的有序 map／float_roundtrip 配置由运行器审计。实际 reader 根／按序释放和完成／producer 更新已经证明；attempt 新鲜性、完整根及业务历史组合仍是调用者义务；worker 退休筛选现由下面的 retain 对应证明覆盖。

`ENV-JSON-LIBRARY` 新增正常返回的 Map::retain 基础契约：完整原始 key 覆盖、回调决定／最终值绑定及禁止新增 key。实际只读捕获的 worker 回调、reader 根绑定和事件分派另有函数体证明。原生缓存回调、完整准入／历史／根／所有权组合及 unwind 仍未完成，见[事件分派边界](FORMAL_VERIFICATION_zh.md#实际-queue-事件分派与-worker-退休)。


`ENV-JSON-VALUE-CONSISTENCY` 单列 Value 解析观察定律：同一字节的独立成功解释保留 null／bool／as_u64／text、数组顺序与对象 key／value，忽略分配和布局，不保证接受性或自定义 Deserialize 确定性。`ENV-STRING-KEY` 另列字符内容相等对应逻辑 String key 身份的前提。catalog 字段／操作／历史等价证明消费这些前提；实际物理前缀选择及完整 Coordinator／GC 组合仍未闭合。

`ENV-JSON-ENCODE` 单列外部库序列化关系与仅适用于 Value 的兼容定律，要求实际编译启用 `float_roundtrip`，不保证任意自定义 serde 类型往返一致。straw 必须保持输入字段、绑定 run、使用准确的序列化字节／操作并维护历史。catalog 解释唯一性已在独立的 `ENV-JSON-VALUE-CONSISTENCY` 与 String 内容身份前提下证明；原生适配仍未闭合。


当前 queue 历史边界分为三类责任：`ENV-JSON-LIBRARY` 及解析／编码契约提供库行为；实际 `queue_history` 证明准备、目标准入、有序业务更新、回执和精确部分失败。`Caches`／`Recovery` 是独立可变的回调对象，只读业务输入；其内容和终止性不受规格保证，正常返回时业务状态保持来自借用边界。原生缓存一致性、消费记账身份、panic 隔离及 run／File／fencing 对应仍是实现义务。`State::apply_business` 绑定实际解码向量，成功字节前缀到业务 fold 的组合已证明，没有假定独立 JSON 解析结果完全相同。见[历史组合记录](../verification/measurements/20261026-queue-history.json)。

`ENV-QUEUE-APPLICATION-HOOKS` 单独记录实际 before／after 回调及其返回成功标志，不保证成功、终止、持久化或业务／缓存内容正确。application 控制器证明精确回调／错误顺序与真实 journal 隔离；原生 callback 名称映射、完整 CommitIo 上下文及任意 unwind 仍分开审查。恢复只交出从初始状态完整重建的业务结果，失败 journal 保持隔离。见[application 证明边界](FORMAL_VERIFICATION_zh.md#实际-apply-隔离与全新恢复状态)。

`ENV-QUEUE-GC-CALLS` 将实际根选择到持久退休、锁内 collection 代码，与 JSON／digest、WAL 和具名文件系统原语分开。preflight 不带任何成功效果假设，且不能借用 queue／WAL 状态；progress 来源／缓存和外层锁上下文仍是 straw 调用者义务。`ENV-NAMESPACE-PRIMITIVES` 将文件系统／路径观察、借用的组件迭代与 straw 缓存／resolver 义务分开。`ENV-WRITER-RETIREMENT` 单列固定 gate 排他、永久 fencing，以及“已提交 I/O 完成或取消后才能移交锁”的独立要求；PID、超时或裸 advisory lock 都不能证明后者。原生适配和完整历史对应仍须另行建立。JSON Map key 唯一性是外部库表示事实，不是直接假设根集合保持。

## 文件与缓冲基础接口边界

`ENV-FILE-PRIMITIVES` 将 OS／库／回调保证与实际 `wal_source`、`wal_sink` 流程证明分开。缓冲器逻辑 cursor 可不同于底层文件位置；消费缓冲器必须保持身份／字节／持久性，后续 seek 再建立所需位置。返回错误可以伴随副作用；`Source` 保持显式有效性，失败后不能把缓存长度作为成功返回；`Appender` 从文件及 hook 操作推导 Writer 保证。零字节读取允许在 EOF 后成功。原生薄封装、快照一致性、外层锁／open／run 绑定与任意 unwind 仍是独立义务。见[文件流程证明范围](FORMAL_VERIFICATION_zh.md#实际恢复与追加的文件操作流程)。

`ENV-CATALOG-TRANSACTION` 将锁／open／metadata 基础接口与 straw 的 context、refresh、callback 义务分开。实际事务分派检查绑定和 fresh 身份，先恢复再调用回调，成功返回前再次检查上下文。原生适配仍需建立这些状态对应；留下 active 事务的合法回调必须保持身份。返回错误路径与捕获 unwind 回归是不同证据，锁操作报错不代表已经释放。见[事务证明范围](FORMAL_VERIFICATION_zh.md#实际-catalog-事务入口与退出)。

catalog 回调审查提供了具体例子：分派证明要求原生回调受事务作用域约束，但此前 Rust API 暴露 `&mut Catalog`。原生测试表明，移走对象可在解锁后保留写入资格，捕获 panic 也可能留下可直接使用的方法。借用的 `LockedCatalog` 回调和 unwind 失效处理修复了这些实现缺口。这是审查适配边界时发现的 API／实现缺陷，不是文件系统违约，也不是条件化 Verus 证明自动发现的 bug。

## 依赖遍历 source 与缓存边界

`ENV-CLOSURE-SOURCE` 属于 straw 适配义务：逐节点展开成功时，必须返回绑定节点自身的规范 pack 和全部 record／dependency 后继，并保持身份稳定。实际 `closure_node` 已在 Digest／Value 契约下证明字段／数组检查及完整有序摘要／引用绑定，图连接仍要求图域与 key 上下文前提。原生认证读取、序列化／SHA、namespace 及完整 `ClosureReader` 对应仍未闭合。在此接口下，实际 `closure::collect` 证明完整可达 pack 相等、有限图循环安全遍历、完整请求根缓存、清空保持有效及错误不写部分缓存；`empty_cache` 提供基例，`resolve` 拒绝任意可达 tombstone。缓存与图／上下文绑定、完整原生 retain／GC 仍是单独义务。`ENV-GRAPH` 保留有限 TLA+ 模型自己的图／缓存假设，review 记录此次较窄的实际 Rust 结果。[证明范围](FORMAL_VERIFICATION_zh.md#实际依赖遍历与闭包缓存)。

## GC 候选到 mark 的边界

实际 `gc_mark` 组合恢复后的 catalog 根与候选、resolved JSON 编码、持久追加和完整 pending deletion 快照，没有新增整段操作的环境假设。`ENV-JSON-LIBRARY` 仅新增两个外部基础接口：构造空 Map 和精确 u64 Number；削弱这两个契约的负例必须验证失败。既有 Writer／Resolver／JSON／String 前提继续保留；锁内状态新鲜度、恢复后规范身份、完整逻辑根、物理删除适配及重试进展属于 straw 义务，不是文件系统承诺。见[证明范围](FORMAL_VERIFICATION_zh.md#实际-gc-发现持久化-mark-与-pending-快照)。

## 各层边界

| 环境层 | straw 发出的操作／输入 | 环境行为或所需契约 | 审查位置与缺口 |
|---|---|---|---|
| 输入与运行时 | 过滤 ID、整数运算、集合操作 | 允许重复／空过滤（`ENV-INPUT`）；流式输入可短送／多送、改长度或忽略 callback 错误（`ENV-STREAM`）；信任机器整数和集合契约（`ENV-RUNTIME`）；回收索引要求 String 哈希／相等 key 定律（`ENV-STRING-KEY`），publication hold 要求 String 排序定律（`ENV-STRING-ORDER`）；允许冲突重试更改依赖（`ENV-PUBLICATION-RETRY`）；typed 匹配器收到的字段必须真实对应 envelope／调用方描述（`ENV-TYPED-DECODE`） | `EnvironmentInputs.tla`、Verus/vstd。String 特定定律尚未证明；没有分配器、编译后二进制和完整 FFI 证明 |
| 命名空间身份 | 解析相对 pack 路径、重放旧名称 | 符号链接解析真实，路径前缀／别名绑定稳定，普通／缺失响应不能隐藏 symlink（`ENV-NAMESPACE`） | `EnvironmentNamespace.tla`、`NamespaceIdentity.tla`；实际解析与 replay 对应未证明，未统一 bind mount／inode 等价 |
| 离线清理 | 检查被接受的根、unlink 孤儿路径 | 真实单队列判断、实际停机和完整引用解析绑定，包括 `..` 前的别名（`ENV-OFFLINE`、`ENV-OFFLINE-BINDINGS`） | `EnvironmentOffline.tla`、`EnvironmentBindings.tla`、`OfflineCleanup.tla`、`OfflineAliases.tla`；完整 Python／OS 细化未证明 |
| 进程内 writer 访问 | 发布持锁时，fault callback 或另一线程请求 metrics／seal／close | 非阻塞互斥 token、busy／poison 响应，callback 可等待发起请求的调用方（`ENV-LOCAL-ACCESS`） | `EnvironmentLocalAccess.tla`；实际 `access::decide` 已证明，标准 mutex／GIL／FFI 和适配对应仍未证明 |
| 读取可见性 | fresh open，读取并认证区域 | 不同次打开、不同 reader 可观察不同缺失区域，达到尝试次数界限后完整；允许持续损坏（`ENV-VIS`）。逐次完整／短读／错误响应见 `ENV-READ`。哈希在模型中无碰撞（`ENV-AUTH`） | `EnvironmentVisibility.tla`。这是超过 JuiceFS 正常文档行为的防御性模型；真实缓存页、哈希实现被抽象掉 |
| 持久化与恢复 | write、unlink、文件／目录 sync、replay | 成功 sync 持久化此前状态（`ENV-SYNC`）；可见目录不一定持久，创建失败可留下前缀，祖先需要单独屏障（`ENV-DIRECTORY`）；失败不等于回滚；文件名可以仍缺失或因崩溃恢复（`ENV-DELETE-FAILURE`）；stat 存在／缺失均可陈旧（`ENV-GC-STAT`）；GC 成功的 unlink／可信 unlink NotFound 必须意味着当前名称不存在（`ENV-GC-IO`）；已封存空 reservation 的父目录可能从未创建，打开目录的 NotFound 不等于持久删除（`ENV-GC-PARENT`）；结果不确定的完整 frame 可以幸存（`ENV-WAL-RECOVERY`）；串行 append 可留下 header／payload／trailer 的任意未确认前缀（`ENV-WAL-PREFIX`） | `EnvironmentPersistence.tla`、`EnvironmentWal.tla`。没有证明服务／设备断电行为及完整字节级 replay 适配层 |
| 物理批量回收 | 按目标索引 unlink、同步共享父目录、记录回执 | 目标／父目录绑定稳定，已 mark 名称不可重建，unlink 与目录持久化响应真实；错误可已产生效果（`ENV-GC-BATCH-EFFECTS`、`ENV-GC-PRIMITIVES`、`ENV-DIRECTORY`） | 实际批次／unlink／父目录分派及有序祖先屏障已证明；原生目标／路径／句柄、repair 聚合持久性及完整根集合／活性未闭合 |
| 未完成写入与存活 writer | 记录起点、truncate／unlink、同步后重试 | 私有文件只有存活 owning writer 修改；截断保留准确起点之前的前缀（`ENV-INCOMPLETE`），失败可以已经移除尾部 | `EnvironmentIncomplete.tla`、`IncompletePublication.tla`；offset 捕获、文件身份、PID 检查位置、进程崩溃恢复未证明 |
| Writer 进程生命周期 | 已获准的写入、暂停／恢复、关闭 | 暂停可恢复，进程死亡与存储故障分开；怀疑超时不意味着旧写入已失去能力（`ENV-WRITER-LIFETIME`） | `EnvironmentWriter.tla`、`WriterLifetime.tla`；当前保留未关闭 reservation，没有实现崩溃 writer 的安全回收 |
| 发布准备阶段 | 登记新名称、创建／打开、取得 offset | 报错可能已经完成登记或创建空文件（`ENV-PREPARATION`）；私有身份不复用 | `EnvironmentPreparation.tla`、`PublicationPreparation.tla`；guard 的调用位置和完整 I/O 适配尚未证明 |
| 完整 publication 的交接 | staging、seal、内部 owner 释放、成功返回 | 失败可能已生效；调用方可改用另一 ID／API 或冲突内容（`ENV-HANDOFF`），不能假设它会自行等待原请求完成 | `EnvironmentHandoff.tla`、`PublicationHandoff.tla`；实际请求匹配、入口检查、完整 I/O／GC 细化未证明，库返回不等于远端 RPC 确认 |
| Staging 释放与回收后重试 | 展开依赖、释放、重新打开／重试 | 释放响应丢失后，所有无 owner 的 manifest 都可能已被 GC；引用身份对应不变的完整直接依赖（`ENV-CONSUMPTION`） | `EnvironmentConsumption.tla`、`StagingConsumption.tla`；直接边与 owner 释放同 frame、完整 replay／字节绑定仍是适配义务，旧日志可能没有依赖图 |
| 队列提交后的 staging 清理 | WAL 接收、释放 staging、查询缓存结果、恢复 | 接收已持久化后，清理仍可失败；释放回执必须真实，publisher 可再次 staging 同一引用（`ENV-COMMIT-CLEANUP`） | `EnvironmentCommitCleanup.tla`、`CommitCleanup.tla`；同 frame 保存意图／回执、恢复与入口屏障是 straw 的义务，完整适配未证明 |
| Catalog 与其他客户端 | lock、refresh、retain/release、scan/mark | staging／seal 报错可能已经持久生效（`ENV-PUBLICATION-FINALIZE`）；初次 fresh open 的缺失可信、池模式在客户端启动前确定（`ENV-GC-DISCOVERY`）；scan 到持久 tombstone、恢复重新检查到 owner append 的临界区与守约客户端串行化（`ENV-CATALOG-SERIAL`）；其他参与者提供完整依赖闭包，真正结束读取后才释放（`ENV-PEERS`） | `EnvironmentCatalog.tla`、`EnvironmentPersistence.tla`／`PublicationFinalization.tla` 及 `Ownership.tla` 的 peer 动作。分布式锁实现、新鲜状态、原子持久 catalog 更新仍是假设 |
| 依赖图与闭包缓存 | 展开引用、复用缓存闭包 | 有限不可变图；成功展开枚举全部后继，缓存返回完整传递闭包（`ENV-GRAPH`） | `EnvironmentGraph.tla`、`DependencyTraversal.tla`。遍历模型抽象为成功操作，未组合真实 I/O 错误、JSON 解码和恢复；不提供堆内存界限 |
| Coordinator 与时间 | 发出／使用 lease、完成请求、替换 owner | 外部保证只有一个 fenced owner（`ENV-FENCING`）；过期是 idle 时的非确定布尔事件（`ENV-CLOCK-ABSTRACT`） | `EnvironmentClock.tla`、`Completion.tla`。没有数值时钟漂移、延迟 RPC 队列、选主或 split-brain 模型 |
| 执行进展 | 调度步骤、失败后重试 | 动作弱公平（`ENV-FAIR`）；GC 故障期最终结束，但不限制之前重试次数（`ENV-STABILIZE`）；失败后外部调用者还需重新调用 GC（`ENV-GC-RETRY`） | `EnvironmentProgress.tla`、`environment.rs`。GC 删除重试模型已由这些前提推导完成；没有逐任务防饥饿保证 |
| Verus 模型／适配边界 | 推进控制器、更新持久 catalog | 通用控制器显式消费稳定效果响应、效果／重试弱公平（`ENV-VERUS-PROGRESS`）；实际历史必须对应串行 catalog 模型步骤（`ENV-VERUS-CATALOG`） | `verification/common/verus/environment.rs`。**真实 I/O、锁和恢复适配是否产生这些历史、响应与公平性仍未证明** |
| 部署与性能 | 在实际机器运行完整负载 | 可见性、带宽和控制延迟随工作负载、缓存及客户端状态变化（`ENV-PERFORMANCE`） | 测量记录与 [FILESYSTEM](FILESYSTEM_zh.md)。实测 p99 不能成为硬延迟上界或定理前提 |

这些边界是相对于各自证明层而言的。`Ownership.tla` 把一次 catalog 更新视为原子且持久；下层的 GC 删除、WAL 模型则拆分相关副作用。`DurableHandoff` 现在组合检查独立日志与 GC 的有限交错，但这仍未从下层实现推导出上层的原子契约；完整细化证明仍然缺失。其他客户端的 acquire/release/rewind 动作留在 `Ownership.tla` 中：它们是遵守协议的参与者，不是任意攻击者，也不是所有 peer 实现都已被验证。

## 与验证层级的对应

[FORMAL_VERIFICATION](FORMAL_VERIFICATION_zh.md) 的 L0–L5 是证据层级；上面的表是组件边界。独立 review 时需要同时标明两者。

| 验证层 | 此层消费的环境 | 此层通过以后仍需审查 |
|---|---|---|
| L0 契约 | 部署／协议声明及未建模事项 | 声明是否过强、是否遗漏允许的响应；本层不产生证明 |
| L1 TLA+ 搜索 | 独立 `Environment*.tla`、配置输入域和公平性 | 有限状态边界、隐含前提，以及真实代码能否对应模型步骤 |
| L2 Verus 数学证明 | `environment.rs` 中的历史、I/O／崩溃响应、公平性及调用方重试条件 | 无界定理依赖哪些假设；下层是否真的提供串行 catalog 等抽象动作 |
| L3 实际 Rust 函数 | 函数前置条件、vstd 运行时契约和调用方传入的描述／结果 | 调用位置、值与真实文件的绑定、syscall 返回解释；被证明的函数不会替调用方履行前置条件 |
| L4 集成测试 | 测试夹具、故障 hook／syscall 注入、进程调度 | 注入是否代表目标响应，覆盖的是哪些切点；未注入的执行仍未被穷尽 |
| L5 部署测量 | 实际 JuiceFS 客户端、缓存、网络、负载及实验清理方式 | 样本与部署是否相符；分位数和带宽是测量结果，不自动成为 L1–L3 的保证 |

同一个 `ENV-*` 可被多个层级消费。下层出现反例时，沿清单查找消费者；新增实测现象时，先判断是否在现有允许行为内，再决定修实现还是修环境模型。

## 层间契约由谁满足

这里的“环境”是相对于当前被验证的组件而言的，可能包含 **straw 自己尚未验证的下层代码**。因此，“实际执行违反了环境假设”只是定位到一个契约边界，不能直接归因于文件系统。

| 上层消费的契约 | 实际需要满足契约的部分 | 独立 review 时要区分的情况 |
|---|---|---|
| `ENV-VIS`、`ENV-AUTH`：读取观察与认证 | 文件系统／客户端缓存，以及 straw 的 fresh open、区域选择、校验和错误分类 | 观察超出允许的延迟／损坏模型，还是实现读错区域、漏校验或没有真正重新打开 |
| `ENV-PUBLICATION-FINALIZE`：staging 与 sealing 响应 | straw 发布完成适配及串行 catalog／恢复实现 | 独立 API 是否在 catalog seal 前返回成功、失败操作是否已生效、seal 前是否已有 staging；`.sealed` 名称本身不满足 catalog 契约 |
| `ENV-INCOMPLETE`：未完成写入清理 | 文件系统 truncate／unlink／sync，OS 进程身份，以及 straw 私有 writer／恢复适配 | 截断损坏旧前缀，还是起点记录错、清理失败丢描述、漏同步、fork 后触碰父进程文件；存活 writer 的重试不能代替崩溃恢复 |
| `ENV-WRITER-LIFETIME`：进程状态与 open reservation | OS 调度／进程死亡、straw 的写入与 close 生命周期，及未来恢复协议的 fencing | 进程只是暂停、确已死亡，还是旧操作已被禁止；存储恢复但没有 close 导致的保留，与已封存目标的删除失败是不同边界 |
| `ENV-PREPARATION`：准备阶段不确定副作用 | 唯一私有名称、文件系统 open／unlink／屏障、catalog replay 和存活 writer 的适配 | 报错可能已经创建状态；区分清理描述遗失、过早使用 offset、删除既有前缀、漏屏障及进程死亡 |
| `ENV-COMMIT-CLEANUP`：已提交事务的清理回执 | WAL／catalog 持久前缀、straw 的标记／回放／索引及调用方重试 | 缓存结果跳过清理、未持久保存意图、回执与释放不同 frame，还是下层错误报告持久回执；恢复时重做已完成释放会误消费后来的 staging |
| `ENV-CONSUMPTION`：不可变依赖与回收后重试 | Manifest 认证、catalog frame／replay、其他 owner 和 GC | 根 owner 不存在不能证明整个依赖已消费；区分真实边遗漏、未保存完整图、图与释放分开持久化、replay 遗失图，或旧日志未记录这些信息 |
| `ENV-HANDOFF`：请求交错与成功 staging | 调用方请求、catalog 响应及 straw 的完整交接适配 | 其他请求本来就允许出现；覆盖未返回文件的描述是实现错误。只有真实 staging 成功却没有建立约定 owner，才违反这里的 catalog 契约；仍需检查下层适配才能归因 |
| `ENV-PENDING-PUBLICATION`：完整文件的 sync 响应 | 文件系统持久性、私有路径稳定性及 straw 的 pending／重试适配 | 成功屏障未持久化，还是描述保存过晚、重试换了文件、遗漏屏障或校验；保留描述是算法义务 |
| `ENV-NAMESPACE`：别名映射到一致身份 | 文件系统查询、稳定部署命名空间，以及 straw 路径／缓存／replay 适配 | readlink／缺失响应不真实、别名被替换，还是 owner／tombstone／receipt 使用原始字符串；缓存正确不等于解析正确 |
| `ENV-GRAPH`、`ENV-PEERS`：完整闭包与 owner 生命周期 | straw 的引用解码、遍历和闭包缓存，以及其他队列／reader／checkpoint 的调用方 | 依赖在输入里就缺失、遍历／缓存漏边，还是仍在读取时提前 release；这些问题不能仅靠存储一致性解释 |
| `ENV-DIRECTORY`：目录链持久性 | 文件系统目录项、稳定别名／挂载及 straw 的 create／路径解析／祖先同步 | 创建失败后遗漏屏障、只同步叶子、同步失败被吞掉，还是外部替换目录／别名或成功屏障未持久化 |
| `ENV-GC-DISCOVERY`：首次发现 GC 池 | 池初始化、文件系统名称查询，以及 straw 的 open／错误处理和共享 GC 要求标记 | 首次 open 虚假返回 NotFound、将错误解释为不存在、忘记 GC 要求，还是仍有活跃 legacy 客户端时改变池模式 |
| `ENV-CATALOG-SERIAL`、`ENV-VERUS-CATALOG`：串行、持久的 catalog 动作 | 分布式锁及 straw 的临界区、refresh、append／sync、replay 适配 | 锁不互斥，还是 straw 绕过锁、使用旧快照或错误地把未持久动作视为成功 |
| `ENV-SYNC`、`ENV-GC-IO`、`ENV-DELETE-FAILURE`：删除与持久化响应 | 文件系统，以及 straw 对 stat／unlink／目录 sync 的调用顺序和结果解释 | 真实屏障成功后状态仍丢失，还是屏障漏调用／调用在错误目录，或将不可信 NotFound 当成实际不存在 |
| `ENV-GC-PARENT`：删除后的父目录可同步性 | 私有命名空间、创建权限、目录 open／mkdir／sync，以及 GC 的分类和重试适配 | 空 reservation 的父目录从未创建，还是已存在目录被误报缺失；漏修复导致无限重试，或将缺失当成 receipt；补目录不能替换已有目录身份 |
| `ENV-APPEND-IO`：追加字节与同步效果 | 文件系统 descriptor／字节语义、SHA 和 straw Appender | 共享追加与 catalog 确认已在 Writer 下证明；须区分原语响应不真实、原生 fd／callback 处理错误及 payload／操作绑定错误 |
| `ENV-WAL-RECOVERY`、`ENV-WAL-PREFIX`：持久前缀与不确定／不完整 frame | 存储持久性，以及 straw 的 frame 校验、截断和恢复同步 | 已确认持久字节丢失，还是恢复器漏掉合法 frame、接受损坏 frame 或过早使用恢复结果 |
| `ENV-CROSS-LOG`：独立日志与删除响应 | 两份日志的持久前缀、目录屏障及 straw 的跨日志调用顺序 | retain／退休尚未持久化就推进另一日志、恢复缓存被过早信任，还是存储丢失已确认前缀；两个日志的失败选择必须独立 |
| `ENV-VERUS-PROGRESS`：稳定效果及两类弱公平 | 操作响应、调度与调用方重试，以及其到控制器 View 的对应 | 存储恢复但无调用方重试、持续的应用输入错误、未返回的锁／I/O，或真实恢复没有建立模型要求的历史 |
| `ENV-FENCING`、`ENV-CLOCK-ABSTRACT`：唯一 owner 与过期响应 | 外部 owner 管理、实际时钟，以及 lease／RPC 适配 | 是否存在多个可写 owner；抽象布尔过期是否足以表示现场顺序。当前没有数值漂移和延迟 RPC 的组合模型 |
| `ENV-FAIR`、`ENV-STABILIZE`、`ENV-GC-RETRY`：执行进展 | 调度器、存储服务和重新调用 GC 的调用方 | 持续错误、动作饥饿和失败后不再调用是不同前提失效；进展前提失效本身不等于所有权安全失效 |

每条契约的消费模型由清单中的 `models[*].contracts` 给出。Review 时沿“消费模型 → `ENV-*` → 提供该保证的代码／服务 → 证据”追踪；只有补上实现与下层模型到该契约的对应证明，才能把这条上层假设标为已推导。测试通过或把假设移到独立文件都不能完成这一步。`ENV-VERUS-PROGRESS` 现已用于推导跨失败／重试的完成，而非直接声明下一阶段最终成功；真实 I/O／恢复适配及整个控制面的对应证明仍未完成。


catalog 状态与物理 replay 接口是两个独立 review 边界。`catalog_log.rs` 已证明有类型 frame 历史的 fold、索引／cursor／sequence 一致、部分 apply 隔离、rebuild 和匹配确认；`ENV-REPLAY-APPLY` 仍要求原始 payload／解码绑定，`ENV-REPLAY-IO` 仍要求 Source 缓冲快照及原语返回值的对应。Source 独占借用实际 File，但这不能证明 sync 已执行；`ENV-VERUS-CATALOG` 仍要求完整的持锁持久历史。这些属于 straw 义务，须与文件系统响应是否真实分别审查。

## 已拆出的可执行规范

TLA+ 算法现在导入独立模块：

- [EnvironmentVisibility.tla](../verification/storage/tla/EnvironmentVisibility.tla)：每次 fresh open 允许的区域观察。
- [EnvironmentNamespace.tla](../verification/common/tla/EnvironmentNamespace.tla)：稳定的解析后命名空间身份（`ENV-NAMESPACE`）。
- [EnvironmentBindings.tla](../verification/offline/tla/EnvironmentBindings.tla)：被接受引用所需的完整解析绑定（`ENV-OFFLINE-BINDINGS`），最终身份本身不足以保护可读性。
- [EnvironmentReaders.tla](../verification/protocol/tla/EnvironmentReaders.tla)：调用方仅在实际读完后确认 reader；取消、超时和恢复不提供这个证据（`ENV-PEERS`）。
- [EnvironmentRead.tla](../verification/common/tla/EnvironmentRead.tla)：完整读取、短读或 I/O 错误；失败可消费前缀，单次尝试允许持续损坏（`ENV-READ`）。
- [EnvironmentAuthentication.tla](../verification/storage/tla/EnvironmentAuthentication.tla)：范围读取模型消费的无碰撞 chunk 认证契约（`ENV-AUTH`）；预期字节与引用的绑定仍是实现义务。
- [EnvironmentIncomplete.tla](../verification/storage/tla/EnvironmentIncomplete.tla)：截断保留前缀以及失败时未发布尾部的可能变化（`ENV-INCOMPLETE`）。
- [EnvironmentWriter.tla](../verification/gc/tla/EnvironmentWriter.tla)：进程暂停、恢复、死亡与不可靠的怀疑信号（`ENV-WRITER-LIFETIME`）。
- [EnvironmentCommitCleanup.tla](../verification/protocol/tla/EnvironmentCommitCleanup.tla)：提交后释放的错误响应及真实持久回执（`ENV-COMMIT-CLEANUP`）。
- [EnvironmentLocalAccess.tla](../verification/storage/tla/EnvironmentLocalAccess.tla)：进程内非阻塞互斥 token 与 busy／poison 响应（`ENV-LOCAL-ACCESS`）。
- [EnvironmentConsumption.tla](../verification/protocol/tla/EnvironmentConsumption.tla)：不可变引用的直接依赖，以及释放后 manifest 可消失的边界（`ENV-CONSUMPTION`）。
- [EnvironmentHandoff.tla](../verification/protocol/tla/EnvironmentHandoff.tla)：未完成交接期间的任意请求与成功 staging 契约（`ENV-HANDOFF`）。
- [EnvironmentPreparation.tla](../verification/storage/tla/EnvironmentPreparation.tla)：新私有名称登记／打开失败后的不确定副作用（`ENV-PREPARATION`）。
- [EnvironmentParentRepair.tla](../verification/gc/tla/EnvironmentParentRepair.tla)：缺失父目录、打开响应和幂等创建的环境（`ENV-GC-PARENT`）。
- [EnvironmentCrossLog.tla](../verification/protocol/tla/EnvironmentCrossLog.tla): 独立队列／catalog 的完整帧恢复投影，以及文件／目录持久化响应（`ENV-CROSS-LOG`）；跨日志不是一个原子事务。
- [EnvironmentWal.tla](../verification/wal/tla/EnvironmentWal.tla)：串行追加的三段前缀、成功同步与完整帧认证；崩溃可保留未确认的部分帧（`ENV-WAL-PREFIX`），不等于已证明字节解码。
- [EnvironmentPersistence.tla](../verification/common/tla/EnvironmentPersistence.tla)：成功 sync 的效果、失败后文件名和 frame 的可能状态。
- [EnvironmentCatalog.tla](../verification/common/tla/EnvironmentCatalog.tla)：串行锁假设下的 catalog 访问条件。
- [EnvironmentClock.tla](../verification/protocol/tla/EnvironmentClock.tla)：抽象布尔过期响应。
- [EnvironmentInputs.tla](../verification/common/tla/EnvironmentInputs.tla)：外部请求／过滤输入域，包括重复 filter 和冲突重试的依赖集合。
- [EnvironmentGraph.tla](../verification/common/tla/EnvironmentGraph.tla)：引用图与完整缓存闭包契约。
- [EnvironmentProgress.tla](../verification/common/tla/EnvironmentProgress.tla)：弱公平与故障最终停止的前提。

这些定义不选择重试策略、任务分配、tombstone 候选或 receipt 接收规则，算法决策仍留在算法模块。各模型仍通过一个 `Next` 组合算法动作和环境选择；尚未建立统一的异步文件系统／RPC 模拟器。

Verus 的通用控制器现在由 [`retry_liveness.rs`](../verification/protocol/verus/retry_liveness.rs) 从稳定的效果响应、效果调度弱公平和调用方重试弱公平推导完成；旧 `progresses` 前提和辅助定理已删除。新模型以生产 `initial`／`advance` View 表达失败、Crash、Retry 和停顿，对任意有限阶段数证明最终完成，并给出停止重试的无限反例。`ENV-VERUS-PROGRESS` 的提供者仍须保证有效输入、操作最终返回和真实响应映射；文件系统恢复本身不能证明每个应用阶段最终成功。`serialized_catalog_history` 仍是更高层无界所有权证明的抽象历史前提。

针对 GC，新增 [`gc_liveness.rs`](../verification/gc/verus/gc_liveness.rs) 的无界删除重试证明，**不消费 `progresses`**。它显式区分成功、带部分副作用的失败、崩溃、失败后的新调用和停顿；用真实删除控制器的 `initial`／`advance` View 转移。`environment.rs` 独立定义 I/O 响应、崩溃响应、故障最终停止及 I/O／重试的弱公平性。失败后 rank 为 3，重新调用后为 2，Unlink 后为 1，目录同步后为 0；证明稳定期最终到达持久删除，不限制稳定期之前的失败次数。

这里假定目标 pack 已有持久 tombstone，永久无 owner，也不会被其他客户端重建。成功的 unlink（包括可信 unlink NotFound）必须意味着实际当前不存在。stat NotFound 已不再用来满足该前提；**unlink 自身是否可能受陈旧负缓存影响、真实 syscall 适配、catalog 锁和重放到这个模型的对应关系仍未证明**。GC 不会自动在后台重试：调用者停止调用时，即使存储恢复，也可能永远留下未持久化的删除。Verus 另证明了这个无限反例，以及一个满足全部前提、经历失败后完成的实例，避免把不可满足的前提当作进展证明。

三个新的 Verus 环境对照分别让目录同步成功撒谎、取消重试公平、允许错误永不停止；另一个算法对照让重试不重置失败控制器。它们都必须产生语义验证失败，报告区分 `environment_negative` 与 `algorithm_negative`，记录实际变异环境哈希。

纯整数和过滤循环的 Rust 证明依赖运行时契约，不依赖文件系统进展。vstd 提供的集合规范是可信依赖；禁止项目中的 `assume`／`admit`／省略函数体，不意味着这里证明了 vstd、求解器或编译器。

实际 `reclamation.rs` 的 HashSet 修改和 `traversal.rs` 的泛型队列显式要求 `environment.rs::string_hash_contract()`（`ENV-STRING-KEY`）：标准 `String` 哈希／相等关系符合 vstd 的 key 模型。固定版本 vstd 尚未证明这个 String 特定前提，因此结果以它为条件。它属于运行时／库契约，不是文件系统或公平性假设。证明仍检查实际修改：重复 tombstone 不重新安排已完成工作，receipt 只能从 pending 中移除已标记路径。

`GCBatchDeletion.tla` 还消费 `EnvironmentPersistence.tla::EnvSyncedDirectory`：父目录屏障只持久化其成员名称之前的变化，不改变其他目录（`ENV-SYNC`、`ENV-GC-IO`）。模型提供稳定的 pack 到父目录映射；Rust 适配层必须为全部 tombstone 构造完整目录集合，包括已经不存在的名称。这一映射／覆盖关系尚未由批量完成检查循环证明。过早屏障、遗漏父目录均有独立的算法负向对照。

`GCReceipt.tla` 组合删除持久性和完整 frame 恢复（`ENV-SYNC`、`ENV-WAL-RECOVERY`）。所有父目录屏障先于 receipt 写入，因此 append 报错后幸存的完整 receipt 仍安全；缺失／撕裂 receipt 保留待删除义务。receipt 不删除永久 tombstone。这里沿用禁止路径重建、其他客户端遵守协议的契约，没有建模恶意客户端伪造合法 receipt。字节级 replay、真实适配层顺序和新旧客户端交错分别由 `tests/test_gc_receipts.py`、`verification/gc/probes/gc_receipt_probe.py` 测试，不由这个有限模型或 HashSet 契约证明。单 pack 无界 GC 活性证明尚未扩展到 receipt 持久化与恢复。

`PayloadStream.tla` 复用输入环境中的 `EnvStreamLengths`，明确不信任自定义 provider 的长度与 callback 错误传播。算法分别检查两次遍历；provider 是否返回仍是前提。模型的 callback 数／长度上限只限制搜索，不能拿来证明任意自定义代码终止。Rust 长度 guard 已验证全部 `u64` 输入；trait／closure 与 I/O 的完整对应关系尚未证明。这里的错误并非 JuiceFS 可见性故障，不能靠增加 reader 重试预算解决。

`EnvironmentGraph.tla` 单独声明 `ENV-GRAPH`：有限且不可变的图、完整后继枚举，以及缓存值等于完整传递闭包。`DependencyTraversal.tla` 消费它，分别检查遗漏边和不完整缓存；`Ownership.tla` 仍假定输入闭包正确，两者还没有完整组合证明。实际泛型队列沿用 `ENV-STRING-KEY`，没有新增 String 定律证明。文件／JSON 到节点和物理 pack 的映射、真实缓存正确性与分配成功仍需独立审查。

`ReactivationScan.tla` 消费 `ENV-READ`，把 `read_exact` 的完整、短读和错误响应与扫描策略分开。失败可能已经读到一部分，但这些字节不能使本次尝试授权 owner；损坏检测依赖 `ENV-AUTH`。它验证一次已获得完整受保护闭包的扫描，未组合重试、实际锁、引用解码和 WAL。真实 syscall 是否返回仍在适配边界之外。payload scratch 的 1 MiB 上限是 straw 的算法选择，由实际 Rust 容量函数保证；不是测量得到的文件系统常数，也不是整个进程的内存上限。

新增 `ReactivationRace.tla` 单独审查锁外校验的环境：peer 可以在扫描期间 release、retain、mark 和 unlink，已打开描述符可能仍然校验成功；**校验结果不保证此刻还有 owner，也不提供 pin**。最终授权必须在刷新后的同一次锁内重新检查保护来源与 tombstone 并 append。身份绑定依赖不可变 extent／完整引用（`ENV-AUTH`、`ENV-GRAPH`），串行临界区依赖 `ENV-CATALOG-SERIAL`。只改 `EnvironmentCatalog.tla`、让 peer 绕过最终锁的对照产生 `NoDanglingRestore` 反例；该对照与两个算法竞态对照显式选择这一不变量及 `TypeOK`，正例仍检查全部性质。它没有把字节扫描、真实文件描述符、私有校验凭据或 WAL 恢复统一证明。Coordinator 已用锁外扫描；低层兼容 API 仍可持锁扫描，metadata 遍历和最终检查／写入也仍需锁，不能从这些模型推出锁等待上界。

`CatalogReplay.tla` 补充了 `ENV-WAL-RECOVERY` 的一个交错：恢复 sync 失败后，存储可丢掉未确认的完整尾部，**客户端进程和它已解析的缓存却继续存在**。环境不保证“解析成功即持久”，也不要求客户端随存储一起重启。算法必须在缓存可能包含未确认修改时重建派生索引，同时保留本对象已确认的持久前缀下限。空刷新没有这种修改，可复用已确认缓存，但仍要 fresh open、检查新 frame 和 sync；这是一项算法工作量策略，不是新的环境保证。模型的稳定 I/O 和公平重试是定性前提；真实缓存重建、字节 replay、锁和 syscall 对应仍需实现审查。四机故障探针主动模拟这个响应，不能据此宣称 JuiceFS 部署真实发生过尾部丢失；详见 [FORMAL_VERIFICATION](FORMAL_VERIFICATION_zh.md#catalog-重放失败与未确认缓存)。

### 一次适配层反例：陈旧 stat

`ENV-GC-STAT` 允许 stat 的存在／缺失观察不可靠。旧 `gc.rs` 在 stat NotFound 后跳过 unlink，却将控制器推进为 Unlink 成功；已有的删除控制器证明不能证明这个分发选择正确。扩展 `GCReceipt.tla` 的 stat 步骤后，该分支违反 `ReceiptIsSafe`。修复要求真正调用 unlink，再按其结果推进；stat 不再是删除完成证据。这里定位到的是 straw 适配层没有满足 `ENV-GC-IO`，而不是目录 fsync 保证失效。

四机 1,200 次自然 stat 观察未见陈旧 NotFound；单次错误 stat 的注入仍能稳定复现旧缺陷。环境模型允许的行为范围、部署观测及实现修复是三种不同证据。unlink NotFound 的真实性仍是独立假设；已写入的错误历史 receipt 不会自动重新验证。详见 [测量与范围](FILESYSTEM_zh.md#gc-陈旧-stat-与删除凭据2026-10-23)。

## Writer 存活性与回收资格

`ENV-WRITER-LIFETIME` 单独区分运行、暂停和死亡。暂停的进程可以从已经通过 catalog 检查的位置继续写；已经交给内核或服务端的写入也可以在进程暂停、死亡后完成。`EnvWriterCompletes` 因此独立于进程是否正在运行。“存储错误最终停止”不要求一个已死亡进程恢复，也不要求它执行 close。怀疑／超时可以发生在任何进程状态，模型没有把它当作死亡证明。不调用显式退休的基线策略仍可无限保留 open reservation。新增 incarnation 退休 API 在 ENV-WRITER-RETIREMENT 下提供持久 fencing 和已登记 publication reconciliation，仍依赖真实排他、namespace／sync 语义，以及 gate 移交前已提交 I/O 完成或取消的独立条件。PID 或超时不能替代这些要求。

[`WriterLifetime.tla`](../verification/gc/tla/WriterLifetime.tla) 从一个已注册、无 owner、写入中的 open pack 开始。正常策略检查不回收活跃写入、正常关闭后的条件进展；干净场景还检查最终删除。这些对照明确区分：

- **不安全的候选算法**：暂停 → 怀疑超时 → collector 擅自 seal → writer 恢复 → GC mark → 已获准的写入继续，违反 `NoWriteAfterRetirement`。当前 straw 没有采用这种算法；这不证明返回给用户的数据已经丢失。
- **死亡仍不足以退休**：`writer_dead_has_inflight` 检出死亡后直接 seal／mark 违反 `NoLiveWriterRetired`；`writer_dead_late_write` 给出死亡 → seal → mark → 在途写入完成的 `NoWriteAfterRetirement` 反例。正常策略在允许此交错时仍安全，`writer_crash_safe_leak` 为正例。
- **不调用显式退休的基线策略的进展边界**：初始 writer 已死亡、所有存储操作成功、调度满足弱公平，仍可永远保留 open reservation。`RequiredProgress` 有真正的无限停顿反例；这没有推翻以“目标已 sealed、持久 tombstone、永久无 owner”为前提的 GC 删除定理。

独立 review 要检查环境提供的是“暂时没有活动”“怀疑死亡”，还是足以阻止旧操作继续的 fencing。未来恢复协议必须处理暂停前已经获准的操作；PID、超时或取得某个锁本身尚未在这里被证明能提供该保证。锁服务／session 失效、网络分区和恢复没有统一模型。正常 close、catalog 序列化和 in-flight 写入的实际适配仍未细化到此 TLA+ 模型；完成与 release 被抽象合并，字节持久性和完整所有权模型未组合。

[四机记录](../verification/measurements/20261023-writer-lifetime.json)包括 16 个真实 SIGSTOP／SIGCONT／SIGKILL 场景，每次在 header 后、catalog 临界区外暂停。8 个恢复场景完成并校验 2 KiB／8 MiB payload，释放、关闭后正常 GC；8 个终止场景的三次后续 GC 都保留 open pack。终止时每个文件只有 12 字节 header，合计 96 字节由实验私有目录清理，不能计作 GC 成功。这些有限观察不证明无限泄漏，不是跨主机锁／断电测试，也不能产生安全超时或带宽结论。

可单独导出这层的提供方、实现义务、未证明部分和消费模型：

```sh
python verification/runners/environment_manifest.py --contract ENV-WRITER-LIFETIME
python verification/runners/environment_manifest.py --model WriterLifetime
```

## 参数到底属于哪一边

| 参数／数值 | 分类 | 含义 |
|---|---|---|
| `MaxRetries`、重试等待 | 算法策略 | reader 愿意花费的尝试／等待；总等待不是 I/O 截止时间 |
| `verification_capacity` 的 1 MiB 上限 | 算法策略 | payload 校验 scratch 上限，不包含 metadata／图／返回值；不约束存储响应时间 |
| `VisibleBy` | 环境前提 | 从第几次**尝试**开始 fresh open 保证完整，不是毫秒数 |
| `AllowedGaps`、`Corrupt` | 环境行为 | 允许的缺失观察／持续损坏 |
| `Readers`、`Tasks`、identity 集合、`MaxFilter`、小 `WordMax`、崩溃／epoch 次数 | 有限搜索边界 | 限制 TLC 状态空间，不是 API 配额或实际文件大小限制 |
| 弱公平、故障最终停止 | 时序前提 | 定性的进展假设，不提供时间界限 |
| 实测 p99／最大值 | 部署证据 | 样本分位数／最大值，不能直接填成全局收敛时间或 lease 界限 |

几 KiB 到真实单次 100 GB 的数据测试与稀疏 TB 偏移检查覆盖不同边界。稀疏偏移不证明 TB 吞吐，大数据测试也不证明元数据服务持续可用。审查假设时必须保留测量负载和适用范围。

## 证据与独立 review

两个形式化运行器都会把环境清单复制到报告目录。TLC 为每个案例记录算法哈希、配置哈希、实际环境模块哈希、契约编号和 `case_kind`；Verus 记录全部实际证明输入及环境哈希。registry 与模型 import 不一致时，验证启动前就报错。CI 保存这些文件，sdist 也包含它们。

不启动 TLC／Verus 也可以单独校验和查询清单：

```sh
python verification/runners/environment_manifest.py --check
python verification/runners/environment_manifest.py --list-layers
python verification/runners/environment_manifest.py --layer durability --layer progress
python verification/runners/environment_manifest.py --contract ENV-GC-IO
python verification/runners/environment_manifest.py --model GCReceipt
```

`--list-layers` 从清单生成组件层与契约编号索引；`--layer` 导出选中层的全部契约、提供方、实现义务、未证明部分、证据及受影响的消费模型。这里的层是组件边界，不是 L0–L5 证据层级。`--contract` 进一步缩小到指定契约；`--model` 则导出一个模型跨层消费的全部前提，并列出每条前提的其他消费者。这三种选择方式互斥，各自可重复；不带参数导出完整 JSON。所有审查报告都包含由同一清单生成的 `layers` 索引。

例如，持久化层的审查不能代替完整 GC 审查：GC 还依赖所有权、命名空间、调度和重试。先用 `--layer durability` 审查这一边界，再用 `--model GCReceipt` 或 `--contract ENV-GC-IO` 追踪消费者。`ENV-PERFORMANCE` 没有形式化消费模型，这是有意保留的测量边界。

检查器拒绝缺失的审查字段、重复编号／列表项、无效证据路径、未知契约、遗漏的 TLA+ 消费模型，以及环境 import 与清单不一致。消费关系从同一份清单反向生成，避免手写两份索引。它只检查**已声明依赖的结构一致性**，不能证明没有漏写语义前提，也不能证明证据已经满足契约。当前支持单行 `EXTENDS`；规范新增 import 形式时需要同时更新检查器。

版本 2 的[审查验证记录](../verification/measurements/20261023-environment-review.json)保留清单哈希、负向检查和完整 TLC／Verus 重跑结果；它与部署性能测量分开记录。

新增[按层查询验证记录](../verification/measurements/20261024-environment-layers.json)覆盖 19 个组件层、38 条契约及 34 个消费者，并核对 73 种既有报告选择的依赖内容未变。本轮单独重跑十九个环境对照和 Verus 正向验证（115 项通过）；生产代码、环境契约和模型源码未改，没有将既有完整测试结果记为本轮重跑。

可以只运行环境对照，报告仍保留被消费的算法与完整环境输入：

```sh
python verification/runners/check_tla.py --case environment_sync_lies --case environment_wal_forgets \
  --case environment_lock_not_serial --case environment_visibility_bound_broken \
  --case environment_reactivation_lock_not_serial --case environment_discovery_open_lies \
  --case environment_directory_barrier_lies --case environment_file_sync_lies \
  --case environment_namespace_identity_lies --case environment_truncate_loses_prefix \
  --case environment_gc_parent_sync_lies --case environment_preparation_unlink_lies \
  --case environment_handoff_stage_lies --case environment_consumption_edges_lie \
  --case environment_workflow_edges_lie --case environment_local_mutex_grants_busy \
  --case environment_handoff_receipt_lies --case environment_queue_record_early_ack \
  --case environment_queue_record_receipt_lies --case environment_range_authentication_lies \
  --case environment_typed_decode_lies --case environment_read_attempt_visibility_lies \
  --case environment_lookup_bound_lies --case environment_offline_open_lies
```

二十四个 TLC 对照只修改**环境源码**，算法源码保持相同：

| 被削弱的环境保证 | 应失败的性质 |
|---|---|
| fresh open 将仍存在的共享标记错误报告为缺失 | `NoOwnedDeletion` |
| 超过名称 lookup 尝试界限仍虚假缺失 | `NoFailureWithinContract` |
| 目录 sync 报告成功，但删除没有持久化 | `DurableCompletion` |
| 恢复可以丢掉已经持久化的 completion | `AcknowledgedDurable` |
| Catalog 访问绕过 scan 到 tombstone 的串行化 | `NoDanglingReferences` |
| 恢复的重新检查到 owner append 之间允许 peer 修改 | `NoDanglingRestore` |
| 超过 `VisibleBy` 仍能打开不完整视图 | `NoFailureWithinVisibilityContract` |
| 已启用 GC 的池，首次 fresh open 虚假报告 NotFound | `NoLostAcknowledgedData` |
| 目录 sync 成功却未持久化新建的子目录项 | `NoLostAcknowledgedData` |
| 完整文件 sync 成功却未持久化其数据 | `DurableVerifiedReturn` |
| 别名解析返回原始名称而非共享身份 | `NoLostOwnedData` |
| 成功 truncate 错误地截掉已确认前缀 | `PrefixPreserved` |
| GC 父目录 sync 成功却未持久删除，随后允许写 receipt | `ReceiptIsSafe` |
| 准备失败的 unlink 报成功，但私有文件仍存在 | `NoAbandonedPrivateFile` |
| Publication staging 报成功，但没有建立根 owner | `CompletedHandoff` |
| Manifest 展开成功但遗漏真实依赖 | `ConsumeComplete` |
| 工作流消费图遗漏 payload 依赖 | `ReleasedHasRecipe` |
| 本地 mutex 在外层 writer 持锁时仍发放 token | `ExclusiveAccess` |
| 提交后清理查询谎报已经有持久释放回执 | `CompletedConsumption` |
| 队列大记录的 reader 尚未读完就确认释放 | `NoLostReaders` |
| 队列大记录恢复查询谎报已经有持久释放回执 | `NoUnconsumedStaging` |
| Chunk 认证把不同的实际／预期字节序列判为一致 | `NoCorruptReturn` |
| typed 解码把调用方字段冒充为已认证 envelope 字段 | `ReturnedMatches` |
| 描述符复用模型在可见尝试界限后仍打开缺失视图 | `NoFailureWithinContract` |

报告将其标为 `environment_negative`，与算法变异分开。出现预期反例说明定理依赖该契约，不表示 JuiceFS 实际违反了它。例如串行化失效，既可能来自锁服务错误，也可能来自实现绕过正常锁；模型本身不能替我们归因。

发生问题时按以下步骤审查：

1. 保存操作／receipt ID、观察到的字节和顺序、实际二进制、算法／配置／环境哈希及部署设置。检查实际加载的 native 文件，不能只看包版本号。
2. 判断该观察是否被环境模型允许。如果允许但规范失败，检查算法／性质；如果模型允许正确响应而代码违反规范，检查实现对应关系；如果观察被排除，先核对部署文档契约：可能是环境模型过强或遗漏，也可能是部署违反了前置条件。不能凭模型全绿直接归因。
3. 将具体观察保留为回归，并在有依据时扩展环境行为，重跑该契约的消费模型与负向对照。解释状态数、前提和结论的变化；不能为了让证明通过而随意加强假设。

### 故障审查记录

每次审查可以在 issue 或实验记录中填写下表。尚不能确定的项保留为“未知”，并列出能够区分候选原因的下一项检查；不因模型通过就默认环境有错。

| 记录项 | 必须说明的内容 |
|---|---|
| 审查边界 | 组件层、消费模型、`ENV-*` 编号，以及使用的是 L0–L5 中哪种证据 |
| 现场观察 | 操作及返回顺序、已观察到的副作用；区分实际返回值、推断状态和没有观察到的状态 |
| 可复现身份 | 二进制与模型／配置／环境哈希、部署设置、原始日志或反例路径 |
| 允许行为 | 现有环境是否允许该观察；对应的环境操作／前提，以及把现场执行映射到模型步骤时仍未知的部分 |
| 责任与影响 | 保证提供方、straw 适配义务、受影响消费者；安全性与进展结论分别记录，不把二者一起判失效 |
| 结论与后续 | 算法错误、适配错误、环境模型遗漏／过强、部署不满足契约，或尚未归因；列明支持证据、回归和需要重跑的消费者／对照 |

以 GC 为例，`unlink` 成功后目录同步报错、重试时名称仍缺失，是 `ENV-DELETE-FAILURE` 允许的行为。如果实现因此跳过目录同步并写入回收凭据，应检查适配顺序；这段观察本身没有违反同步契约。若怀疑成功的同步没有持久化删除，还需先核对被同步的目录身份、操作先后和崩溃恢复证据，再判断 `ENV-SYNC` 是否失效。GC 永久不再被调用则涉及 `ENV-GC-RETRY` 的进展前提，本身不能推出删除安全性失效。

性能归因也遵守这条边界：`ENV-CATALOG-SERIAL` 要求互斥和新鲜状态，不承诺获取锁的耗时；`ENV-FAIR` 是定性的模型进展前提。恢复探针现分别记录候选检查、校验、最终 catalog 访问、append 与 sync，并单列实验用的 marker 写入和暂停。[阶段计时对照](../verification/measurements/20261023-reactivation-timing.json)没有复现此前的 8.68 秒样本，因此该异常仍未归因。最终 catalog 时间同时包含锁、刷新和授权适配，不能仅凭这一跨度认定锁服务慢，更不能据此修改 `VisibleBy` 或加入硬延迟保证。

[小切片分阶段测量](../verification/measurements/20261024-read-latency.json)另把一组慢请求定位到 `open64` 调用中：两次打开的耗时占 JuiceFS 最慢样本总时间的大部分。此证据归入 `ENV-PERFORMANCE`，不修改 `ENV-VIS` 或 `ENV-FAIR`；调用内等待仍可能包含内核、调度、网络和服务端行为。模型中的抽象 Open 动作与真实 syscall 完成之间仍需适配／进展对应，测到某次成功返回不是所有调用最终返回的证明。复用描述符的候选优化必须同时审查“同次尝试的绑定／认证”与“失败后的 fresh open”，不能根据平均成本省掉后者。

主要剩余缺口是 JuiceFS 内部、网络／锁服务、数值时钟和完整 Rust 恢复流程的组合模型。目前的证据支持审查已声明契约和具体执行，不代表已经建立这个统一模型。

## 预读环境与缓冲策略

`ENV-READ` 新增 `EnvReadChunkSizes`：一次成功的非 EOF refill 可以返回请求上限与快照剩余范围内的任意正长度片段。`CatalogReadBuffer.tla` 消费这一响应域；预读的文件游标并不表示这些字节已经校验或 apply。64 KiB 容量、每次 refresh 丢弃 buffer 和追加前 seek 是 straw 的策略；文件 clone 共享游标、读取响应及 seek 语义属于标准库／OS 适配边界。有限模型的一次失败和小字节长度是搜索界限，不是环境必须只失败一次的契约。没有从这个模型推出任意字节 replay、持久前缀和多客户端的统一一致性证明。

## 首次 GC 发现与池模式

`ENV-GC-DISCOVERY` 单独区分 stat 与 fresh-open 的发现响应。stat 可以陈旧或报错；旧代码把它们降为“没有 GC”，使 retain 误报成功。新实现用 fresh open，并在同一 Config 的 clone 之间保持 GC 要求，错误不会变成关闭 GC。自动发现成功后，打开缺失的 catalog 会返回错误，不能重新创建空日志。显式初始化 GC 池仍允许创建首个 catalog；这不构成 catalog 丢失后的恢复保证。

仍未消除的环境边界是**首次** fresh-open NotFound：自动兼容 legacy 模式依赖它确实表示未启用 GC，且池模式在参与者运行前确定。仅改变这个环境响应即可得到数据丢失反例。显式 `online_gc=True` 可避免以初次名称缺失决定 legacy 模式；它不证明锁、目录同步或整体存储服务。运行中把活跃 legacy 池切换为 GC、历史数据迁移和 atomic clone 状态到真实执行的细化都未证明。

`GCDiscovery.tla` 是这一边界的消费模型，catalog retain 被抽象为原子持久动作。它不能借用其他模型的绿色结果来宣称已经组合证明。Verus 的实际决策函数接收抽象 Observation；File::open 的真实性和 I/O 错误分类仍需适配层审查。四机错误注入不是部署自然出错的证据。

## 名称可见不是目录持久凭据

`ENV-DIRECTORY` 单独描述目录名称与父目录项持久性的差异，包括创建／同步失败留下的前缀；消费模型为 `DirectoryDurability`。父目录屏障只覆盖该目录里的项，文件或叶子目录同步不能证明祖先名称已持久。实现现在对已存在名称也重新完成整条实际祖先链。目录身份、别名和挂载稳定、祖先直到根都能打开同步，以及路径／I/O 适配正确仍是前提。详见[证明边界](FORMAL_VERIFICATION_zh.md#创建目录失败屏障与发布)。

### 冲突重试与内部 publication 所有权

`ENV-PUBLICATION-RETRY` 允许调用者用同一身份、更改依赖集合重试；拒绝不代表之前的副作用被撤销。`PublicationRetry.tla` 从“内部 hold 已持久化、发布中断、原 source 已释放”开始。这不允许调用者在任意 API 报错后释放 source：初始状态已经假定保护建立成功。其他客户端可在 hold、拒绝、正确重试和 staging 交接之间执行 GC，但不能修改其他 writer 私有的 `publication:*` owner。

环境提供完整、不可变的 pack 闭包及串行持久 catalog 动作；进展另外要求调用者正确重试和弱公平性。straw 的义务是保留先前尝试的根，先 stage 完成结果，再释放 operation holder。这些是算法责任，不是文件系统承诺。放弃发布可能无限期保留根。模型没有从字节 replay／锁推导 catalog 原子性，也没有证明任意发布恢复或跨进程幂等性。

实际 `holding.rs` 并集循环另有运行时前提：`environment.rs::string_order_contract()`（`ENV-STRING-ORDER`）要求 String 排序满足 vstd 的有序 key 契约；这里没有证明该定律。只把环境前提削弱为 true 的对照会使实际循环证明失败；两个算法对照分别丢弃旧根、忽略新根。在库契约下，证明覆盖任意输入大小的精确并集；闭包解码、集合转换、catalog append 和调用点交接仍未证明。观测到 hold 与模型不一致时，应先独立审查这些适配边界，不能直接归因于存储持久性。

### Publication 完成与可回收性

`ENV-PUBLICATION-FINALIZE` 提供已持久化、登记为 open 的文件、完整 staging 根、串行 catalog 动作，以及遵守协议的 GC／peer。stage／seal 报错后，持久布尔状态可能保持原值，也可能已经生效；`EnvironmentPersistence.tla::EnvUncertainBoolean` 独立描述这个选择。模型表示串行恢复后观察到的持久结果，尚未从原始 frame、真实锁或文件系统响应时序推导它。

straw 负责先 Stage 后 Seal，独立文件仅在两步均成功后确认发布；packed 发布只 stage，保持可写直到显式关闭／轮换。这些是算法选择，不是环境承诺。最终完成还需要调用者重试、I/O 最终稳定和弱公平性。引用到文件的对应、多文件 record_set 组合及字节适配仍需单独 review。删除在这里是上层 GC 动作，其下层契约由删除／receipt 模型承担。历史中被放弃 writer 的 open 项不能仅凭 `.sealed` 后缀或过去 API 成功返回就推断为可安全封存。

### 完整文件重试与同步认知

`ENV-PENDING-PUBLICATION` 区分“同步成功返回”和“同步报错但副作用可能已持久化”。`PendingPublication.tla` 从完整 frame、open 登记及精确私有文件描述开始；存活 writer 可以收到其他身份、重试原请求或遇到持续损坏。`ENV-SYNC` 提供文件屏障，`ENV-AUTH` 提供 checksum 检测；文件屏障成功不等于随后最终名称的目录项已持久化。模型尚未组合路径查询／rename 错误、崩溃／进程重启或所有权交接。

在同步前保存描述、拒绝其他身份、重试未知结果的屏障是 straw 的责任。独立文件不可变时，可沿用先前成功返回的文件同步，但重试仍需校验字节。抽象进展依赖动作可执行、调用者重试、I/O 最终稳定和弱公平；遇到损坏可报错结束，不保证原路径已经消失后仍能成功恢复。实际适配保留 pending 并返回这些错误。

实际泛型 `pending.rs` 函数证明描述不变、同步认知只由成功响应建立；不证明文件系统真的满足响应契约，也不证明外围赋值先于 syscall。观测违反模型时应审查适配层：此前独立文件路径即使 fsync 完全正常，也会因保存描述过晚而重复写入。

## 命名空间契约与实现边界

`ENV-NAMESPACE` 要求在稳定池根内真实解析路径；普通组件或 NotFound 响应不能隐藏符号链接。相关别名和前缀绑定必须在缓存使用及后续 catalog replay 期间保持稳定；原先缺失的私有前缀允许创建为普通目录／文件。允许删除数据文件，保留的悬空别名仍需指向同一已回收路径。移除或重定向历史日志仍依赖的别名不在契约内。

straw 必须在 owner／pack／tombstone／receipt 各边界归一化、拒绝越界路径，并在 replay 时保持这一关系。本次别名数据丢失发生于正常文件系统响应下，定位到 straw 适配层，不能据此归因 JuiceFS 一致性。两个已证明的缓存函数只保证查询和绑定保留，不代替解析器及适配层调用证明。

这里统一的是解析后路径身份，并非 inode 身份：bind mount 别名未统一，hardlink 名称仍分开，额外未登记链接不会被回收。缓存路径／前缀及永久历史可能持续增长。解析器的 40 次悬空链接 fallback 展开限制是算法 guard，不是实测文件系统常数，也不限制内核总遍历次数。可用 `python verification/runners/environment_manifest.py --contract ENV-NAMESPACE` 对照[精确二进制测量](../verification/measurements/20261023-path-identity.json) 独立审查。冷重放开销单独记录，不进入安全性前提。

## 已记录身份与 producer 义务

`NamespaceEncoding.tla` 将 writer 归一化和 replay 消费 `path_encoding=1` 分开。**先归一化全部四类路径字段，再写标记**是 straw 的算法义务，不能算作文件系统替它保证的行为。Replay 依赖合规 catalog producer（`ENV-PEERS`）及稳定路径身份（`ENV-NAMESPACE`）；缺失标记不保证已归一化，未来或畸形编码不能当作版本 1。

实际 Rust `identities::encoding` 已对所有 `Option<u64>` 验证。JSON 到 option 的解码、归一化循环、标记位置、字节 framing 和 replay 分发仍是适配层义务，由测试连接。模型探索一个含四类字段的事件，弱公平下条件完成，尚未细化全部实际历史，也没有证明完整 writer Rust 实现。真实标记只消除重复 readlink 工作，不加强文件系统一致性、不证明路径解析、不限制 catalog 内存，也不认证任意写入者。时间和拦截到的 API 调用次数与这些前提分别记录。

## 未完成写入、截断与进程边界

`ENV-INCOMPLETE` 描述准确保存的原长度、私有路径独占及 truncate 响应；调用顺序仍由算法决定。Packed 模式只能丢弃该起点之后尚未发布的尾部；独立模式只删除从未暴露引用的私有 `.partial`。截断／删除成功后仍需文件／父目录屏障，之后才能重用 writer，或将从未暴露的最终名称 seal。失败可能已有副作用，描述必须保留；seal 已持久生效后 peer GC 产生的永久 tombstone 同样满足该名称已退役的条件。

这里的存储崩溃动作保留客户端清理描述，模拟存储恢复到持久长度／名称。它**不是 writer 进程重启模型**。进程死亡后描述丢失及旧版本留下的垃圾仍可能安全泄漏；新名称登记到 open／packed seek 之间的失败现在另由下述 `ENV-PREPARATION` guard／模型覆盖。没有基于超时自动封存遗弃 writer；也未证明 offset 捕获、PID 检查位置或完整 I/O 分发。实际 generic controller 保留描述的证明不证明路径或长度取值本身正确。

Linux 的 [truncate 语义](https://man7.org/linux/man-pages/man2/truncate.2.html)与 [fsync／目录屏障](https://man7.org/linux/man-pages/man2/fsync.2.html)支持接口契约的来源；失败副作用是模型保守允许的行为，不是每次错误必然如此。四机真实 syscall 的前／后 EIO 注入检验适配层；未模拟真实掉电，不认证整个部署的持久性。[测量与边界](FILESYSTEM_zh.md#未完成写入与清理重试2026-10-23)单独记录，不能把其 p99 作为保证。

## GC 父目录与尚未满足的进展前提

`ENV-GC-PARENT` 允许 catalog 中已封存的空 reservation 尚无父目录：历史 writer 先登记，目录创建失败后仍可正常关闭／封存。这是守约旧客户端就能留下的状态；当前新名称的 setup 清理会先修复父目录，GC 仍需兼容历史状态。旧 GC 永远只尝试打开该目录，即使错误期已结束也无法完成。此前删除活性模型的“同步动作最终成功”不能替代这个实际可执行性义务。

新的 `GCParentRepair.tla` 将打开、缺失、幂等创建、同步、receipt 和重试分开。NotFound 可以来自实际缺失，也可以是故障期内对已存在目录的错误响应；因此不能直接跳过屏障。实际 GC 对缺失响应调用既有 create／同步 helper，其他错误传播。创建或屏障失败后仍保留删除义务；新客户端再次看到目录存在时仍同步删除。成功创建后 syscall 返回 EIO 也可能被标准库的“目录已存在”检查协调为成功，之后仍须完成同步，四机注入轨迹单列了这个情况。

这里依赖既有稳定私有命名空间：补建的是从未承载该文件的缺失前缀，或创建操作对现有目录保持幂等；不允许把已有目录身份替换后再重绑定回来。需要在缺失处有创建权限。模型证明 receipt 前文件删除持久，**不证明空占位目录本身最终持久存在**，也没有把新增目录动作机械组合进 GC 的无界 Verus 活性证明。实际 `gc_io::classify`／`sync_parent` 分派及 `directory_io::sync_all` 祖先屏障循环已经证明；ErrorKind 的原生映射、路径枚举、句柄与聚合 repair 持久性仍需单独 review。

## 新 publication 的准备阶段与清理义务

`ENV-PREPARATION` 把登记／创建／打开与实际写入分开：报错可能留下 open catalog entry 或空私有文件。新名称唯一、unlink 缺失可信、路径稳定及 catalog replay 串行是提供方契约；**登记前保存描述**、错误后保留、修复／同步父目录是 straw 的义务。不能给既有 append pack 随意使用零 offset。

新 guard 只退役新私有名称；打开并取得 offset 后，实际泛型 `capture_prefix` 保留同一身份，再切换到正常写入回滚。既有 pack 的准备失败保持原前缀。即使登记原本未生效，清理成功也可能创建一个 sealed 空 entry，随后由 GC 退役；它增加元数据工作，但不会永久保持 open。清理失败则阻止重用，等待存活调用方重试；peer tombstone 满足此前 seal 响应不确定的退役条件。

`PublicationPreparation.tla` 检查描述义务、seal 前持久删除、不遗弃私有名称及条件进展。父目录修复／catalog 动作仍是抽象效果，尚未机械组合下层模型。`preparing`／`capture_prefix` 有实际 Rust 证明；调用位置、真实 seek 返回值、syscall 身份和完整适配层没有。进程死亡、遗弃的依赖 hold 与历史残留仍是独立缺口。见[精确二进制记录](../verification/measurements/20261023-publication-preparation.json)。

## 完整文件到成功返回的交接

`ENV-HANDOFF` 将调用方行为与算法义务分开：原操作报错后，另一 ID、另一 API（即使 ID 相同）或冲突内容都允许出现。straw 必须保存原文件描述，直到 staging、必要的 standalone seal、外层 manifest staging 与内部 owner 释放都成功；不能把“调用方不会插入其他请求”作为环境保证。

这次旧版反例是在允许的环境内发生的实现错误：staging 已生效但响应报错，随后另一 publication 覆盖重试缓存，原请求再写一份，旧文件的 owner 留存。修复让完整文件描述绑定 API／ID 并持续处于未完成状态，提前拒绝其他请求，避免创建它们的依赖 hold。GC 允许与重试交错；`seal()` 不能丢弃描述。此前已成功返回、后来被释放／回收的缓存重放，不重新产生一个无法完成的 guard。

`PublicationHandoff.tla` 分开检查描述未遗忘、未返回文件未丢失、交接成功时根已 staging／内部 hold 已释放，以及稳定存储、弱公平和原请求重试下的完成。环境反例只让 `EnvStagedRoot` 在成功返回时为假；它说明证明依赖 catalog 契约，不说明部署中出现了这种行为。模型从数据已持久化开始，尚未与 pending 文件、完整依赖图、目录和进程恢复机械组合；库成功返回不表示远端调用者收到 RPC 响应。显式 close／放弃、完整文件形成前的依赖 hold，以及崩溃 writer 的恢复仍是独立缺口。

可单独导出这层的审查材料：

```sh
python verification/runners/environment_manifest.py --contract ENV-HANDOFF
python verification/runners/environment_manifest.py --model PublicationHandoff
```

[精确二进制与验证记录](../verification/measurements/20261023-publication-handoff.json)保留原始测试、四机响应丢失对照和 100 GB 单次重试。测量不新增环境必然满足的延迟界限。

## Staging 释放后不再依赖 manifest 存活

`ENV-CONSUMPTION` 允许释放已经持久化、响应却丢失；GC 随后删除无 owner 的 manifest，调用方仍可重试或重新打开。原实现每次重新读依赖图，因此即使文件系统已恢复、调度公平也可能永远无法完成重试。这属于模型允许的执行中的实现缺口。

实际 catalog 现在在释放 staging 的同一 frame 里保存精确直接边，重新打开时恢复该图；不确定 frame 丢失时，图和 owner 状态一起重建。不能只因根 owner 已消失就返回：根文件被 GC 后，仍有其他 owner 保护的依赖可以重新获得 staging，重试仍须沿已记录的边消费它，同时保留其他 owner。直接边检查要求每个子节点都有已有或同 frame 的定义；这只证明结构闭合，**不证明边与 manifest 真实内容相符**。

模型将每个节点的可读性分别抽象，未组合实际 shared pack、字节认证、catalog／WAL 适配和完整 GC。旧日志的释放若没有记录依赖图，且 manifest 已被回收，新版无法凭空恢复；若 manifest 尚在，新版可读取并记录。图和日志随唯一引用增长，尚无 compaction 或内存上界证明。延迟测量不能补上这些契约。

```sh
python verification/runners/environment_manifest.py --contract ENV-CONSUMPTION
python verification/runners/environment_manifest.py --model StagingConsumption
```

[测量和证明记录](../verification/measurements/20261023-staging-consumption.json)分别保留算法反例、环境反例、metadata 成本和四机重试结果。

## 未完成准备阶段的依赖 owner

`ProvisionalHold.tla` 消费 `ENV-PREPARATION`、`ENV-PUBLICATION-RETRY` 和 catalog 持久响应契约：hold／release 报错可以已生效。失败准备的依赖仍需保留供原请求重试，包括调用其他请求之后；显式 `seal()`／`close()` 才表示放弃这些未形成完整 publication 的准备。straw 必须在发出 hold 之前记住每个 owner，不能用后来的请求覆盖旧身份。开始清理后，释放报错要保留正在处理的身份，下一次操作先完成已开始的清理。

完整文件已交给 handoff 后，其 owner 退出准备集合，不能再被准备清理删除。其他旧准备仍可在 close 时清理；内部 pack 轮换不能触发清理。这些是实现义务，不能靠排除错误输入、其他请求或 close 前的 GC 来满足。

模型区分 hold 失败、完整文件之前失败和完整交接响应丢失，检查一个失败请求之后的另一个请求以及显式结束时的清理。它抽象第二个请求输入仍可用，GC 只检查完整交接的保护，不是任意 peer／图／共享 pack 的组合证明。Verus 的实际 `holding::finish` 只证明释放成功标志为真才清空当前清理描述，否则保持原值；待清理集合、调用位置和 syscall 对应仍未证明。进程退出后的 guard 丢失与历史内部 owner 仍需恢复协议。

内存中记录了清理身份不等于 hold 已持久生效。特别是首次 hold 报错可能完全未生效；调用方仍需保留源 owner，直到目标保护确已建立。四机的 hold 响应丢失切点位于真实 sync 成功之后，不能推广到任意 hold 错误。

## 跨阶段工作流的环境边界

`SharedRecordWorkflow` 单列消费准备、交接、消费图、reader 响应和 GC peer 契约。ReaderCount=4 对应本轮四机执行；payload／manifest 两节点对应观察到的两个最终文件。吞吐量和分块 p99 只进入测量记录，不进入模型的进展前提。模型的 `ReadResponse` 是整个分片调用的抽象成功／失败，短读和错误都不能授权释放；它没有宣称底层 I/O 会在给定时间返回，也没有组合所有真实重试。允许 reader 失败时，只证明工作流得到安全的失败结果并保留 owner，未证明后续恢复回收。

## 进程内 writer mutex 与 callback 进展

`ENV-LOCAL-ACCESS` 属于运行时边界，与分布式 catalog 锁分开。Rust [Mutex 契约](https://doc.rust-lang.org/std/sync/struct.Mutex.html#method.try_lock)提供排他的 guard 和非阻塞 `try_lock` 响应。Python fault callback 可以请求同一 writer，或等待另一调用方请求它；writer 必须等 callback 返回才能继续。如果直接假设 callback 总会返回而不检查这个依赖，就会隐藏实现死锁。

`WriterAccess.tla` 显式保留这个依赖。旧阻塞策略即使满足弱公平，也有无限等待执行；修复策略返回 busy、保持 publication 状态，让 callback 返回，随后才允许另一次访问。后续尝试观察到 poison 时仍拒绝。仅把 `EnvironmentLocalAccess.tla` 改成“外层持锁时仍发出 token”，会违反 `ExclusiveAccess`，用于独立审查 mutex 假设；它与算法等待环反例不同。

实际泛型 Rust `access::decide` 证明仅在收到 token 且未 poisoned 时保留并返回该精确 token，busy／poisoned 输入均拒绝。标准 mutex、结果到 token 的映射、GIL／FFI 和完整适配调用位置尚未证明。其他 catalog／coordinator／journal 锁、任意 callback 的终止仍不在该模型内。探针的两秒 watchdog 只是观察窗口，随后显式终止自己启动的子进程，不是环境延迟上界，也不是 JuiceFS 慢的证据。

## 已提交事务的清理与后来的 staging

`ENV-COMMIT-CLEANUP` 是队列消费的下层契约，其中提供保证的部分也包括 straw 自己的 catalog／恢复代码。环境允许“WAL 已确认 → staging 释放报错 → 同 ID 重试或新进程恢复”，也允许释放已持久化但响应丢失。publisher 可以在此后重新 staging 完全相同的不可变引用。

算法必须把 `StagingHandoff` 意图与接收放在同一 WAL frame，把释放、依赖图和 `consumed_handoffs` 回执放在同一 catalog frame。恢复和返回缓存结果前完成未确认工作；已完成的 ID 只查询回执，不再次释放。未完成项继续保留其队列 owner。显式 `release_publications` 仍按请求释放整个依赖图，与队列提交的单次交接语义分开。

这轮复现的泄漏属于**算法／适配错误**：环境行为已经在允许范围内，但实现漏记、漏重试后续清理。仅在下层确实错误报告或丢失了已确认回执时，才是这项契约的提供方失效；仍要继续区分 catalog replay 与存储持久性。`CommitCleanup.tla` 分别检查算法变异和只改环境的虚假回执，不能用同一个“测试失败”标签代替归因。

模型从已经 retain 的一个根开始，允许清理失败、响应丢失、恢复、后来重新 staging、独立 reader 和 GC。未组合任意依赖图、多个队列、WAL 接收之前的失败和完整进程崩溃。Verus 只证明实际 pending 移除和回执集合更新函数，确认标志、ID／字节绑定、调用位置和整个 I/O 适配尚未证明。旧 WAL 没有标记时，无法安全判断 staging 是漏清理还是后来建立，升级不会自动清除这种历史歧义。

## 队列恢复、远程读取与 GC 的组合边界

`QueueRecordWorkflow.tla` 把已接收但未完成的 staging 清理、已完成交接历史、input lease、取消、恢复、reader 确认、GC 和回收后的再次恢复连起来。`EnvironmentReaders.tla` 单独表达 `ENV-PEERS`：只有实际读取已结束的调用方才可确认 reader。**执行 lease 被取消／过期／换 epoch，不等于读取已结束。** 这一前提由应用／supervisor 提供；straw 必须持久保留相应 reader 根，直到收到符合身份的确认。

因此两类反例需要分开 review：取消或恢复时遗失 reader 根是算法错误；调用方提前确认则违反 reader 生命周期契约。环境对照只改变确认资格，会导致 `NoLostReaders` 失败。真实探针由 supervisor 验证远端进程成功退出、分片身份和字节覆盖后再确认；通过 stdin 确认本身不构成生产分布式 fencing 或 RPC 证明。四台机器的 coordinator 恢复严格串行，前一个进程已退出后才允许下一个 owner 打开。

正例采用 1／2／4 个 reader，并将实测负载的 0／128／1024 条已完成交接历史放入配置。每个历史步是一次已经成功的抽象交接，保存独立 ID 的持久回执；reader 恢复与 GC 后恢复都逐项检查这些 ID。**没有模拟历史 JSON 字节、内存分配或持久化耗时**；对应的时延与内存只作为测量。读成功后才移除 running reader，确认后才移除持久根。允许读失败的配置可以终止为 failed 并保留数据，不保证错误后最终 GC。实际 syscall／网络请求能否返回、真实调用位置、锁和 WAL／catalog 细化仍未证明；弱公平与实测 p99 都不能替代这些义务。

可分别审查环境与组合模型：

```sh
python verification/runners/environment_manifest.py --contract ENV-PEERS
python verification/runners/environment_manifest.py --model QueueRecordWorkflow
```

## 范围读取的响应、认证与复制

`RangeRead.tla` 将 `ENV-READ` 的完整／短读／错误响应与 `ENV-AUTH` 的认证契约分开。新模块 `EnvironmentAuthentication.tla` 只要求传入的实际序列与预期序列一致才认证成功；实际 SHA-256、预期 checksum 与不可变引用的绑定仍未证明。其他模型对 `ENV-AUTH` 的既有抽象不因此自动获得细化证明。

straw 必须认证全部被触及的 chunk，再返回请求范围内的精确字节；请求范围外、但同属一个 chunk 的损坏也必须使读取失败。1 MiB scratch、chunk 切分、复制偏移、checked 计数以及“发生错误不返回内部已复制字节”均是算法义务。仅用重复 pattern 的全 chunk 请求不能充分区分偏移错误，因此新探针使用全局 word 位置相关的数据，并额外核对返回字节。

模型穷举配置长度内所有合法区间，包括空区间，检查精确输出、认证覆盖、checked 覆盖、损坏拒绝、失败不返回、无故障不误报及 scratch 上限；弱公平下每次尝试结束。七个正例、九个算法对照和一个只改认证环境的对照见[记录](../verification/measurements/20261024-positioned-range.json)。小整数用于区间组合搜索，100 GB、256 MiB 和测得的 p99 不直接填入模型作为保证。

这是一份在引用／envelope 已验证、数据仍有保护时的单次读取模型。实际 seek/read 绑定、无限 syscall 等待、共同重试预算、并发 GC 和完整 Rust 循环的组合证明仍缺失。实际 `bounds::clip`、`next_buffer` 和 scratch 容量函数的 Verus 证明保持独立；测量程序本身不是经过 Verus 验证的生产代码。

## Tensor 校验粒度与环境边界

`TensorGranularity.tla` 将新 producer 的固定 chunk 策略与旧记录的编码粒度分开检查。固定 4 MiB 是 **straw 的算法选择**，不是 JuiceFS 必须提供的行为，也不是延迟上界。模型只枚举长度、区间、摘要数量及认证范围的算术；不发起 I/O，不为环境添加 4 MiB 原子读取假设。

`ENV-AUTH` 继续要求读取所用的元数据确实绑定到不可变记录；实际 reader 必须使用已编码的 chunk 大小，不能把旧记录的 checksum 按新粒度解释。`ENV-RUNTIME` 继续涵盖整数／库模型与内存分配的可信边界。实际 `bounds::authenticated_span` 的 Verus 合约证明所有 u64 输入的合法性判定、空区间、覆盖、尾部裁剪及额外认证字节上限；Python producer、hash 列表生成及整个 I/O 循环没有因此得到 Verus 证明。

审查时可区分：新 producer 自行增大 chunk 而违反认证量上限，是算法策略问题；旧记录按编码粒度多读，是已知格式兼容成本；checksum 与实际记录绑定错误，须继续定位适配或 `ENV-AUTH` 提供方；相同字节量耗时不同则属于 `ENV-PERFORMANCE` 的部署测量。元数据增长、重试乘数与物理后端读取量都不在单次 payload 认证量界限内。见[粒度测量记录](../verification/measurements/20261024-tensor-granularity.json)。

## Typed 引用与 payload 分配的边界

`ENV-TYPED-DECODE` 明确要求 adapter 忠实提供已认证 envelope 与调用方期望的 dtype、shape、length。提供方是 straw 的 Python／PyO3／JSON／索引适配层；这项前提失败不能直接归因于 JuiceFS。`EnvironmentTyped.tla` 同时允许 payload 分配返回成功或失败，不建立数值内存容量保证，也不包含 metadata 分配失败、进程 abort 或文件系统请求永不返回。

`TypedRead.tla` 分开表示 metadata、字段比较、payload 分配／读取和返回。错误描述必须在 payload 工作之前被拒绝，空切片也不能无条件成功。跳过比较、延后比较、空请求提前返回、错误拒绝正常请求、对固定不匹配无限重试，是算法变异；只让环境提供伪造的匹配字段，是单独的环境对照。弱公平只用于模型动作最终执行，不把 I/O 耗时或 p99 纳入前提。

真实 Rust `typed::matches` 证明字段相等的充要条件，包括任意长度 shape vector；JSON 解码、Python shape 乘积检查、调用位置和完整 I/O 细化仍未证明。本轮回归与四机测量检查这些适配边界的具体执行，见[记录](../verification/measurements/20261024-typed-preflight.json)。元数据仍必须被读取、认证和解析；“提前拒绝”只承诺不为已识别的 typed 不匹配分配或读取 payload。

## 一次读取的描述符与重试边界

`ReadAttempt.tla` 将一次调用分成 Open、Index、Envelope、Payload、关闭失败尝试和 Retry。`ENV-VIS` 仍决定 fresh open 的视图，`ENV-AUTH` 仍要求缓存索引已经认证并绑定到同一不可变引用；不新增“看到索引就必然看到 payload”的假设。`ENV-FAIR` 只使抽象步骤推进，不保证真实 syscall 返回或给出耗时上界。

同一次尝试复用描述符是 straw 的选择。`SingleOpen` 限制一次外层尝试至多成功取得一个数据文件描述符；它是可检查的开销性质，不是文件系统契约。模型的 Open 抽象了成功取得描述符，不计底层失败的 open syscall。实际 `open_committed` 对 NotFound 至多调用五次 File::open，中间等待 10、20、40、80 ms；这条名称查找重试及其他 open 错误未包含在本模型中。这 150 ms 的等待预算独立于 extent 可见性重试的 3.85 秒，二者都不是 API 截止时间。旧的重复打开变体违反此性质，不代表旧实现因此返回错误数据。失败前后继续检查 `ReleasedBetweenAttempts`、认证后返回和可见尝试界限内的成功；允许超过预算和持续损坏最终报错。模型覆盖普通读取、冷／热 session，但不包含任意调用历史、实际缓存 key 比较、并发 GC 或完整字节解析。

Rust 的 `inspect_opened` 将本次文件与已检查索引一起交给 frame 读取；`Reader.last` 仍只有引用／索引值。实际 `bounds::frame_position` 对所有 u64 输入证明：extent 地址可表示且完整 frame 位于 extent 内时，返回准确的绝对起点，否则拒绝。它不证明传入数值绑定了正确文件，也不证明 OS 支持任意 u64 seek。真实 File 传递、Drop、打开次数和 fresh-open 重试属于尚未整体细化的 I/O 适配；原生回归与调用计数提供具体执行证据。

## 名称 lookup 与数据可见性是两套预算

[`EnvironmentLookup.tla`](../verification/storage/tla/EnvironmentLookup.tla) 单列 `ENV-LOOKUP`：命名空间解析完成后，一个**已返回的**只读 open 可以得到描述符、NotFound 或其他错误。NotFound 不意味着永久缺失；`NameBy` 是每轮数据尝试内，从第几次名称查询开始保证成功的条件前提，不是毫秒或部署保证。其他错误在任意尝试都可能出现。真实 namespace 查询、错误解释、描述符身份及 syscall 是否返回仍是适配／环境边界。

[`ReadLookup.tla`](../verification/storage/tla/ReadLookup.tla) 将这套响应与 `ENV-VIS`、弱公平组合。实际策略最多五次 File::open，只有前四次 NotFound 各等待 10／20／40／80 ms；EIO、权限等其他错误立即传播。成功打开后的可重试校验失败使用另一组 100／250／500／1000／2000 ms 等待。每次外层重试重置名称查询预算，但不重置外层预算。

对于当前普通读取与冷／热 session 的单描述符路径，组合上限是 **30 次 File::open，显式等待合计 4,750 ms**：`6 × (10 + 20 + 40 + 80) + 100 + 250 + 500 + 1000 + 2000`。这只是算法请求的 sleep 总量；命名空间解析、open、read、seek、校验、分配和调度耗时均不包含在内，不能当作 API deadline。`NameBy <= 4` 与 `VisibleBy <= 5`、无其他错误及无持续损坏共同支持成功性质；任一条件缺失时，有限重试允许失败。

模型把每轮数据校验合为一步，`ReadAttempt` 另检查索引／envelope／payload 顺序。两者不是完整细化或组合证明。`ENV-LOOKUP` 不会自动加强 catalog 初次发现的 NotFound 保证，也不提供 GC unlink NotFound 的真实性。独立审查可执行 `python verification/runners/environment_manifest.py --contract ENV-LOOKUP`。受控注入和自然名称观测分别记录在 [FILESYSTEM](FILESYSTEM_zh.md#名称查询与数据重试的组合2026-10-24)，不能把注入次数当成真实缓存统计。

这里的 open 响应明确位于 **Rust `File::open` 返回处**。[Rust 1.90 的 Unix 实现](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/fs/unix.rs#L1185) 用 [`cvt_r`](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/pal/unix/mod.rs#L310) 在内部重试 EINTR。因此 30 次 File::open 不限制 libc open／内核 syscall／RPC 次数；持续中断也不满足“调用返回”的前提。本轮单独注入有限 EINTR 序列并保留逐调用记录，没有向真实进程发送信号，也没有证明无限中断下终止。

## 离线清理的命名空间与停机前提

`ENV-OFFLINE` 单独声明离线管理工具的环境。离线清理只支持没有 `storage.log` 和 `queues` 标记的单队列布局；其他 checkpoint／队列的根不能仅靠根目录的单份 journal 枚举。Advisory stat 允许陈旧缺失；fresh read-only open 的 NotFound 仍需真实表示标记不存在。其他错误不证明不存在。根 journal、完整认证依赖闭包和稳定命名空间分别依赖 `ENV-GRAPH`／`ENV-NAMESPACE`。

报告产生后、真正清理开始前可以出现新的共享标记或 owner，因此清理必须重新检查。`all_participants_stopped=True` 是调用方提供的停机前提，**不是工具验证了停机或完成 fencing**；最终检查至 unlink／目录同步之间必须保持停机、标记和路径稳定。删除失败可能已经移除了无 owner 文件，不承诺回滚；重新清理可能需要重新生成报告。

[`EnvironmentOffline.tla`](../verification/offline/tla/EnvironmentOffline.tla) 将允许失真的 stat、可信 fresh-open 缺失和删除失败副作用分开。`OfflineCleanup.tla` 检查当前 root 的历史引用、外部共享引用、标记的两次检查及停机要求。模型将完整闭包折叠为一个布尔条件，未机械验证 Python 的 JSON／递归遍历、报告相等比较、路径解析和实际 syscall；它不替代在线 GC 的所有权／receipt 模型。

旧 `inspect.py` 把 `Path.exists()` 的假阴性当作无共享存储的授权。即使所有参与者真的已停止，checkpoint 或其他队列的历史引用仍然有效，旧工具也可能删掉它们。修复改用不读取文件内容的 fresh open，存在则拒绝，非 NotFound 错误也拒绝；`cleanup_orphans` 保留完整重新检查。若 fresh open 自身也虚假返回 NotFound，环境负例仍会违反 `NoOwnedDeletion`。这项修复没有证明 JuiceFS 的 fresh-open 真实性，也不会恢复过去已误删的数据。

可用 `python verification/runners/environment_manifest.py --contract ENV-OFFLINE` 独立审查提供方／实现义务，用 `--model OfflineCleanup` 查看跨层前提。真实私有池的旧／新 syscall 对照见[离线清理记录](../verification/measurements/20261024-offline-guard.json)。它定位的是工具对已有允许行为的解释错误，不是测得 JuiceFS 自然发生了 stat 假阴性。

## 离线引用：文件身份与解析途中依赖

[`EnvironmentBindings.tla`](../verification/offline/tla/EnvironmentBindings.tla) 中的 `ENV-OFFLINE-BINDINGS` 将完整路径解析轨迹与 `ENV-NAMESPACE` 的最终身份分开。合法文件别名可以与真实 pack 使用不同名字；链式或目录别名本身也可能命中清理器的 `*.partial`／`*.sealed` 枚举。链接目标含 `..` 时，途中经过的目录可能不在最终规范路径中，因此只保留文件及其最终父目录仍不够。

环境提供真实的成功 `readlink` 结果和 EINVAL 分类、完整轨迹，以及外部绑定稳定性。**稳定性排除外部重绑定，不预先假设 straw 不会删掉别名。** 模型允许清理算法 unlink 路径，随后检查被接受引用所需绑定是否仍在。实现必须保护解析途中所有绑定，对 partial 候选也执行检查，拒绝未知或越界身份，并 unlink 枚举出的原路径而非其解析目标。成功身份／轨迹缓存只在本次检查内使用；解析失败禁止删除，不能解释为没有所有权。

用 `python verification/runners/environment_manifest.py --contract ENV-OFFLINE-BINDINGS` 或 `--model OfflineAliases` 单独审查。环境负例刻意遗漏中间绑定，触发 `AcceptedReadable` 反例。这是有限轨迹模型，尚未证明 POSIX 解析器；Python 路径展开、图／WAL 解码、报告比较和系统调用对应关系仍未机械证明。硬链接／bind mount 等价与外部重绑定不在现有契约内。检查器的 40 次链接展开上限用于拒绝异常路径，不是可见性或延迟上界。

[四机对照](../verification/measurements/20261025-offline-alias.json)使用接受任务之前创建的合法稳定别名，没有文件系统故障注入。这是允许环境下的实现遗漏，不能归因于 JuiceFS 违反契约。

`ENV-REPLAY-APPLY` 现在将已证明的 catalog 原始历史适配器与剩余 decoder 义务分开。`catalog_target::Apply` 投影实际联合日志历史并保持不变量；`Decoder` 不假定业务语义。原始 JSON／run／路径到操作的含义应与 Source 可见性／sync、外层 catalog 锁／open 分别 review；实际 rebuild／replay／barrier／确认组合已有 Device 下的证明。Journal Target 仍未证明。

`ENV-REPLAY-APPLY` 还要求成功 apply 不留部分记录、所有返回都保持上下文（catalog 确认下限）。真实 catalog Target 已证明；`Apply::recover` 已证明实际重建、字节恢复与确认组合。Decoder::reset 的实际缓存对应仍未证明；外层锁／fresh-open 和 Source 原语语义继续独立 review。

既有 ENV-JSON-ENCODE／ENV-JSON-VALUE-CONSISTENCY 定律现也用于实际 queue journal 编码及成功 replay 观察。straw 证明写前校验及精确事件／payload／frame 绑定；实际恢复前缀选择、原生 File／run 比较与完整业务 replay 仍是自身义务。

`ENV-CROSS-COMPOSITION` 将多请求持久效果抽象与原生实现分开。删除恢复／sync 语义由文件系统提供；完整不可变闭包、queue／staging／holder 命名空间隔离、准确活跃快照、回执恢复及下层效果线性化属于 straw。模型允许 apply／回复丢失和重复恢复，不把两个日志的持久化合成一笔事务。Verus 的名称恢复／删除谓词在共享环境模块，TLA 前提在 EnvironmentCrossComposition；允许丢失受保护持久名称的环境变异在两种工具中均违反保护性质。登记契约不意味着原生对应或活性已经证明。

`ENV-GC-BATCH-EFFECTS` 现将实际 `gc_receipt` 的编码／追加／reclaim 对应与物理删除分开。该函数保持精确规范目标集合及永久 tombstone，但不证明原生 unlink／父目录修复／屏障已经使这些名称持久不存在。目标到物理状态的绑定及已恢复 catalog 上下文，仍须与回执序列化分开审查。

`ENV-JSON-CANONICAL-CONTENT` 单独记录四条固定版本的 `serde_json::Value` 库契约：库自身序列化后成功解析，对应树节点的 canonical 内容保持；array、string 与 unsigned number 构造保持相同 canonical 输入。这依赖排序 Map 和 `float_roundtrip` 配置。不能由 Value 相等或较弱的 JSON 观察模型推出序列化字节相等，正零和负零就是反例。实际 handoff 构造和恢复仍是 straw 义务；有限 codec 回归不能证明 serde 内部实现。
