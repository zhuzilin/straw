# straw 核心实现与证明对应表

[English](VERIFICATION_BOUNDARIES.md)

本表是调用链的审查入口，不是已经闭合的细化证明。`View` 一栏没有覆盖整个对象时，明确写为“部分”；不能从 helper 的后置条件推断调用者的后置条件。[FORMAL_VERIFICATION](FORMAL_VERIFICATION_zh.md) 区分证明、有限模型和执行证据。

0.1.4 冻结新增证明，剩余实现对应与活性义务延期；本次发布的回归覆盖见[范围矩阵](FORMAL_VERIFICATION_zh.md#本次发布的验证范围)。既有证明检查仍为必需。下文各阶段证据保留记录时的输入范围，不能据此宣称更新后的构建已经通过同样验证。

## 实际入口与模型动作

`catalog_decode::decode` 已将实际字段解码与语义谓词连接到完整有序操作；View 使用固定命名空间 map 和语义操作序列。`catalog_frame::Decoder` 已在解析库关系及 Resolver／KeySet 契约下建立原始 payload／run 到操作的语义对应，Target 保持这份业务历史；`LogState::new` 已建立空历史，`catalog_append` 连接输入／run／序列化与追加历史；独立解析值的完整 catalog 业务解释唯一性已在显式库／String key 前提下证明；原生 Resolver／KeySet 与完整事务组合仍未闭合。HashMap 消费迭代单列两个库原语契约。见[构造证明边界](FORMAL_VERIFICATION_zh.md#catalog-字段到有序业务操作)。

`protocol::execute_commit` 是实际四步效果分派器，在 apply 前隔离，只在成功返回后解除。其单事务 View／效果契约与原生 `CommitIo`、完整根集合及 coordinator replay 分开。捕获 unwind 的边界由原生数据丢失回归和有限 `CommitApplication` 模型补充，不能称为 Verus 已证明 unwind 语义。

| 实际入口 | 状态对应／模型动作 | 调用者必须建立的前置条件 | 需要建立的后置条件及现有证据 | 尚未证明的 straw 适配义务 |
|---|---|---|---|---|
| `Store::finish`、`Config::finalize_publication`、`Store::publish_attempt`，见 [store.rs](../rust/store.rs) | 部分：`finalization`／`pending` 控制器；PublicationFinalization、PublicationPreparation | 记录／引用合法，准备阶段持有完整依赖，路径身份稳定 | 返回引用前数据、路径、stage 和 seal 达到对应持久阶段；控制器与部分状态函数有 Verus，外围有故障回归 | 实际写入／目录屏障与返回值的对应；完整闭包、writer 重启；不能直接当作 DurableHandoff 的入口已被证明 |
| `Journal::append`，见 [journal.rs](../rust/journal.rs) | 实际 wal_append／Writer driver；`wal::Append@` → `ProgressView` | 有效 events，未 poison，独占正确日志，cursor 与 sequence 对应有效前缀 | 实际 framing、字节效果派发、错误传播和 sync 顺序已在 Writer 接口下证明 | 实际 Journal State 恢复／追加／隔离已证明；原生 Appender、queue payload 序列化／run 绑定及外围 fd／锁仍未闭合 |
| `Journal::recover` | `wal_recovery` 的实际 Device/Target driver；WalRecovery 的 scan/truncate/sync/seek | 正确日志身份、可读取的前缀快照、独占恢复 | 成功时 transactions 是完整有效前缀，cursor/sequence 在尾部；只允许截去合法不完整尾部 | driver 已证明字节校验、SHA 范围、原始记录顺序及恢复后置条件；实际 journal JSON/event-list 校验与事务重建已证明；具体 Source、完整 Coordinator 业务 apply 和 fd 绑定仍需细化 |
| `Catalog::transaction`、`refresh_with_sync`，见 [gc.rs](../rust/gc.rs) | 实际 `catalog_log::State` 与有类型历史、`catalog_transaction::State` 入口上下文；CatalogReplay；完整 Catalog → CatalogView 尚无 View | 锁授予串行访问，重开的路径仍指同一 catalog inode | refresh 成功后才调用业务操作；索引对应已同步前缀；未知尾部／部分 apply 后重建 | 联合状态转移、有类型历史、部分 apply／rebuild 和实际 replay／barrier／确认组合已证明；实际 catalog_transaction 在独立契约下证明入口绑定／fresh 身份检查、先恢复再回调、末尾 context 校验与精确错误选择；原生锁／open／refresh／context／callback 对应和完整持久历史仍未闭合 |
| `LogState::{replay_record,begin_append,finish_append}`、`Catalog::append_frame` | 实际 `catalog_log::State` 与 `catalog_state::State`；`control_plane::CatalogView` 仍是独立所有权历史模型 | 已验证 frame/event、run identity、路径编码；改变索引前已设置 rebuild | 实际索引等于有类型 frame 历史的顺序 fold；cursor／sequence 与该历史一致；部分失败、rebuild、待完成计划和恢复边界确认已证明 | append_to 已组合 Writer 持久性与确认；解码后的 frame 已绑定业务操作；实际 append 输入／序列化／历史绑定已连接；完整业务解释唯一性已在 Value 观察／String key 前提下证明；路径解析、closure cache 和原生 Writer 对应仍需细化 |
| `Catalog::append_frame` → `catalog_append::{prepare,append}` | 原始 JSON 字段、传入 run、Prepared 字节／操作及 interpreted LogState 历史 | 原历史满足解释关系；Resolver／KeySet、Value 编码库定律与 Writer 契约 | 保持非 run 字段，序列化同一已解码 Value，持久追加后恰好扩展一帧；错误保留原历史 | 相同原始前缀下的完整业务状态等价已证明；原生规范化／命名空间／File 对应仍未闭合 |
| `Catalog::closure_inner`、`retain_many`、`consume_once` | 实际 closure 图／缓存 View；部分 holding/consumption 集合 View；Ownership、CommitCleanup | 认证引用对应完整图，pack 身份准确，锁内刷新完成 | retain 包含完整可达 pack 且拒绝 tombstone；旧 receipt 重放不消费新的 staging | 实际 closure::collect/resolve 已证明完整遍历、缓存保持和 tombstone 排除；实际 closure_node 进一步证明字段选择及完整有序 child／digest 绑定；原生认证 manifest／digest／路径到图及缓存上下文、实际 receipt 身份、原子日志更新到模型动作仍未闭合 |
| `Coordinator::commit`、`open`、`finish_consumption`，见 [coordinator.rs](../rust/coordinator.rs) | 实际 execute_commit 分派器及单事务 CommitView；Completion、CommitCleanup、DurableHandoff；完整 Coordinator 尚无 View | 外部 fencing、合法请求与引用、catalog 的持久 retain 契约 | retain 先于 queue acceptance，退休先于 prune；请求／epoch／receipt 的恢复保持业务与所有权一致 | 分派／错误传播／隔离已在 CommitEffects 下证明；原生 effect 对应、所有请求与重试分支、根集合、JSON apply、完整跨日志细化仍未闭合 |
| `Coordinator::apply` → `queue_task` | 实际 task Value 字段及单条状态转换 | serde 字段／克隆／映射契约；调用者选择正确任务与事件 | 创建、租约／generation、checked retry、终态、失败证据及 progress／yield 更新已证明；局部错误保持任务不变 | 原生缺失目标检查拒绝畸形历史；完整目标分派、reader 根、全局计数、缓存、准入及有序业务 fold 仍未闭合 |
| `Coordinator::apply` → `queue_reader`、`queue_receipt` | 实际 reader 表、submission／commit 和 producer 字段 | serde 相等／字段／映射契约；正确 attempt／receipt 与调用分派 | 追加保留旧根／租约；按序释放仅删除目标并描述错误前缀；submission 先于 commit 追加，producer 只改所选条目 | ReaderRetired 筛选已在新增 retain 基础契约下证明；attempt 新鲜性、完整根／历史／缓存／fencing 仍未闭合 |
| `Coordinator::apply` → `queue_state::{edit,submit,edit_many}` | 整个 queue Value 的精确 task 表查找及统一更新的有序轨迹 | 既有 serde／String 契约；调用者传入匹配种类／参数 | 成功仅改所选 task；局部失败保持传入状态；epoch 批次有精确成功／错误前缀并保持其他根和 peer task | 外层事件选择、epoch／order／缓存及完整混合历史仍未闭合 |
| `Coordinator::apply` → `queue_transition` → `queue_transition_roots` | 整体 queue 的任务／reader／回执组合、精确错误前缀和 progress 根保持 | 既有 serde／String 契约；根推导另需 String 内容身份 | 证明完成／失败／progress 顺序效果和指定 peer／前缀保持 | 外层准入／分派、退休、计数器／缓存、完整 replay 与所有权仍未闭合 |
| `Coordinator::apply` → `queue_admin` | queue 状态及有界 order／epoch；精确提交前缀及 producer／reader／request 修改范围 | 既有 serde／String 契约；容量上界和正确原生调用绑定 | 实际分配／批次／管理状态更新已证明；原生写 WAL 前准入有回归及 mutation 证据 | 完整准入／分派／历史／索引／所有权对应仍未闭合 |
| `CommitIo::{arm_apply,apply,confirm_apply}`／`Coordinator::open` → `queue_application` | 实际 journal 隔离标志、业务 fold、私有确认凭据与回调轨迹 | JSON／String、`ENV-QUEUE-APPLICATION-HOOKS`；恢复使用新的可用解码 journal | 隔离、回调／错误顺序、部分业务效果、成功后确认、仅诊断入口和全新完整 replay 结果已证明。原生 callback／上下文／File 映射、公开可变 API、派生缓存和跨日志所有权仍未完成。 |
| `Coordinator::apply_transaction` / `State::apply_business` → `queue_history` | 全部业务 JSON 根、epoch／order／sealed；任意有限事件／事务 fold | serde／String 契约；回调只借用独立缓存或记账对象，业务输入只读 | 初始化、完整准入、分派／回执顺序、失败前缀与位置、实际解码向量到字节前缀业务 fold 已证明。缓存／记账内容、原生 run／File／锁／fencing、quarantine 与跨日志所有权仍未完成。 |
| `Catalog::collect_with_stat`、`gc_batch::collect` | 实际 BatchView：固定目标／父目录、mark、当前／持久缺失和回执集合；GCDeletion、GCBatchDeletion、GCReceipt | 正确刷新且锁内的 owned/sealed/tombstoned 快照、完整根集合；BatchIo 满足 Effects 契约 | 实际批次循环已证明所有目标 unlink、全部共享父目录屏障先于回执，错误可有副作用而仍保持回执安全 | 实际 gc_mark 已连接完整候选扫描、精确 resolved JSON mark、持久追加与完整 pending 快照；应用根／完整闭包、原生锁与目标路径、repair／receipt 效果及活性仍需细化 |
| commit/recovery/collection 的重复调用 | `retry_liveness` 和 `gc_liveness` 的抽象事件历史 | 有效输入；锁与 I/O 进展；分别声明每个持续可执行 effect 和 caller retry 的公平性 | 稳定后的模型执行完成；GC 单对象定理从永久无主且已经 mark 开始 | 真实调用历史到事件的映射；从 sealed/unowned 到发现／mark 的无界活性；永久无效输入的错误终态 |

## 前提的责任归属

以下调用者目前只能通过源码审查和回归定位责任；除单列的 helper 函数体外，尚没有证明整条调用链建立这些前提。

| 已证明函数／性质 | 建立前提的实际调用者 | 未闭合的对应关系 |
|---|---|---|
| `framing::{prefix,header,trailer,parse_header,trailer_valid}` 的字节布局、整数往返及检查等价性 | `Journal::{append,recover}`、`Catalog::{append_frame,refresh_with_sync}` | 共享 driver 已证明 SHA 输入范围和读取参数对应；具体 Device 与文件字节、JSON 记录语义仍未进入完整证明 |
| `wal::frame_extent` 的无溢出 frame 边界 | 两个 replay 循环，从实际 header 解码长度并传入 offset/size | offset/size 来自同一有效日志快照，调用者按返回的 end 前进 |
| `Indexes::{roots,candidates}` 的完整根／候选集合 | `Catalog::collect_with_stat`、reactivation 调用实际索引方法 | 实际 HashMap 到快照已证明；上层建立完整逻辑 owner，索引对应持久前缀，锁一直覆盖到 mark |
| `traversal::{schedule,next}` 与 `consumption` 的集合覆盖 | `closure_inner`、`consume_with_receipt` 从 manifest／缓存枚举 children | 解码正确且完整，缓存的图与 immutable reference 一致 |
| `commit_advance`、`delete_advance`、`reclaim_advance` 的阶段转移 | `Coordinator::commit`、`collect_with_stat` 根据实际 effect 的 Result 传入 success | Result::Ok 必须意味着该 effect 的全部业务／持久化后置条件；报错前效果必须保留 |
| `delete_batch_finished` 的逐项检查 | 实际 `gc_batch::collect` 已证明全部 parent barrier 返回后才传入 true | 泛型 parent 集合覆盖已证明；原生路径绑定、repair 对正确目录链建立屏障仍未证明 |
| `LogState` 的联合状态及有类型历史 | `refresh_with_sync`、`append_frame` | 状态方法已经证明部分修改前失效、失败后隔离及 cursor／sequence／索引一致；append_to 已将 finish 连接到 Writer 同步成功；仍需证明原始 frame 对应、replay 确认调用与原生 Writer／Source |
| `catalog_history_safe` 的无界模型不变式 | 未来 Catalog 的完整 View／调用者细化 | 当前 `serialized_catalog_history` 是未证明的 straw 适配前提，真实锁和持久化服务只是其中的外部部分 |

外部可信边界包括文件系统与 OS 的字节读写、同步、锁语义，哈希认证契约，以及部署提供的 coordinator fencing。SHA／serde 实现也尚未进入证明。`exclusive_owner` 只是调用者提供的声明，不构成 fencing。

属于 straw 的义务包括：对正确对象调用这些接口、解释返回值、保留错误前已产生的副作用、构造完整闭包和快照、在索引可能部分修改前设置 rebuild、从恢复状态建立下游 API 的前提。它们目前未证明的部分必须保持 open，不能通过将其写成“环境满足”来关闭。

应用可以无限期保留 reader/checkpoint owner。此时安全性要求保留数据。回收活性只适用于最终永久无主的 sealed 对象，并额外要求继续 collection、所需任务公平以及操作最终完成；不能由文件系统恢复单独推出应用释放。

## 关闭一条对应关系所需证据

必须同时记录实际源码函数及状态 View、模型动作／不变式、建立前置条件的调用者、正常和报错路径的后置条件，以及剩余可信接口。Verus 要检查实际函数体；TLC 和故障回归验证模型范围及具体执行，不能代替实际调用链的细化。源码更新后，应重新检查受影响证明并保留精确输入哈希。

实际 `catalog_frame::Decoder`、`catalog_decode::decode` 与 Target 已连接解析／run 检查、字段／语义准入、有序业务操作与原始历史。`catalog_append` 建立输入／run／序列化到该历史的另一侧对应。catalog_equivalence 已在单列的 Value 观察一致性与 String 内容身份前提下证明相同原始前缀的六类索引／cursor／sequence 唯一性；原生命名空间／缓存／集合／File、追加前规范化和完整持锁事务仍未闭合。

`journal_recovery::Apply` 检查 JSON event-list 结构，按原始 frame 顺序重建 transactions。`journal_state::State` 重置部分恢复、确认匹配边界并隔离不确定追加；这不是完整 Coordinator 业务 apply 或外层 Journal 文件／run 绑定证明。

`request::{lookup,response,copy_field,remember}` 是实际验证的回执函数；`request_history` 对 admitted 历史证明幂等和保留。原生请求 key／hash、admission、授权及完整业务／跨日志恢复仍是单列义务。[正式验证范围](FORMAL_VERIFICATION_zh.md)记录当前确切输入、回归证据与可信库定律。

`Coordinator::apply` 现将六类 consumer／batch／checkpoint／retention 分支及末尾请求回执交给实际已验证的 `queue_metadata::apply`。View 是精确嵌套 JSON edit 序列；成功执行完整计划，失败描述已执行前缀及首条无效路径。`json_update::update` 在单列 Map／Clone 库契约下证明精确叶子效果和无效路径不变。原生分支选择、部分状态隔离、其余事件、调度缓存及完整 replay／跨日志组合仍是调用者义务。

`journal_append` 将实际 queue 事件校验／序列化连接到 State／Writer 追加；`journal_equivalence` 将所得帧连接到成功的字节／JSON replay，并在相同原始前缀下保持有限历史观察。这不等于完整业务 replay、解析接受性、物理前缀选择、原生 File 对应或 run 身份比较已获证明。

多队列／请求的 `cross_log` 归纳证明现建立组合的**持久效果模型**不变量；与实际 `catalog_state::eligible` 的条件连接明确要求原生持锁快照建立根对应。它尚未证明 Coordinator／CommitIo／BatchIo、命名空间、闭包缓存或下层 WAL 恢复产生这些历史；上述实际入口仍需承担这项细化。


实际 `gc_receipt::record` 已证明规范目标集合到回执编码／持久追加／精确 pending 与 pack 移除的组合，保留 tombstone 和其他索引。实际 `gc_io` 已证明删除／父目录分派，`directory_io` 已证明完整有序屏障循环及失败前缀。原生目标／路径／物理不存在的绑定、完整祖先枚举与 repair 聚合持久性、已恢复锁上下文仍未闭合；回执编码不替代物理删除证明。

## 抽象持久效果与剩余细化边界

下表覆盖 `cross_log::Action` 的全部动作。相关 TLA+ 模型使用同类持久效果，其逐模型环境契约在清单中单列。表中已有下层证明不等于外层原生动作已获证明。`DurableHandoff` 的 WAL 碎片投影另有证明，但它与此组合模型的细化连接仍未完成。

| 抽象动作 | 实际实现及已建立结果 | 剩余实现对应 |
|---|---|---|
| `Retain`、`Hold`、`Release` | `Catalog::{retain_many,hold,release}`；有类型索引 fold、catalog 追加／历史已证明 | 原生逐节点／pack 映射及缓存上下文、逻辑 owner 命名空间、锁／refresh 与原生追加效果 |
| `Stage` | publication finalization 与 catalog owner 追加；实际顺序 controller 已证明 | payload／路径持久化、完整发布闭包、writer 修改资格与 generation 映射 |
| `Accept` | `protocol::execute_commit`、`journal_append`、request helper；分派／隔离、精确持久编码 frame 和回执操作已证明 | admission／key／digest、真实 retain 闭包、完整业务 apply 与原生效果；`applied=false` 表示接受已持久而缓存不可用 |
| `Retire`、`Prune` | `Coordinator::collect_garbage`、metadata apply、`Catalog::prune`；实际元数据更新与 catalog fold 已证明 | 完整 live 根、先退休后释放的真实调用顺序、未来 replay 根保护；单调退休模型不含 rewind |
| `Consume` | `Catalog::consume_once`、实际 receipt 集合更新与 cleanup controller 分别有证明 | 真实 request／handoff 身份、遍历与追加绑定；restaging generation 的完整细化尚未完成 |
| `Mark` | 实际 `gc_mark::mark` 扫描 catalog 并精确持久标记全部合格候选 | 逻辑 protected 对象与刷新后 catalog 快照的对应；原生锁／fd／namespace 绑定 |
| `Unlink`、`DirectorySync`、`Receipt` | 实际 `gc_batch::collect` 在逐 effect 契约下证明目标覆盖及屏障先于回执 | `BatchIo` 目标／父目录映射、真实 unlink／repair／sync 和 catalog 回执效果；批次到逐 pack 动作的投影 |
| `CrashQueue`、`RecoverQueue` | journal／catalog 字节恢复、JSON 事务重建、实际 queue_event 有序目标选择／恢复等价和六类元数据分支已证明 | 实际保留前缀、完整 Coordinator replay、run／epoch／fencing 和缓存重建 |
| `CrashStorage` | 外部崩溃可恢复未同步删除；既有 WAL 投影独立描述保留 frame | 部署持久性契约，以及原生命名空间恢复到抽象 present／durable 集合的对应 |
| `Idle` | 抽象 stutter 不改变模型字段 | 每个省略的原生操作必须保持 View 或映射到其他动作；尚未证明全部 Rust 语句 |

这里关闭的是原子抽象及其显式剩余边界的清点，不是原生细化、跨日志顺序或系统活性。不能从几个模型分别通过直接推出组合正确。
