# 形式化验证：TLA+ 与 Verus

[English](FORMAL_VERIFICATION.md)

**核心实现细化尚未完成。** [实际调用链对应表](VERIFICATION_BOUNDARIES_zh.md)列出当前 View、调用者前提和可信边界。已有模型和 helper 证明通过，不等于原生 WAL／锁适配、全部派生索引、coordinator 组合与 GC 调用链已被证明。

各层环境假设已拆成独立模块和带稳定编号的清单，见[环境模型与假设审查](ENVIRONMENT_MODELS_zh.md)。

清单逐条区分保证提供者、straw 实现义务和未证明的对应关系。可用 `python verification/runners/environment_manifest.py --list-layers` 查看组件层，`--layer durability` 单独审查持久化环境，`--model GCReceipt` 查看 GC 模型跨层的全部前提，或用 `--contract ENV-GC-IO` 追踪受影响模型；详见[独立审查入口](ENVIRONMENT_MODELS_zh.md#证据与独立-review)。组件层与下述 L0–L5 证据层级分开；清单检查通过不代表 L0 假设已被 L3 实现证明。

审查以“当前组件消费的契约”为边界：上层环境可以包含 straw 自己的下层实现。环境假设失效时，还需区分建模遗漏、部署不满足契约和实现适配错误，不能直接归因于 JuiceFS；各层责任见[层间契约由谁满足](ENVIRONMENT_MODELS_zh.md#层间契约由谁满足)，现场归因使用[故障审查记录](ENVIRONMENT_MODELS_zh.md#故障审查记录)分别记录安全性与进展影响。

控制面已经加入 **Anvil 式的渐进验证结构**：实际 Rust 控制器通过 `View` 对应数学状态机，逐步证明可执行转移与规范一致，再单独证明模型的安全性和条件进展性质。目前落地于提交顺序、lease 过滤去重、GC 候选选择与删除；尚未证明全部 straw Rust 或完整恢复流程。

## 本次发布的验证范围

本次发布冻结新增形式化验证，保留并持续运行已经接入的 TLA+、Verus 和语义负例。剩余实际调用链对应与活性证明延期；以真实实现的回归、故障注入和恢复测试作为本次发布检查，不把测试通过记成证明完成。

| 尚未完整证明的范围 | 本次发布的实际测试 |
|---|---|
| catalog 持锁 retain/reactivation、闭包缓存与完整调用链 | `test_online_gc.py`、`test_consumer_rewind.py`、`test_dependency_traversal.py` |
| queue/catalog 跨日志交接、staging 释放与完整恢复组合 | `test_durable_handoff.py`、`test_commit_consumption.py`、`test_staging_consumption.py`、`test_request_replay.py` |
| extent 全部认证读取与独立区域可见性 | `test_extent_visibility.py`、`test_extent_bounds.py`、`test_tensor_preflight.py` 及 Rust Reader 回归 |
| 原生 replay 部分失败、缓存隔离与公开入口组合 | `test_wal_recovery.py`、`test_metadata_recovery.py`、`test_journal_contract.py` 及 Rust Journal 回归 |
| 崩溃 writer 的完整 publication/ownership/GC 组合 | `test_writer_retirement.py`、`test_pending_publication.py`、`test_incomplete_publication.py` |
| 采样续读、训练 checkpoint 与 GC 的完整组合 | `test_resume_workflow.py`、`test_task_recovery.py`、`test_queue_gc_roots.py` |

真实调度、锁/I/O 最终进展、完整跨层安全性和条件活性仍未获得端到端证明。文件系统、外部 fencing、应用释放 owner 等假设继续单列，不因改用测试而扩大环境保证。测试只覆盖执行到的场景；性能结果另以实际构建和工作负载标识记录。

采样只从已持久化的 continuation 恢复。训练应用负责选择并加载 checkpoint，把模型/优化器状态与数据游标一起恢复，并保护 checkpoint 后需要重放的数据；straw 不会自动加载模型，也不能恢复已经合法回收的数据。


## 实际 GC mark 到持久回执调用链

生产 `gc_collection::Batch::run` 已证明完整的下层 collection 链：持久 mark、精确 pending 快照、全部目标 unlink、共享父目录屏障、缺失父目录的完整祖先修复和持久 receipt。成功建立 `after_collection(initial)`、pending 为空且原始目标集全部持久缺失；返回错误只允许初始、mark 后或完整 collection 后的业务状态前缀。适配器显式保持不可变上下文，允许物理状态和日志状态按协议变化。

路径／目录 handle 身份、完整祖先枚举以及单次 unlink／sync 语义保留在 `ENV-GC-NAMED-PRIMITIVES`；完整 Batch／Repair／receipt 算法由实际 Rust 证明承担。外层持锁刷新、完整根来源、queue 先退休再 prune 和条件进展仍单独推进。[固定 collection 记录](../verification/measurements/20261026-gc-collection.json)保存确切输入、正向与语义负例及原生回归证据，不自动覆盖后续源码，也不证明性能。

## queue 根集合与退休流程集成

实际 `queue_roots` 遍历包含未结束 task、未消费 commit、未结束 batch、reader、consumer、producer 和 checkpoint。此前遗漏了 producer 续采样状态：合法保存的 `producer_state` 仍在队列里，但 GC 可以删除其唯一数据。旧安装 wheel 已复现；包含修复的独立快照通过 310 项定向 Python 回归，包括重新打开、状态替换、请求重试及共享依赖。这些结果对应那份快照，不自动覆盖之后的 writer 退休改动。

`queue_gc` 已接入真实的退休、owner 构造与锁内 collection 分派。字段私有且不能克隆的凭据只在退休调用成功后携带同一份根集合；owner 构造保持完整，`require` 拒绝缺失、空集合或已 tombstone 的所有权，`prunable` 只选给定队列前缀下已经过期的 owner。`retirement_preserves_roots` 从实际 `queue_metadata::outcome` 的 `RetentionAdvanced` 分支推出七类根保持，没有把整段保持性质直接当作前提。

JSON 库边界新增一个窄表示定律：Map entry 的字符串 key 按字符内容唯一；已检查的展开引理连接既有 map 表示和字段查询，没有信任新的 straw 算法。`ENV-QUEUE-GC-CALLS` 将精确调用／结果轨迹与下层持久化效果分开。原生 commit／上下文、progress cache 和完整删除／回执对应仍未闭合，不能从接口成功推断端到端 GC 正确。递归根遍历证明正常返回结果，不保证无限栈／分配能力或终止。[集成快照记录](../verification/measurements/20261026-writer-roots-integration.json)绑定 695 项完整入口 Verus 检查、127 项 Rust 运行测试、13 项编译文档测试和 947 项安装 wheel 的 Python 测试；后续源码修改仍须重新取得证据。

## 原生 WAL 文件边界

生产 `wal_file` 现调用经过检查的 `file_adapter::NativeFile`／`NativeBuffer` 函数体。不可复制的 tracked 权限保存可变 bytes、cursor 和持久前缀；标准库 File 的逻辑值只表示句柄身份。适配器传递真实长度、offset 和字节切片，在缓冲转换中保持同一 descriptor，按顺序散列两段输入并传播原语错误；File／Writer 有效性显式传递到 Source／Appender 和追加调用者。

`file_primitives` 中十个标准 I/O 薄封装属于固定并审计的可信边界（`ENV-NATIVE-FILE-PRIMITIVES`），不宣称已经证明 OS／标准库内部。SHA／错误转换和 callback 仍单列契约。构造与可变方法还通过经过检查的借用归还关系连接句柄与权限，采用 [Verus final-reference 语义](https://verus-lang.github.io/verus/guide/mutable-references.html)。生产 Journal 的恢复、追加和根退休现调用 [journal_io](../rust/journal_io.rs)：构造原生适配器、执行实际 WAL／业务操作，并在每条 Result 返回路径交还原始 File／权限。成功将交还权限绑定到精确 cursor、持久边界及已接受事件；恢复失败保持原状态或隔离状态。Source 构造时读取长度失败也交还原始借用。[固定调用链记录](../verification/measurements/20261026-journal-io.json)包含把操作错误地指向另一份 State、掩盖恢复失败的语义反例。下述后续 Journal 所有者证明已覆盖初始打开与 Run 准入；锁授权、原生 callback 对应和完整 coordinator 组合仍是独立义务。

[调用链整合记录](../verification/measurements/20261026-owned-chains-integration.json)固定同一组证明／运行输入，覆盖 Journal I/O、拥有实际字段的 catalog 事务、退休准备及 writer 恢复。记录包含完整正向检查、原生和安装 wheel 回归，并明确排除后续 progress 缓存与 typed GC callback 修改。

生产 [journal_owner](../rust/journal_owner.rs) 持有公开 Journal 实际使用的同一 path、可选 File 和 State。创建分支要求完整祖先目录准备，打开新文件，完成文件与父目录同步，再通过 `journal_io` 追加精确的 `Run(identity)` 首帧。恢复分支回放同一句柄，仅当首事务与预期 Run 头相等才返回；合同保留原始字节、解码历史及精确的 accepted-frame 关系。`append_with` 在错误时保持文件身份、只读模式和已确认的 sequence／offset，不确定写入继续隔离。`close` 移走句柄但不清空状态，之后追加在调用 hook 或 I/O 前被拒绝。证明权限与初始化证据会从普通构建中擦除。

单次 OpenOptions 行为与 Path 表示固定在 [journal_primitives](../rust/journal_primitives.rs)。原生对应边界仍逐项认证目录准备、文件／目录同步和故障回调，不提供“整个 Journal 初始化成功”或“身份检查正确”的假设。文件系统持久性、调用方 lease 或 writer gate 提供的独占权和 callback 不干扰仍是独立环境义务。原生回归覆盖错误身份、已有 journal 不可被新建覆盖、只读与关闭后拒绝追加、两类初始化屏障，以及 Run 已持久但回复丢失；既有 append-unwind 和失败回放回归继续覆盖隔离与恢复。

## 实际 catalog 命名空间适配器

`catalog_namespace::Adapter` 已用经过检查的 Rust 实现生产 Resolver trait。构造、reset 和 resolve 保持显式 `valid` 关系，将 cache 绑定到 source namespace；该前提继续传递到字段解码、原始 Target 恢复／应用、append、mark、receipt 和 queue owner release。证明使用普通的显式前置条件，没有新增整条调用链不 panic 的假设。`NativeKeySet` 仍调用原来的标准批量构造，在单列库定律下验证，没有改成逐项插入。

实际借用组件 factory、PathSource 构造／legacy 分派、cache 初始化、有序组件解析和缺失目标重建均已检查。`namespace_step` 把实际逐组件算法连接到单次 readlink／canonicalize／Path 原语。剩余原生义务是在真实 catalog 锁下建立初始或复用 namespace 的有效性，以及准确对应这些原语。稳定别名和真实的缺失响应仍属环境要求，不再假设整个 Resolver 或逐组件算法正确。

## 显式 writer 退休

`Store::writer_id()` 与 `Config::retire_writer`（Python 为 `writer_id`／`retire_writer`）定位一个确定的 incarnation。每次修改都取得永久 gate 并检查持久 fence。退休拒绝仍被持有的 gate，依次持久化 fencing、排空／修复 payload 文件、核对操作 registry、封存 reservation 并记录完成。gate 和 registry 位于 `control/writers`，与可以删除或重建的 payload 目录分开。

恢复先 staging 全部完整结果，再释放 registry 中的准备 owner。普通重试与退休共用中间 publication 的持久消费身份，因此丢失回复不能释放其他客户端后来重新建立的 staging。未知旧 owner 继续保护。recover／abandon 是显式策略，没有依据超时接管或自动退休守护进程。

实际 `writer_registry::{decode,apply_event,apply_frame,replay}` 已通过 `journal_history_to_registry`，将 journal 接受的原始前缀及其实际解码值连接到有序 registry 业务历史。它检查严格 schema、有效事件准入、单调登记的操作 ID、不可改写的依赖／结果证据及精确结算。`reconcile_registered` 从这些真实操作选择每次验证／staging、中间引用消费身份、准备 owner 和最终结算 ID。成功路径给出精确两遍动作顺序；错误只能执行该顺序的前缀，允许当前叶动作已经生效再报错。恢复回执保留已有条目，覆盖全部恢复结果，新增值必须来自已记录的结果证据。

生产 `writer_gate_flow` 检查 open／lock／inode／header／fence 准入，并拥有返回的句柄。`bound` 拒绝 named inode 改变；marker 持久化检查 create-or-reopen、文件同步和目录同步顺序。实际 drain 循环同步 pack，只删除枚举出的同 writer 范围内 partial 名称，最后执行目录屏障。Store 发布／清理函数借用拥有句柄的 permit，Rust 所有权保证调用期间句柄仍存活。`writer_ownership` 检查准备 owner 已登记、held path 有同 owner 依赖闭包的正证据，并精确选择父目录身份属于该 writer 的未 seal pack。native 实现直接调用这些已检查的算法。

恢复认证直接复用已检查的 `closure::collect` 遍历和 `closure_node::children`，`authenticate_all` 覆盖每个结果根；native reader 每次只认证一个 manifest 或 record，不再提供整个递归恢复遍历。剩余 native 对应逐项限定为：descriptor／run／writer 构造，标准 map／目录快照，OS open／lock／metadata／read／unlink／sync 与 Path 原语，Reader 单节点认证，以及指明参数的 catalog stage／consume／release／append 调用。它们是独立契约和适配义务，不把完整退休、owner 筛选或 reconciliation 算法作为文件系统假设。`WriterRetirement` 检查有限的暂停／死亡／重试与 GC 交错；环境要求 gate 交接前已提交 I/O 完成或取消，交接后的 fence／registry 读取能观察之前的持久状态。重新 open 本身不能排除负目录项缓存。四机 smoke 只确认同步 payload chunk 之后的具体现象，不认证任意远端 I/O 排空、锁交接一致性或延迟上界。锁和 registry 同步增加了运行工作，性能尚待测量。

只读 `publication_receipt` 按精确 writer／API／submission 身份查询，fencing 后仍可使用。已检查的选择器包含内部 Done／Retired 已持久化的结果，覆盖完成后响应丢失的边界。查询不重新 staging、不改变所有权，也不会恢复已经消费并回收的引用；用法和边界见[使用指南](USAGE_zh.md#发布响应丢失后的-writer-回执查询)。

[writer 准入记录](../verification/measurements/20261026-writer-admission.json)区分有限协议模型、经过检查的 gate／marker／drain 算法与原生 Permit 接线。旧 writer 排除及在途操作顺序依赖明确的固定 gate／新鲜观察／drain 契约；完整 publication 到所有权／GC 组合仍单列。

## Rust 所有权边界

Coordinator 的业务状态、limits、epoch 以及持有的 journal／storage 现提供不可变视图。Journal 保留只读解引用以查看 sequence／transactions，但不再暴露可变解引用或可变路径。外部 Rust 调用者不能经这些接口清除隔离、修改已恢复业务状态、绕过 coordinator 直接追加 WAL、替换 journal，或修改持有的存储配置；直接 `usage()` 也执行已验证的可用性判断。

这属于 Rust 可见性／借用检查证据，与 Verus 状态转换证明分开：新增八个编译失败示例拒绝上述操作，正向示例检查只读查看；既有捕获 apply unwind／GC 回归同时检查直接 usage 被拒绝。封装本身没有完成原生 callback／上下文、文件身份或整个分布式调用链对应。

Rust 读取迁移到 `state()`、`limits()`、`epoch()`、`journal()`、`store()`，Journal 路径使用 `path()`，发布使用独立 `Store`。Python API 和持久格式不变，`recovery_seconds` 诊断仍可直接读取。

## 实际 apply 隔离与全新恢复状态

[queue_application](../rust/queue_application.rs) 已接入生产 commit 的隔离、回调、业务更新和确认。`arm` 在回调之前使实际 journal 状态失效；`apply` 只读借用该 journal，在两个回调和完整事务都成功之后，才返回字段私有的确认凭据。`confirm` 消费凭据，只清除隔离标志。既有 commit 控制器将凭据保留到成功分支。`permitted` 是实际入口检查：隔离或已关闭的 coordinator 只接受诊断 `state` 查询，不改变原有诊断权限，也不增加 I/O。

`application_result` 区分业务修改前失败、事务部分失败和完整应用后回调失败；`hook_result` 在独立的 `ENV-QUEUE-APPLICATION-HOOKS` 接口下证明 before／after 调用和结果的精确顺序。下层回调可以报错或产生外部副作用，不保证成功、终止或 unwind 回滚。原生 callback 名称绑定、完整 CommitIo／上下文对应仍是已审查并回归测试的适配边界，不因声明接口而自动获得证明。

`restore` 从已证明的初始业务状态重新构造，仅在全部 replay 成功之后交出 `Restored`。任意返回错误都保持 journal 隔离，同一份隔离的解码状态不能直接重试；须先由物理恢复建立新的可用 journal。`rebuilt_fold` 和 `journal_equivalence::restored_prefix` 将全新结果连接到被接受的字节前缀，不使用此前部分应用的业务状态。派生缓存／消费记账、原生文件身份、公开可变 Rust 字段及完整 catalog／GC 组合仍单独保留。Verus 的正常返回证明与捕获 unwind 的原生回归分开计证，后者复用既有 commit-apply panic 回归。

规格敏感性检查刻意删去回调结果后置条件，并忽略 after 回调错误，剩余业务状态证明仍能通过；保留轨迹后置条件时，同一代码 mutation 会失败。这说明回调错误传播需要独立规格，不是新生产缺陷的证据。

[application 记录](../verification/measurements/20261026-queue-application.json) 保存确切输入、回调／隔离／恢复语义负例及原生／wheel 检查。本轮不声称新发现生产缺陷或零开销，性能测试继续后置。

## queue 业务历史与 WAL 组合

生产 [queue_history](../rust/queue_history.rs) 已连接初始化、事件准备、完整目标准入、业务分派、缓存回调与最终回执。commit 使用其中的 `transaction` 循环，恢复使用 `journal_state::State::apply_business`，直接借用该 journal 实际解码出的事务向量，只跳过第一个身份头。实际 `journal_owner::open` 现会先将首帧与请求的 Run 身份比较，再返回持有句柄的 Journal。

`transaction_result` 覆盖输入顺序中的每个事件；失败给出完整成功前缀及当前事件已经产生的部分效果，不把错误误写成回滚。`recovery_result` 扩展到任意有限事务序列，区分事务前的消费记账失败与事务内的业务失败。`frames_fold` 证明分帧执行等于相同顺序的完整事件 fold。`journal_equivalence::physical_prefix_business` 组合实际 WAL 恢复与业务应用的后置条件：最终业务状态来自**被接受字节前缀实际解码出的 Values**，去掉身份头后逐事件执行。它没有把另一份独立解析或仅观察等价的历史代入；现有数字观察模型仍只覆盖无符号整数观察。

这份定理中的状态包含全部业务 JSON 根，以及 epoch、排队序号和 sealed 标志。初始化函数证明各表／数组为空、retired prefix 为零。缓存和消费记账回调只取得不可变业务输入及单独借用的可变状态；正常返回无法修改业务状态，证明没有要求缓存内容正确。调度／usage／deadline 一致性、记账 ID、回调终止／unwind、原生 File／run／fencing 对应及跨日志／GC 所有权仍未完成，与现已证明的事件和事务循环分开。

[历史组合记录](../verification/measurements/20261026-queue-history.json) 保存确切输入、语义 mutation 和原生／wheel 检查。回归覆盖准入先于业务修改、业务部分失败、刷新缓存后的回执失败、事务／事件错误位置与混合历史。恢复路径已去掉整份事务历史的克隆，没有新增 I/O 或持久格式变化；这不是吞吐测量。

## 实际 queue 事件分派与 worker 退休

[queue_replay](../rust/queue_replay.rs) 是 `queue_history::apply` 实际调用的业务分派函数。`body_result` 将每种事件绑定到既有 task／reader／receipt／metadata 关系，同时保持不应变化的 epoch、排队序号和 sealed 状态。租约与失败批次保持输入顺序及重复 ID；错误描述全部成功前缀，以及出错条目已经产生的效果，例如失败计数器报错前 reader 可能已经释放。`receipt_flag` 精确指出已经写过请求回执的 metadata 分支，`finish` 只为其他分支登记回执，已登记分支保持状态不变。原生 deadline 回调仍在原来的成功位置执行，批次部分失败时也保留原顺序；缓存内容尚不属于这份业务状态证明。

`queue_reader::retire` 证明实际 worker 判断与完整筛选结果，`queue_admin::reader_retired` 将事件 worker ID 绑定到 queue 的 reader 表。只删除匹配 worker 的条目，其他 reader 的值及全部续读根不变，其他 queue 根也不变。缺失或类型错误的 reader 表返回明确错误。缺失 worker 字段继续沿用原有 null 索引／相等语义；API 授权另属调用者义务。回调体只写一份，验证构建附加闭包合约；原生仍使用 stable Rust 和原来的原地 `Map::retain`。

`ENV-JSON-LIBRARY` 新增**一个外部库基础接口** `Map::retain`，当前白名单共二十一个方法。正常返回时它覆盖所有原始 key，将实际回调决定及最终值对应到保留结果，并禁止凭空新增 key；没有假定 straw 判断、事件绑定或 peer 保持已经正确。实际回调只捕获不可变引用，且不修改条目。可变捕获状态、库内部、回调 panic 回滚和遍历顺序不属于这个契约的已审查使用范围。规格依据固定库版本对 `BTreeMap::retain` 的委托及 Verus 的[回调合约语义](https://verus-lang.github.io/verus/guide/exec_funs_as_values.html)。

本次还发现批量 `yield_tasks`／`release_tasks` 缺少容量预检，现已在缓存回复查询之后、retain／WAL 写入之前检查新序号总数。扩展原生测试强制将私有计数器设为上限，修复前进入禁止写入的 callback 而失败，修复后通过；分别移除两个新 guard 的隔离 mutation 也会触发该断言。这是容量边界防护，不是正常实际负载溢出的复现。[事件分派记录](../verification/measurements/20261026-queue-replay.json) 分开保存实际函数证明、语义负例和原生／wheel 回归。

上述历史组合已连接 prepare／目标准入与分派所需 kind，并完成业务事件／事务 fold；完整调度／usage／deadline 缓存、根提取、run／锁／File 适配及跨日志／GC 所有权仍未完成。没有新增文件系统操作、持久格式或 payload 复制；容量判断会在运行时执行，性能测试继续后置。

## 实际 catalog 事务入口与退出

生产 `Catalog::transaction` 现调用 Verus 检查的同一份 [catalog_transaction](../rust/catalog_transaction.rs) 函数体。在独立的 `ENV-CATALOG-TRANSACTION` 契约下，`execute` 与 `locked` 要求正确 pool／run 绑定；拒绝嵌套进入且不改变外层上下文；持有原始锁时打开新句柄，核对 device／inode 后才安装；恢复成功后才开放回调。回调最多执行一次。成功返回要求最终上下文仍绑定且 ready；已进入事务的返回错误路径会清除 active 标记。`finish` 证明精确错误优先级：操作与解锁都失败时保留操作错误，操作成功但解锁失败时不能返回成功。

模型允许回调留下未确认状态，或替换成 inactive 上下文，因此末尾校验必须拒绝这些情况。仍留下 active 上下文的合法回调必须保持事务身份和恢复来源；这是单独的原生／调用者对应义务，不是任意 Rust 回调都满足的保证。锁／open／metadata 基础接口、原生 refresh／context 适配、完整持久历史和锁竞争进展仍未证明。加锁或解锁报错可能已经产生效果，并无所有失败路径都释放锁的无条件结论。返回值路径证明也不覆盖任意 unwind；原生测试分别检查锁排他／释放、回调报错前的持久效果，以及捕获回调 panic 后必须重新打开才能继续使用。

[事务记录](../verification/measurements/20261025-catalog-transaction.json) 绑定确切输入、语义负例与原生／wheel 结果。提取保留既有文件系统调用顺序和错误消息，使用栈上封装与单次回调槽，没有新增堆分配、文件系统屏障或 payload 复制；没有新的性能结论。

原生边界审查复现了两条无锁写入路径。捕获回调 panic 后，直接调用 `Catalog::pack` 仍可能看到 active／ready 状态；现在通过回调 unwind guard 使日志失效。Rust 回调还可以用 `mem::replace` 取走活跃 Catalog，在锁释放后继续写入，甚至绕过其他描述符持有的锁。现在回调只取得内部私有借用的 `LockedCatalog`，只读 Deref 和显式操作转发不暴露可变 Catalog。Rust 生命周期／类型检查拒绝取走所有权、经可变解引用取走对象，以及将回调借用泄漏出去。这些属于编译器／回归证据，与 Verus 分派证明分开。

**Rust API 迁移：** `Catalog::transaction` 和 `Config::catalog` 的回调参数改为 `&mut LockedCatalog<'_>`。使用推导参数类型和普通操作的闭包保持调用写法；显式标注 `&mut Catalog` 的回调需修改类型。Python API 与持久格式不变；原生 callback／context 对应和完整持久历史证明仍未完成。

## 实际恢复与追加的文件操作流程

生产 `wal_file::Source` 和 `Appender` 现在使用 Verus 检查的同一份 [wal_source](../rust/wal_source.rs) 与 [wal_sink](../rust/wal_sink.rs) 函数体。`Source::from_file` 建立 device 有效性，实际 WAL、catalog Target 和 queue 恢复在所有返回路径保持此前提。有效性约束可用状态下的缓存长度；read、truncate 和 seek 在操作前置为不确定，失败后保持不可用，并在 seek 报错返回前恢复持有的句柄。消费缓冲器保持文件身份、字节和持久性，但允许底层物理 cursor 不同于缓冲器的逻辑 cursor；最终 seek 再建立所需位置，同步回调只消费一次。

`Appender` 从精确长度／位置观察、原输入写入范围、queue／catalog 同步及既有 fault hook 顺序建立 Writer 契约。两个实际流程在所有返回结果中保持同一抽象文件身份。`ENV-FILE-PRIMITIVES` 单列 File／BufReader／Take、SHA 和回调语义；原生薄封装、快照新鲜性、多次成功读取的一致性、真实锁／open／run 绑定与物理 I/O 对应仍是下层义务。没有省略 straw 函数体。错误返回证明没有覆盖任意 Rust unwind；另有原生测试验证同步回调在产生文件副作用后 panic，被捕获后 source 仍不可用。

此次审查修正了一处过强的读取契约：零字节读取在 EOF 之后也可成功，cursor 不变。`read_matches` 现在允许这一行为，同时保持所有非空读取的精确范围与内容；`empty_read_matches_any_cursor` 防止再次用规格排除它。真实缓冲／非缓冲文件测试覆盖该场景。这是规格修复，不是生产数据损坏证据。[Rust Read 接口](https://doc.rust-lang.org/std/io/trait.Read.html#method.read_exact) 属于外部库边界。

[文件适配记录](../verification/measurements/20261025-file-adapters.json) 绑定确切输入、正向证明、语义负例及原生／wheel 回归。提取保持 payload 复制、缓冲容量策略、文件系统调用及既有故障点；新增防御性拒绝违反既有 Device 前提的直接带缓冲 truncate／sync，并在读取分配前标记不确定。没有性能测试，也没有零开销结论。

## 实际依赖遍历与闭包缓存

生产 `Catalog::closure_inner` 现在调用 [closure::collect](../rust/closure.rs)，证明成功返回的集合恰好等于有限不可变图的全部可达物理 pack。实际堆 frontier 处理重复边、循环和共享 pack；缓存子图提供完整可达集合，只缓存完整的请求根，缓存清空仍保持有效性。读取报错不会写入部分闭包，空缓存基例也已证明。循环以尚未访问的节点集合严格递减，因此在逐节点调用能够返回时终止；这不是 I/O 延迟或资源上界。

实际 `closure::resolve` 再组合 tombstone 扫描：成功结果与删除集合不相交；`Reclaimed` 错误必须有真实可达 tombstone。冷遍历和缓存命中都检查。这在有效图、根身份正确且初始缓存完整的前提下，建立返回 retain 调用者的集合；尚未证明原生调用者建立这些前提或持锁追加位置正确。

`ClosureReader` 调用的实际 [closure_node](../rust/closure_node.rs) 现已证明字段选择与 manifest 后继构造。manifest 字段存在时优先于直接 segment，包括值为 null 的情况；摘要计算前先检查两个后继数组。成功结果按 records 后接 dependencies 的顺序完整保留重复项、克隆内容及摘要／引用配对；摘要错误不会返回部分列表。`complete_keys` 和 `graph_binding` 在明确的图域及 key 上下文前提下，将结果连接到遍历接口。[逐节点记录](../verification/measurements/20261025-closure-node.json) 绑定确切输入、8 个语义负例和运行测试。

`ENV-CLOSURE-SOURCE` 仍单列认证 manifest／index 读取、原生序列化／SHA 对应、规范 pack 映射及图／缓存上下文。这些原生前提和完整 `ClosureReader` 实现对应尚未证明。没有新增可信库基础接口，也没有省略 straw 函数体。提取会为展开的 manifest 建临时 child 向量，并在根缓存命中前克隆传入引用；字段选择现在使用既有的已验证对象迭代 helper。这些是可执行分配／查找变化。payload I/O、同步操作和后继顺序保持，但不能声称零开销。

[闭包记录](../verification/measurements/20261025-dependency-closure.json) 绑定确切输入及结果：完整 Verus 正向检查、13 个语义负例、原生循环／共享 pack／缓存／失败测试和真实文件的重试／retain／GC 回归。负例要求漏后继／pack／缓存后代、缓存失败前缀、错误身份绑定／缓存根，以及错误 tombstone 接受／拒绝命中语义失败。完整逐节点适配、所有权／GC 组合及活性仍未完成；没有新增性能测试。

## 实际 GC 发现、持久化 mark 与 pending 快照

生产 `Catalog::collect_with_stat` 调用 [gc_mark::mark](../rust/gc_mark.rs)。Verus 现在连接实际 owner 根枚举、完整 sealed／无主／未 tombstone 候选选择、resolved-path JSON 构造、catalog 序列化／追加和返回的 pending 快照。成功后 owners、packs、edges、consumed handoffs 不变；永久 tombstone 恰好增加全部候选；待删除集合包含旧 pending 和全部新候选。非空候选必须先确认一条持久 catalog frame，才允许进入删除批次；没有候选则不执行 mark I/O。mark 报错直接返回，不开始 unlink；不确定写错误使日志保持不可用，直到恢复。

mark 使用 catalog replay 已规范化的身份，写入 `path_encoding=1`，仍由既有 resolver 检查路径语法。移除的是 catalog key 的重复规范化，没有更改 frame 格式或增加文件系统屏障。候选数组采用 HashSet 快照顺序，协议不保证数组顺序；已有别名／旧日志回归覆盖此路径。这是验证接入，不是新发现的生产缺陷，也没有测得性能提升的结论。

GC mark 阶段的库白名单为十七个基础接口，该阶段只新增空 `Map::new` 和精确无符号 `Number::from(u64)` 契约，没有信任 straw 函数体。输入日志仍必须 valid、ready 且具有解释历史；Writer、Resolver、JSON 和 String 前提单列。完整应用根／闭包、原生持锁恢复／fd 绑定、`BatchIo` 的目标／路径／父目录／回执对应和最终发现／重试仍未证明。返回快照已供实际批次调用使用，但尚不代表完整外层原生 GC 细化完成。

[GC mark 记录](../verification/measurements/20261025-gc-mark.json) 绑定确切输入、完整正向证明、11 个语义负例和原生／wheel 回归。负例覆盖漏候选、标记 live 对象、错误字段／编码、忽略追加失败、丢失旧 pending 和重复处理已完成回执。新增集成用例覆盖六个 mark 写入／同步故障边界，以及旧 pending、新垃圾和受 reader 保护对象并存的恢复。没有性能测试。

生产 `BatchIo::receipt` 现调用 [gc_receipt::record](../rust/gc_receipt.rs)，把传入的规范目标集合连接到 resolved-path JSON、持久 catalog 追加和精确 reclaim fold。成功后只移除这些目标的 pending deletion 和 pack 记录，保留永久 tombstone、全部 owner、staging edge 与 consumed handoff。证明允许重复名称及任意已标记子集，不会清空其他 pending 工作。已完成回执投影为 `deleted − pending`：`mark_preserves_completed` 证明 mark 不会伪造完成回执，`receipt_completes_exactly` 证明本次追加恰好使目标集合加入该投影；空输入不追加。写入结果不确定时日志保持不可用，已确认历史不变，但完整且未确认的 frame 仍可能在恢复后保留。

回执直接使用 pending deletion 已选择的规范 key，去掉物理删除后重复解释名称／集合转换的流程；格式和屏障顺序保持，目标数组顺序不是协议保证。物理持久删除仍由调用者建立：原生目标／路径／父目录绑定、目录修复与屏障、外层持锁恢复上下文及薄 Writer／Resolver 对应仍未完成。这是实际调用组合证明，不是新的完整物理删除定理，也不是新复现的生产缺陷。[回执记录](../verification/measurements/20261025-gc-receipt.json) 绑定确切输入、语义负例及既有原生／wheel 故障恢复回归；没有性能测试。

## 多队列、多请求与共享所有权

[CrossLogComposition](../verification/protocol/tla/CrossLogComposition.tla) 与 [cross_log](../verification/protocol/verus/cross_log.rs) 组合 queue 范围内的请求 ID、持久接受、apply／回复丢失、反复恢复、retirement／prune、共享依赖、独立 holder、staging 世代、交接消费回执及 GC。queue 和 catalog 效果仍是不同动作。已持久接受但尚未 apply 的队列不能暴露缓存状态，恢复必须重建回执；退休一个请求不能丢掉其他活跃请求的完整闭包，包括其他队列的请求。

Verus 证明每个允许转换保持组合不变量、任意有限历史的归纳安全性、已接受回执不变，以及每个活跃请求的依赖保护。退休及物理 GC 后的缓存重试不要求旧 payload 仍存在；冲突摘要会被拒绝。已经消费的交接再次回放时，整个状态保持不变，包括后来创建的 staging 世代；持久删除回执不会清除 tombstone。另有具体可达历史见证，包含两个队列使用同一请求编号、apply 丢失、恢复、重新 staging、旧清理重放和一个队列退休，避免规范空洞。`retained_closure_is_complete` 还要求成功 retain 包含全部依赖，不能靠阻止后续接受来掩盖遗漏。

与实际 `catalog_state::eligible` 的连接是条件性的：`active_request_excludes_actual_candidate` 在**调用者已建立实际持锁 catalog 根与模型保护对象对应**的前提下，排除活跃依赖成为删除候选。这个对应本身没有被假定为已证明。完整发布闭包、owner 命名空间隔离、原生持久效果／锁／File 对应和准确快照仍是 straw 义务，见 `ENV-CROSS-COMPOSITION`。[独立环境](../verification/protocol/tla/EnvironmentCrossComposition.tla) 允许未同步删除恢复，但不允许已有持久名称丢失。此抽象没有展开 WAL 碎片；`DurableHandoff` 另行展开单次交接，尚未证明它到新组合模型的细化。此模型的 retirement 单调，未包含 rewind／重新激活；旧 lease／epoch 准入、训练 checkpoint／游标匹配、writer 恢复和 GC 活性也仍未完成。TLA+ 与 Verus 分别检查相关的有限／无界模型，没有宣称机械证明了两者之间的细化。

[组合记录](../verification/measurements/20261025-cross-log-composition.json) 绑定 378 项完整正向 Verus 义务和 14 个语义负例，以及三个有限配置（9,944／64,268／492 状态）和 13 个指定不变量反例。70 项安装 wheel 的集成回归通过，其中新增共享依赖、重新 staging、丢失回复、双队列及反复恢复的组合用例。全部生产及构建输入与此前验证的 wheel 一致；此次没有修改生产执行体，也没有声称重跑完整 Python／原生套件或性能测试。这关闭的是多请求／队列的模型组合义务，不是完整分布式实现证明。

## queue 事件编码与恢复事务

生产 `Journal::append` 现调用 [journal_append](../rust/journal_append.rs)：检查实际事件列表，序列化同一个 Value，把这些字节传给已验证的 `journal_state::State::append_to`。成功把输入绑定到一帧完整且持久的 queue frame、返回的原 sequence，以及不变的先前字节前缀。不可用、无效事件和序列化错误保持状态及 writer 不变；不确定写入错误保持确认边界并维持隔离。原生外层保留关闭文件与错误类别映射；此次提取保持现有序列化和 I/O 操作。

[journal_equivalence](../verification/wal/verus/journal_equivalence.rs) 把接受帧谓词与实际帧解码合约组合：成功读取该帧会得到精确 payload 与边界，随后成功解析的 Value 保持原输入的有限深度观察。`encoded_history_observations` 和 `repeated_recovery_observations` 对具有相同原始序列的任意有限历史给出相应结论。这是事务观察的对应，不是完整 Coordinator 业务 fold 的证明。数字只观察 `as_u64`；解析接受性、原生 fd／返回值对应、run 身份比较和物理保留前缀选择仍需单独证明。没有新增可信定律。

[queue WAL 记录](../verification/measurements/20261025-journal-append.json) 绑定 362 项完整正向义务、12 个新增及 21 个受影响语义负例、73 个原生测试和 909 个安装 wheel 的 Python 测试。一个最初无效的负例单独保留，修正后得到语义拒绝；两次正向检查输入相同。两种 Clippy 与格式检查通过，没有新增性能或 TLC 测试。

生产 `queue_history::apply` 现先调用 [queue_event::prepare](../rust/queue_event.rs)，再修改调度索引。Verus 检查 Submitted、Leased、Epoch、Expired、Failed、Cancelled、Yielded、TaskProgress、Completed 的完整有序目标列表：不遗漏、不重排、不去重；非法目标字段必须拒绝，形状正确的列表不能无故拒绝。没有任务目标的事件返回空列表；该函数不验证这些事件的其余业务字段，也不授权未知事件类型。

[queue_event_equivalence](../verification/protocol/verus/queue_event_equivalence.rs) 把同一 JSON 的独立成功解释连接到相同目标列表。`recovered_frame_targets` 和 `recovered_history_targets` 再组合实际 queue 编码与 WAL 恢复，保持每个事务／事件的位置。这是目标选择的对应；完整任务分派、缓存维护、旧 attempt／epoch 准入和完整业务 replay 仍未闭合；单条任务记录的实际修改见下节。[queue 事件记录](../verification/measurements/20261025-queue-event.json) 单独记录实际函数体、语义负例及原生回归。

唯一新增的可信库基础接口是借用 `str` 的索引查询，也就是 serde_json Value 索引使用的同一底层查询：缺失 key 或非对象返回 None，无须分配查询 key 或扫描对象。已核对锁定版本实际调用 Map::get；runner 将此接口加入十八项精确白名单。输出仍是一个有序 ID 向量及复制的 ID，现在按已知列表长度预留容量；这不是实测零开销结论。

## 实际任务记录的恢复更新

生产 `queue_history::apply` 调用 [queue_task](../rust/queue_task.rs) 完成任务创建、租约写入、失败重试、终态、失败证据及续采样字段更新。实际函数体证明精确修改范围及全部无关字段保持。retry 用 checked arithmetic 增加失败次数、清空租约，并按配置上限选择 pending／failed；不可重试的失败保持既有租约字段。progress 更新 input 及提供的 priority、scheduling key、metadata；yield 额外清空租约、返回 pending 并记录传入排队序号。非法 task／spec 形状与计数器耗尽返回错误且不改变该任务记录。这是局部保证，外围 reader／缓存／epoch 修改没有被声称为内存原子操作。

原生恢复回归发现：校验完整的 WAL 如果更新从未提交的任务，旧实现会补造不完整任务并继续恢复。原生调用者现在在业务更新前检查所有目标；九个畸形业务历史用例在旧 wheel 复现错误接受，在修复后要求 `UnsafeRecovery`，且不得追加恢复 epoch。这不表示合法 API 已被证明能产生该历史。计数器耗尽是防御性边界处理，没有另行声称发现合法 API 可达的溢出缺陷。

实际 [queue_state](../rust/queue_state.rs) 现把这些叶子更新绑定到队列 `tasks` 表中的目标任务。分派器传递精确 lease／event、终态、yield 标记及排队序号。`edit` 证明完整嵌套查找和修改范围：缺失表／目标时保持原状态并拒绝，叶子更新失败时整个传入 queue state 不变，成功仅改变所选任务，其他根字段和任务保持。`submit` 按 spec 中的 task ID 安装精确新记录；重复提交准入仍是外层义务；有序分配的实际函数现由下文 queue_admin 覆盖。旧的原生身份 helper 已删除。

生产 epoch 撤销调用已经验证的 `edit_many` 循环，轨迹完整保留输入 ID 的顺序和重复项。成功对应完整轨迹；失败指出精确已完成前缀及第一个失败的身份／目标／状态更新，不执行后续任务。`history_frame` 证明任意此类成功有限批次保持全部非 task 根字段（包含 reader／checkpoint）及未选中的任务。此局部定理针对统一更新种类的 task 批次；上文 queue_history 已补上外层事件类型／参数选择和完整混合业务 fold，缓存语义及原生恢复／隔离对应仍未闭合。下文 queue_admin 覆盖 epoch 赋值和有界序号分配。[queue state 记录](../verification/measurements/20261025-queue-state.json) 单列语义负例和原生／wheel 回归。未新增库基础接口、整段操作假设或文件系统操作，没有补跑性能测量。

实际 [queue_transition](../rust/queue_transition.rs) 现组合 Leased、Failed／Expired、Cancelled、TaskProgress／Yielded、Completed 的任务、reader 和回执更新。关系明确描述后续错误之前已经完成的效果：先安装租约再登记 reader；主动报错先释放当前 reader 再重试任务；过期和取消保留 reader；progress 先扩展 reader 根再改任务，yield 则先释放 reader。完成任务先释放当前 reader、更新任务状态、安装 submission 回执，再向 commits 追加；最后一步失败时保留此前效果，不把整个事件描述成原子回滚。返回的 task ID 借用已经校验的事件字符串。生产 Coordinator 调用这些函数，没有新增 I/O 或持久格式。Completed 现在仅在回执更新成功后清除派生 deadline；回执失败时保留已经隔离但尚未修复的缓存，原生 quarantine 对应仍需单独证明。

[queue_transition_roots](../verification/protocol/verus/queue_transition_roots.rs) 针对已经登记且格式有效的 reader，从实际成功 progress 的合约推导根保持：每个旧根仍处于原位置，非空新输入受到保护，其他 reader 条目不变。推导显式使用 `ENV-STRING-KEY` 既有 `string_content_contract`，把复制 lease 中的 attempt 字符串连接到同一逻辑 map key；这是该定理单列的前提，不是已经证明的库身份定律。此根保持定理未建立 API 准入／新鲜性、调度缓存、根提取及 catalog 所有权；后续 queue_admin、queue_replay 与 queue_history 已补上序号分配、reader 退休和完整混合业务 fold，原生恢复与 GC 仍未闭合。[复合 replay 记录](../verification/measurements/20261026-queue-transition.json) 区分实际函数证明、根保持推导、语义负例及回归证据；不声称新发现合法 API bug 或获得性能结果。

实际 [queue_admin](../rust/queue_admin.rs) 补上提交／序号、epoch、producer、ReaderDone 和请求表的绑定。提交 replay 按输入顺序分配连续序号，畸形历史中的重复 ID 也按原顺序处理，错误时描述精确已完成前缀。producer continuation 仅在任务批次全部成功后更新；producer 表错误保留已经完成的任务和计数器推进。epoch 赋值先于有序撤销，即使 revoked 数组本身无效也明确保留已赋的新 epoch。ReaderDone 和请求回执绑定到正确 queue 根，其他根保持；请求关系使用既有回执 ledger／内容抽象，不是 JSON 分配身份。

排队序号改为 checked increment，耗尽时在修改当前 submit／yield 的任务或 reader 前拒绝，此前成功批次保持；普通 progress 和空提交批次不消耗序号。`has_capacity` 检查完整申请数量，`capacity_rules_out_order_error` 证明符合这个容量上界的批次不会发生序号耗尽错误。原生 submit／yield 在缓存回复查询之后、retain／WAL 写入之前检查容量。强制设置边界的回归确认状态／日志序号不变、不进入 quarantine、旧请求仍可取回缓存回复；两份隔离源码分别去掉原生检查后，都会触发禁止写入的 fault callback，使测试失败。这是边界加固证据，不是正常实际负载耗尽计数器的复现。[队列管理状态 replay 记录](../verification/measurements/20261026-queue-admin.json) 记录确切证明和原生 mutation 输入。上文事件分派／ReaderRetired 阶段已覆盖事件类型／参数及 sealed 状态；queue_history 又补上目标准入和混合业务历史；API 新鲜性、调度索引和所有权仍需对应证明。未新增可信库接口、I/O 操作或持久格式；算术检查会在运行时执行，尚未测量其性能影响。

`ENV-JSON-LIBRARY` 单列新增的借用 `str::index_into_mut` 基础接口：只能修改选中的对象字段，缺失字段时保持原值；锁定 serde 实现调用 Map::get_mut。该阶段库白名单为十九个基础接口，没有省略 straw 函数体。[任务 replay 记录](../verification/measurements/20261025-queue-task.json) 分别记录实际函数证明、语义负例和原生回归。此叶子阶段未证明完整事件分派和历史；后续章节已组合 reader／submission 更新、序号与混合业务 fold，调度缓存、旧 lease／epoch 准入及原生 Coordinator 对应仍未闭合。这些字段更新证明不代表自动选择训练 checkpoint，也不代表完整跨日志／GC 安全性。此次未增加文件系统操作，没有补跑性能测量。

## Reader 保护、完成回执与 producer continuation

生产 replay 现调用 [queue_reader](../rust/queue_reader.rs) 登记租约／输入、追加读取根、删除单个 attempt 及按序执行 `ReaderDone` 列表。追加保留全部旧根和原租约，包含新输入，并按实际 Value 相等关系去重；其他 attempt 保持不变。实际释放循环允许重复 ID，并精确描述后续租约缺少 attempt ID 时已经执行的前缀。非法记录形状返回明确的局部错误。登记仍会替换所选 attempt，因此 attempt 身份新鲜性仍是准入义务。取消、超时及 epoch 恢复继续保留 reader。worker 选择谓词及 `ReaderRetired` 筛选已在上文单列的 retain 库契约下证明。

[queue_receipt](../rust/queue_receipt.rs) 证明实际 submission／commit 顺序及 producer continuation 更新。完成时先登记匹配的 receipt 与 lease，再向有序 commits 追加 receipt，保持旧 commits 及其他字段。如果 commits 形状非法，规格明确已完成的 submission 前缀，不声称原子回滚；缺失 submission 身份在这些修改之前拒绝。producer 更新仅修改所选 producer 条目。后续 queue_transition 和 queue_history 已组合 task／reader 更新与完整业务历史；缓存语义、请求准入及原生隔离仍是单列调用者义务。

`ENV-JSON-LIBRARY` 仅新增实际 `Value::eq`，该阶段库白名单共二十个接口，上文 retain 阶段增加第二十一个。相等结果保持全部有限深度观察，不承诺序列化字节相同，例如浮点正负零可比较相等。数字观察仍仅包含 `as_u64`。运行器拒绝 `preserve_order`，固定有序 map 观察所需配置，并检查 `float_roundtrip`；没有信任 straw 函数体或完整 reader 操作。[reader／回执记录](../verification/measurements/20261025-queue-reader.json) 绑定实际函数证明、语义负例及安装 wheel 的回归。六个新增工作流使用无依赖关系的新旧 continuation 输入和独立 peer reader，覆盖 cancel／expire／recover 后显式结束读取或 retire worker，仅回收已确认结束的根。这些测试补充局部证明；完整根提取、fencing 及跨日志／GC 细化仍未完成。未增加文件系统操作，也没有新增性能测量。

## queue 的 consumer、batch、checkpoint 与 retention replay

生产 `queue_history::apply` 的六类元数据分支调用 [queue_metadata](../rust/queue_metadata.rs)。Verus 检查实际事件解码和按序更新，并把对应的可选请求回执作为最后一步。规范要求接受已识别且标识符类型正确的事件，不能靠全部拒绝来通过验证。[json_update](../rust/json_update.rs) 证明嵌套叶子插入／删除精确、无关字段保持、无效路径不改变状态；不会自动补造中间对象。

多个 edit 不被假定为内存原子操作。失败时 `outcome` 给出已经执行的精确前缀和下一条无效路径；只有所有业务更新成功后才到达回执更新。原生 commit 隔离和构造失败必须阻止部分状态继续使用。完整调用者对应、其余 task／lease／epoch／reader 事件、调度缓存、queue 原始序列化等价及跨日志所有权／GC 组合仍未闭合。这六类分支不证明自动选择模型 checkpoint，也不证明完整训练 replay 数据保护。

外部库白名单现有二十一个基础接口规范（包含上述借用 str 读取／修改索引）。新增 Map `get_mut`／`remove` 的精确修改与无关字段保持定律，以及 Clone 保持有限深度观察的前提，在 `ENV-JSON-LIBRARY` 中明确标为可信库契约；没有省略 straw 实现体。数字观察仍仅包含 `as_u64`，完整数值往返另有运行回归。

恢复测试构造校验完整、业务关系无效的日志：`BatchReady` 没有对象类型的已规划 batch。旧 wheel 会接受缺失／null batch，或在 boolean batch 上 panic；新代码返回 `UnsafeRecovery`，不追加恢复 epoch。这属于畸形历史加固，没有证明合法 API 能产生此历史。此次提取会创建小型 edit／path 向量，改变可执行分配工作，不能声称零开销或带宽达标；性能工作等未完成的正确性义务关闭后再做。

[queue 元数据记录](../verification/measurements/20261025-queue-metadata.json) 绑定完整正向证明（357 项义务）、17 个新增及 13 个受影响语义负例、73 个原生测试和 909 个安装 wheel 的 Python 测试；两种 Clippy、格式与环境清单检查通过。没有新增 TLC 或性能测试。

## catalog 追加与恢复的业务等价性

[catalog_json_equivalence](../verification/wal/verus/catalog_json_equivalence.rs) 证明与分配无关的 JSON 观察保留全部使用到的 wire 字段；[catalog_equivalence](../verification/wal/verus/catalog_equivalence.rs) 将其连接到实际 `catalog_state::run` 与 `LogState` 历史。staging 更新可以采用不同 HashMap 迭代顺序，但不同 key 的更新得到相同 map；owner、pack、mark、reclaim、receipt 仍保持必要的顺序。

`raw_history_state_equal` 对任意有限历史证明：相同原始 payload 序列、固定 run／namespace 下，两次合法解释得到相同六类索引。`completed_states_equal` 将归纳结论用于实际完整日志状态，同时证明 cursor 与 sequence 相等。该结论连接已验证的追加／恢复函数产生的历史，不要求解析结果的分配或操作顺序相同；前提明确要求状态完整，部分 replay 仍由现有不变量隔离。

两个外部前提单列：`ENV-JSON-VALUE-CONSISTENCY` 保证成功解析后的有限深度 Value 观察一致；`ENV-STRING-KEY` 保证字符内容相同的不透明逻辑 String key 具有同一身份。前者仅观察数字的 `as_u64`，不是完整浮点数模型；两者都不保证解析接受性或自定义 Deserialize 的确定性。没有把整个 straw 适配器改成可信假设。原生命名空间／File／锁、实际保留的物理前缀、完整 Coordinator replay 和 GC 组合仍未闭合。

原生 `append_and_fresh_replay_agree_on_every_business_index` 回归在四个 frame 后分别重开，比较六类索引与确认边界，覆盖共享 staging 图、Unicode 身份、重复集合成员、释放回执及 mark／reclaim 顺序。本次只增加可擦除证明和测试，未改变生产执行体；该阶段的完整 Verus 正向检查通过 348 项义务与 14 个定向语义负例；70 个 native 测试、两种 Clippy 与格式检查通过。没有重新构建 wheel／运行 Python、TLC 或性能测试；确切输入与结果见[等价性记录](../verification/measurements/20261025-catalog-equivalence.json)。

## catalog 追加输入与序列化

生产 `Catalog::append_frame` 现在调用 [catalog_append](../rust/catalog_append.rs)。Verus 检查同一份可执行 `prepare`／`append` 函数体：绑定传入的 run、不改变其他输入字段，解码这些字段，序列化同一个 Value，将对应字节与操作交给共享 writer，并在成功／失败后保持 interpreted history。`LogState::new` 建立空历史基例。成功确认只增加一帧，且字节 cursor、持久边界一致；解码错误保留相对原输入的理由，不得变成写入。

`ENV-JSON-ENCODE` 是单独审计的**外部库**契约。通用序列化关系仅通过一个命名库定律连接到 `serde_json::Value`，没有假定任意自定义 serde 类型可逆。运行器核对二十一个库方法规范、四个不透明类型、透明 Value 枚举、两个具名 Value 库定律（编码与观察一致性）及一个 Map key 唯一性定律，并检查实际编译的 serde_json artifact 已启用 `float_roundtrip`。解析／序列化库内部、接受性／深度／资源限制及编译器仍在证明范围外。上述条件业务等价定理已经连接独立分配的解析值与相同的完整 catalog 索引。原生命名空间／集合／File／锁、追加前路径规范化、完整 coordinator replay 和 GC 组合仍未闭合。

审查这个边界发现真实数值缺陷：旧解析配置会在读取序列化 JSON 时改变部分有限浮点数。实际回归复现了相同 lease 时长配置重开却被拒绝、producer continuation 恢复后变化，以及原生 journal 重放后规范事件字节变化。启用精确浮点解析后，这些场景及反复恢复、同 ID 重试、负零和数值边界均保持一致。旧数据中已经舍入丢失的值不能还原；基于旧舍入值计算的请求摘要，在升级后可能与原始未舍入输入冲突。这项解析修复没有性能结论。


安全性／活性前提审查覆盖实际 `execute_commit`、`wal_recovery::recover`、`gc_batch::collect`，以及 `catalog_history_safe` 和 `gc_liveness::history_valid`。安全性合约只要求当前状态／effect 不变量，没有最终稳定或调度前提；独立的 `eventually_finishes_from_environment`／`eventually_reclaims` 明确要求稳定性与 effect／caller 公平性。这关闭的是现有条件定理的前提分离，不是原生调用历史或完整系统进展证明。


[追加／JSON 记录](../verification/measurements/20261025-catalog-append.json)绑定 323 项完整正向 Verus 义务、12 个新增追加／库语义负例与 13 个受影响回执负例、69 个 native 测试及 906 个安装 wheel 后的 Python 测试。两种 Clippy 与格式检查均通过；没有性能测试或新 TLC campaign。


## 数据退休后的请求重放

实际 `Coordinator` 使用的 `request::{lookup,response,copy_field,remember}` 函数体由 Verus 检查：区分缺失与缓存的 null 回复，拒绝冲突摘要，返回已存回复内容，并精确更新一个回执 key。库契约保证克隆保留规范 JSON 内容与摘要文本，不把分配／布局当作相等。`request_history` 对任意有限 admitted 历史证明重复幂等、其他 key 不变、回执保留及分段 replay 等价。

独立的 `RequestReplay` 检查两个请求 ID、冲突内容、丢失回复、重复恢复和已接受数据被回收。下层 append／sync／apply 仍抽象为原子动作；原生 coordinator 如何建立 admission、key／hash 身份、授权、有序业务 replay 及跨日志所有权不变量，仍是未闭合实现义务。`ENV-REQUEST-REPLAY` 明确允许合法 GC 删除旧输入而持久回执继续存在。

回归在旧版复现 consumer state 保存、batch plan、batch ready 与 checkpoint 注册共 8 个失败场景，分别覆盖原进程与重新打开。修复后先查询回执再访问旧数据；重放保留内容冲突检查，不改变 state、WAL、catalog 或物理删除结果。重复 batch plan 延续现有 API：返回当前 batch 状态。回归不能替代完整 coordinator 证明。


[请求重放记录](../verification/measurements/20261025-request-replay.json)绑定 319 项完整正向 Verus 义务、14 个语义负例、5 个 TLC 案例、68 个 native 测试与 904 个安装 wheel 后的 Python 测试。两种 Clippy 均通过；没有重跑旧负例／模型全集，也没有新性能结论。


## 核心验证：分布式协议、WAL 与 GC

核心审查覆盖这三部分及其交界处。下表是目标性质；下文分别给出已完成的有限组合模型、无界重试证明和实际 Rust 证明。完整适配细化仍未完成，不能把这些层的结果直接合并成全部实现已经验证。

| 核心 | 待补的模型／证明 | 要成立的性质 |
|---|---|---|
| 分布式提交与恢复 | 组合 payload 持久化、catalog retain、队列 WAL 接受、staging 消费与响应；分别保存两份日志的当前／持久状态，允许在每个边界崩溃、丢失响应和重试 | 已接受任务的完整依赖闭包始终受持久 owner 保护；相同请求 ID 的重试只生效一次；冲突内容不能冒充重试；旧 attempt／epoch 不能修改新状态 |
| WAL 与 replay | 显式区分 header、body、trailer、同步、合法残尾、完整损坏、恢复截断及恢复后追加；同步报错允许已经产生效果 | 已确认前缀不丢失；完整损坏必须报错；只截断合法不完整尾部；追加位置和 sequence 对应恢复后的前缀；恢复状态同步之前不能授权依赖它的操作 |
| 跨客户端 GC | 把 queue、reader、checkpoint 的 retain/release 与持久 tombstone、unlink、目录同步、回执和重启组合；不能把一次 catalog 操作直接等同于原子持久动作 | 活跃所有权保护全部依赖；tombstone 阻止 late retain；错误不代表回滚；回执蕴含持久删除；取消／lease 过期不会提前释放仍在读取的客户端 |
| 条件进展 | 对提交、恢复和 GC 分别连接环境最终恢复、调度公平与实际重试转移 | 在明确条件下最终完成或返回规定错误；不能直接假设控制器最终进入成功阶段，再称其为环境进展证明 |

WAL 前缀模型提供两份日志各自的恢复边界；`DurableHandoff` 显式拆开跨日志操作；通用控制器和 GC 的 Verus 证明分别处理失败后的重试。三者使用独立环境契约，实际 I/O、解码和恢复适配到这些模型的对应关系仍需单列。

每项交付需要同时说明：环境允许什么、straw 必须做到什么、有限模型搜索到什么、实际 Rust 的哪些函数经 Verus 证明，以及哪些 I/O／锁／编码适配仍未证明。负向对照必须命中指定安全性或活性反例；真实实现回归必须覆盖对应故障窗口及恢复后的后续操作。模型通过但适配未证明时，仍明确标为 L1／L2 证据，不能称为整个分布式系统已经验证。

实际 Rust 的第一步是让队列和 catalog 共用 `rust/wal.rs` 的追加控制器及 frame 边界判定，再直接验证这些函数体。控制器证明不覆盖调用方是否真的写了指定 bytes、是否调用了正确描述符的 fsync，或 JSON／checksum 解码是否忠实；这些仍需要组合模型和故障回归，完整适配细化仍是待补项。

这一首步已通过 128 项 Verus 正向验证、8 个新增 WAL 语义负例和既有的 WAL-before-retain 负例；40 个原生测试及 111 个 WAL／提交／GC 定向 Python 回归通过。新增回归枚举一帧的所有字节前缀，并在恢复后追加、再次恢复核对事务；它模拟文件前缀，不模拟设备断电。首步检查没有覆盖其余 Verus 负例或完整 Python 套件；后续完整重跑见下段。后续跨日志组合模型见下文；完整实现细化仍未完成。

上一轮按功能整理目录后，曾完整重跑 846 个 Python 测试、40 个原生测试和 7 个 Rust example 的构建；383 个 TLC 案例（157 个正例、197 个算法负例、29 个环境负例，其中 20 个是真正的活性反例）及完整 Verus 检查（128 项正向验证、113 个语义负例）均符合预期。`--area wal` 的 30 个案例另行通过。该轮清单为 43 条契约、43 个消费者、19 个组件层；当前为 65 条契约、49 个消费者、31 个组件层。新布局及运行命令见 [verification 入口](../verification/README_zh.md)。这些结果覆盖本轮实际输入，不补足尚缺的跨日志细化或完整环境进展证明。

这里的分布式协议指多个客户端通过共享存储交接数据和所有权。catalog 锁的串行化，以及接管队列前旧 coordinator 已被停止／fence，仍是必须独立审查的环境契约；不是证明 straw 实现了共识或自动选主。应加入失去这些契约时的反例，不能把外部保证藏在“原子操作”里。

安全性不需要假设故障最终停止；活性需要单独列出该假设和调用方重试义务。没有外部 fencing 的死亡 writer 自动回收不能通过增加超时就变成已证明安全。JuiceFS 实现、锁服务和断电持久性仍是外部契约；p99、100 GB 带宽测量是部署证据，不是正确性或时限定理。

## 实际 GC 批量删除分派

`Catalog::collect_with_stat` 调用实际 [gc_batch::collect](../rust/gc_batch.rs)，Verus 直接检查这个函数体。它将目标索引绑定到固定父目录，逐项检查 mark、执行 unlink，收集包括 unlink NotFound 在内的全部父目录，去重后完成所有目录屏障，最后才写持久回执。对任意有限目标集合，成功意味着全部目标持久缺失且已有回执；报错仍保持“回执只覆盖持久缺失目标”，允许 unlink／同步／回执在返回错误之前已产生副作用。

这比逐个控制器的阶段检查多证明了真实循环的覆盖和组合：共享父目录不会遗漏目标，跳过一个屏障不能提前写回执。合法 marked 批次也不能无故返回内部协议错误。删除统计使用已证明的 checked addition，溢出返回错误并保留重试状态；没有声称统计的完整数学对应或发现了实际部署溢出。

原生适配器现调用 [gc_io](../rust/gc_io.rs) 中实际的错误分类、unlink 和父目录同步分派。只有 NotFound 转为缺失；stat 仅用于统计，不能据此跳过 unlink。存在的父目录必须同步，缺失的父目录必须修复成功。[directory_io::sync_all](../rust/directory_io.rs) 由生产 mkdir 调用，证明完整有序祖先屏障，以及失败时准确的已尝试前缀和先前持久效果保持。提取保持已有故障点和文件操作，没有新增分配或 syscall；这不是实测性能保证。

`ENV-GC-PRIMITIVES` 与 `ENV-DIRECTORY` 展开 `ENV-GC-BATCH-EFFECTS` 的下层前提。原生目标／mark／路径／句柄绑定、完整规范祖先枚举、repair 到持久缺失的聚合关系仍未闭合，完整根集合、catalog 上下文和重试活性也仍需证明。成功的故障 hook 必须保持模型状态；任意用户回调修改文件系统不在该契约内。[GC I/O 记录](../verification/measurements/20261025-gc-io.json) 区分实际函数体证明、语义负例和原生回归，不宣称完整 GC 细化或崩溃 writer fencing 已完成。

[批量删除记录](../verification/measurements/20261025-gc-batch.json)绑定 280 项完整正向 Verus 义务与 19 个新增语义负例。负例覆盖错误／遗漏目标、遗漏父目录、忽略 NotFound 的目录、跳过屏障、提前或跳过回执、吞掉错误、无故拒绝、未检查算术，以及返回值或持久性不实的下层契约。真实多 writer 文件回归覆盖部分 unlink、共享父目录、缺失父目录修复及回执响应丢失。已有有限 GC 模型仍是独立证据，本阶段不声称重跑 TLC。完整安装 wheel 的 896 项测试、63 项 native、两种 Clippy、示例及协议／WAL campaigns 均通过；[回收成本对照](FILESYSTEM_zh.md#gc-批量执行对照2026-10-25)保留了实测变慢的结果。

## 实际提交执行与 apply 中断

`Coordinator::commit` 现在调用 [queue_commit::persist_and_apply](../rust/queue_commit.rs)，直接借用拥有文件的 Journal、业务状态、epoch、顺序、seal 状态、待处理 handoff 和派生缓存。它调用已验证的 Journal append，只在持久接受后记录 handoff，再隔离实际 Journal 状态、应用完整业务事务，并消费私有确认值后解除隔离。合约保留同一 File permission，并暴露每个返回错误对应的准确 WAL 接受字节和部分业务结果。旧 `protocol::execute_commit`／`ENV-COMMIT-EFFECTS` 是仅在验证配置中编译的独立抽象分派器；原生 `CommitIo` 已删除。catalog retain 证据和最终 staging 消费的完整组合仍是未闭合的证明义务，相关运行行为由回归测试覆盖。

[queue_handoff](../rust/queue_handoff.rs) 提供实际 handoff 创建和恢复路径。真实解析器要求标记唯一且位于末尾，将其身份绑定到队列命名空间、序号及之前的全部业务事件，遍历这些事件得到完整引用集合，并只把该恢复意图加入待清理列表。旧版无标记 frame 不产生清理意图。公共 catalog API 拒绝保留的 `queue:` 和 `publication:` 命名空间，队列内部按准确前缀保留所有权，typed GC 继续使用内部入口。该准入检查阻止公共替换或释放内部根，但不单独证明历史 owner 与引用的对应。canonical JSON 身份仅依赖单独审查的 Value 序列化往返及构造器契约，不从 Value 相等或浅层 schema 观察推出哈希输入相等。

已复现一个真实 Rust 缺陷：队列接受已持久化、内存 apply 之前发生 panic，调用者捕获后 `journal.poisoned` 仍为 false。后续 GC 消费 staging，却在内存中看不到存活任务，随后 prune 队列 owner 并删除已接受的输入。修复将隔离标记提前到 apply／callback 之前，只有成功完成后才清除。原生回归覆盖两个 callback 切点、禁止继续操作、恢复原请求及恢复后的数据保护。精确前版源码复现删除一个已接受文件；修复版在 GC 前返回 `UnsafeRecovery`。

`CommitApplication` 单独建模返回错误、捕获 unwind、后续 API 入口屏障、恢复、staging cleanup，以及持久退休先于 prune。六个算法负例均违反 `AcceptedDataPresent`。Verus 证明正常 Result 执行，并要求 apply 保持隔离标记；它本身没有证明 Rust unwind 语义。有限模型和原生 panic 回归分别提供这部分证据。

[提交／恢复阶段记录](../verification/measurements/20261025-commit-recovery.json)保存 272 项 Verus 正向义务、14 个定向语义负例、16 个 apply 中断模型案例、63 个 native 测试及 888 个安装 wheel 后的 Python 测试；Rust／Python 格式、默认／Python Clippy、示例及协议／WAL campaigns 均通过。没有重跑完整旧负例或 TLC 集合。

## 实际 catalog 恢复与持久确认的组合

`Catalog::refresh_with_sync` 现在调用同一生产 `catalog_target::Apply::recover`：先检查已确认下限，按需清空不确定或部分 apply 的索引历史并重建，再运行共享字节恢复，最后确认实际边界。Verus 直接检查这段调用流程；成功返回意味着索引对应有类型历史，sequence 对应 frame 数，cursor、合法截断后的文件长度与持久化下限一致，且没有未完成的 apply／append。任何返回错误都不推进此前的确认下限。空增量 refresh 的既有缓存保留行为不变。

Target 契约现在单独描述“成功时没有部分记录”和“apply 不推进确认下限”；真实 catalog Target 已证明这两点，通用字节 driver 将其传递给组合调用。辅助定理把任意长度的原始 payload 历史、frame 长度和重放后缀边界连接起来。确认调用还显式证明必然成功，避免错误参数仅让恢复多返回一个错误、却逃过只约束成功结果的规格。

这是 `ENV-REPLAY-IO` 下的实现证明：Source 的实际 fd／缓冲／返回值及命名空间／缓存对应仍未证明；后文原始历史章节已在独立解析库／命名空间／集合契约下组合实际 decoder 和 run 检查。它没有完成外层加锁／fresh-open、Journal 业务恢复或跨日志与 GC 的全部组合，也不声称所有业务输入都能成功恢复。

[恢复组合记录](../verification/measurements/20261025-catalog-recovery.json)记录 276 项正向 Verus 义务、14 个新增与 57 个受影响的既有语义负例，另有 63 项 native 和 888 项安装 wheel 测试，默认／Python Clippy、格式、示例、协议／WAL campaigns 均通过。本轮没有重跑 TLC 或全部其余 Verus 负例；32 轮恢复成本对照见 [FILESYSTEM](FILESYSTEM_zh.md)。语义负例覆盖跳过 rebuild、使用错误 cursor／sequence／日志种类、忽略确认下限、只读恢复、跳过／错误确认、失败屏障后仍确认，以及下层遗漏完成状态／上下文／持久性的契约。运行器分别验证实际调用方或共享 driver；工具资源耗尽不算语义反例。

## 采样与训练续读边界

`tests/test_resume_workflow.py` 启动独立采样进程并直接退出，不执行 close／析构。新进程恢复最后一次已提交的 `save_task_progress`／`yield_task` 引用；仅发布 continuation 不改变任务进度。旧 attempt 被拒绝，完成结果重试不重复接收，未结束的 active lease 按既有重试预算处理。如何解释 continuation 的 tokens／RNG，以及选择哪个已提交外部模型 checkpoint，由应用负责。

训练回归开启在线 GC，并在不可变 checkpoint **之后**产生新结果。后续 processed 游标可能领先模型 checkpoint；若没有独立 replay owner，GC 会删除这些已处理结果，回退消费者位置时会被拒绝。保留 replay owner 后，先恢复 checkpoint 对应的 consumer state，队列重新持有数据，再释放额外 owner，重放结果与不中断的 mock optimizer 一致。这验证的是显式恢复协议，不是自动选择 latest checkpoint，也没有证明真实训练框架正确。

`TrainingResume` 检查 2、3、4 个位置的应用协议，并为使用队列最新游标、缺少 checkpoint 后数据保护、提交不匹配的模型／数据位置保留反例。`ENV-TRAINING-CHECKPOINT` 明确实现责任属于应用。模型使用有序位置、每个 optimizer step 对应一条输入；分布式 optimizer／RNG、稀疏 batch 和外部 manifest 发布仍在边界之外。

## 实际 WAL 字节 framing：部分实现证明

[`rust/framing.rs`](../rust/framing.rs) 现在由 queue 和 catalog 的真实编码／replay 路径共同调用。Verus 检查这些相同函数体：u64 小端编解码往返、24 字节 prefix、56／40 字节 header／trailer 布局、magic、sequence、声明长度、digest 比较，以及不完整 header 的合法 magic 前缀。`encoded_header_is_accepted` 将编码函数后置条件连接到解析判定；十个语义负例分别删除校验或破坏字段编码／解码。

这仍是字节 framing 的部分证明，**不是完整 WAL 恢复证明**。下层仍需证明 Device 确实对应实际字节及同一日志，上层仍需证明 JSON 解码到业务状态。SHA／serde 原语为可信边界；后续共享字节 driver 的进展及其剩余适配义务见下一节。原生测试另行检查既有格式兼容性，并逐字节损坏 header／trailer；执行证据不替代证明。


## catalog JSON 字段解码

catalog 是持久化在 `storage.log` 中的共享数据所有权账本。队列 WAL 记录任务状态；catalog 记录 queue、reader、checkpoint 或 publication 对数据 pack 的保护，以及 sealed、删除和交接回执。因此跨日志提交与 GC 都使用它；只证明 catalog 不能替代这两条完整协议的证明。

实际 [catalog_json.rs](../rust/catalog_json.rs) 已接入 catalog replay。Verus 证明解码成功当且仅当输入符合其字段类型规范，且输出的每个字符串、map key、列表位置、布尔值和字段存在标记都对应输入。缺失的可选字段生成空更新；字段存在但类型错误必须拒绝。由此修复 replay 将畸形 `owners`、`packs`、`deleted` 当作空更新接受的问题。未知字段仍为兼容性而忽略；路径、受支持的编码版本、graph 身份及回执语义另行校验。

[`ENV-JSON-LIBRARY`](../verification/common/verus/serde_json.rs) 单列十一个 serde_json 基础访问／克隆／映射接口及四个不透明库类型，实际 Value 枚举透明的可信规范。运行器用 Rust 1.99.1 编译实际锁定依赖，记录库与输入哈希，并审计独立白名单。straw 解码函数体没有省略，也没有假设完整业务结果。字节到 `Value` 的成功解析关系与 catalog 历史已组合，catalog 业务唯一性见前文；原生命名空间适配和完整 Coordinator／GC 组合仍未闭合；有类型操作构造的条件证明见下节，原始 Target／history 协议见对应章节；这还不是完整业务 replay 证明。

[catalog JSON 阶段记录](../verification/measurements/20261025-catalog-json.json)通过 196 项正向验证、8 个算法语义负例、2 个库环境语义负例、52 个 native 测试及安装 wheel 后的 883 项 Python 检查。15 个新增畸形字段用例均在精确前版 wheel 上复现。该记录是本轮 JSON 定向证据，不是新的一次完整 Verus 负例或 TLC 检查。

## WAL 效果之前的 catalog 校验

`Catalog::append_frame` 现在先解码并校验整批操作，再标记不确定状态和执行 write／sync。同步后使用同一批已准备操作，不再重复 JSON 解码或命名空间查询。非 object 输入在 writer 修改 JSON 字段前拒绝。这修复了内部写入边界的问题：无效事件可能先被完整同步，再被 apply 拒绝，留下以后 replay 无法接受的持久 frame。原生回归直接调用内部入口，并不代表已经发现公共 API 会生成这些事件。

实际 `Indexes::validate` 证明成功当且仅当顺序 `apply` 能接受整批操作；`apply` 现在也证明同一充要条件，并保留失败时的精确前缀契约。校验不借用可变索引，只为本事件此前出现的 mark 保存临时集合。既有 tombstone 和此前 mark 可以授权 receipt，后出现的 mark 不可以。这是既有有类型索引准入规则的证明，不是完整逻辑所有权或 GC 持久性的证明。

`CatalogTransaction` 区分有效事件与永久无效输入：拒绝不产生 WAL 效果，当前事务仍可用，并在模型的公平性前提下到达错误终态；跳过写前校验会违反该性质。十六个案例保留原有故障窗口、guard 和环境负例。实际解码到模型的绑定及完整 lock／I/O 调用链仍需细化。

[catalog 写前校验阶段](../verification/measurements/20261025-catalog-admission.json)记录 199 项 Verus 正向验证、6 个定向语义负例、16 个 catalog 事务案例、54 个 native 测试及安装 wheel 后的 883 项 Python 检查。模型显式记录无效输入的错误结果；把拒绝伪报为成功另有独立语义负例。

实际 [catalog_event.rs](../rust/catalog_event.rs) 继续验证语义检查阶段：handoff ID 必须恰为 64 个小写十六进制 UTF-8 字节；staging key 必须含精确 `staged:` 前缀及这样的 ID。出现 receipt 字段时必须有 owners 字段，且只允许 release；新增图节点及每个子节点名称均须合法，更新还须满足已证明的图兼容判定。各真实函数均证明接受与拒绝的充要条件，缺失 receipt 字段保持旧格式行为。这些函数由原生 replay／append 和 handoff 流程调用；该阶段没有覆盖路径解析及操作构造；后续条件组合见下节。[catalog 语义检查证据](../verification/measurements/20261025-catalog-event.json)单列该范围。

本阶段通过 252 项 Verus 正向义务、12 个定向语义负例、60 个 native 测试和格式，以及安装 wheel 后的 202 个 WAL／receipt／图／catalog／handoff／在线 GC 回归。前一阶段的完整 883 项测试仍归属于独立 catalog Target 输入。更正：归档中的默认／Python Clippy 日志有两条风格检查失败，旧摘要误写为通过；原日志保留，公开阶段记录已更正。

## catalog 字段到有序业务操作

实际 [catalog_decode.rs](../rust/catalog_decode.rs) 将字段解码、receipt／图准入和操作构造组合起来，原生 replay 与 append 调用同一份实现。成功合约将每个 owner 路径、release、sealed 标记、tombstone、回收、依赖边和 handoff receipt 绑定到输入。owner 路径在固定命名空间契约下规范化并去重；mark 在 reclamation 前，receipt 在全部更新后。图的消费迭代保留每个条目，但不规定 HashMap 顺序。失败不返回部分操作序列；schema、receipt、图和 encoding 错误均有明确条件，路径错误仍取决于 resolver。

三个边界可以单独审查：`ENV-CATALOG-RESOLVER` 要求成功路径结果准确且抽象命名空间不变；`ENV-SET-BUILDER` 现已在单列的标准 `BTreeSet::from_iter` 窄库定律下验证实际 `NativeKeySet` 适配；`ENV-COLLECTION-ITERATOR` 信任四个标准 HashMap/HashSet 迭代原语和两个不透明类型。运行器单独审计库规范。straw 解码／迭代函数体直接验证，没有假设完整 decoder 正确。原始解析／run 检查与 Target 业务含义已在后文原始历史章节组合；实际 append 序列化／历史绑定见前文；原生路径／缓存和完整持锁事务仍未闭合。

[操作构造阶段记录](../verification/measurements/20261025-catalog-decode.json)保存精确输入和定向语义负例。生产循环移动已有 key／value，保留批量集合构造；ghost 历史与证明计数器被擦除。运行性能单独测量，不能由这些证明推出。

操作构造阶段通过 270 项 Verus 正向义务、19 个定向语义负例、62 个 native 测试、883 个安装 wheel 后的 Python 测试、示例和协议／WAL campaigns，以及 Rust／Python 格式和默认／Python Clippy。[64 轮 replay 对照](FILESYSTEM_zh.md#catalog-操作构造性能对照2026-10-25)单列性能证据；本阶段未重跑全部旧负例或 TLC。

## 实际 WAL 追加与 catalog 确认

两个日志现在都调用 [wal_append.rs](../rust/wal_append.rs) 的实际 header／payload／trailer／sync 循环。Verus 合约核对 descriptor 观察到的长度和 cursor 是否匹配预期追加边界，在写入前拒绝序号／frame 溢出，证明成功时旧前缀不变，并将每段写入字节及 digest 范围绑定到输入 payload。新增 frame 满足已验证恢复器使用的同一个 `complete_frame` 判定。只有同步成功才能完成循环；错误响应仍可伴随部分效果。

实际 `LogState::append_to` 组合准入、上述字节写入及已保存计划的 apply。成功时，索引的有类型历史、cursor／sequence、已确认下限与 Writer 的持久字节一致；写入错误保留 pending 计划，并禁止继续使用。由此在 Writer 接口处闭合了追加顺序与确认的证明，包括提前确认和吞掉错误的负例。

[`ENV-APPEND-IO`](../verification/environment_contracts.json) 单独记录外部字节／位置／同步／SHA 保证，以及 straw 的原生 Appender 义务。实际适配器借用原 File，并保留既有 fault hook 位置；其原语返回值和 descriptor 身份仍需对应证明。JSON 序列化、原始 payload 到操作的绑定、锁和完整 Journal 业务状态继续单列。原生 cursor／长度检查会在观察不一致时拒绝追加，这不构成文件系统可见性延迟上界。

[WAL 追加阶段](../verification/measurements/20261025-wal-append.json)记录 225 项 Verus 正向验证、14 个定向语义负例（11 个算法、3 个 Writer 环境）、60 个 native 测试及 883 个安装 wheel 后的 Python 测试，并通过 clippy、示例及协议／WAL campaigns。hash 范围负例检查实际独立 encode 函数体；出现资源限制的原型运行不计入通过证据。此次未重跑全部旧负例或 TLC。

## catalog 联合状态与有类型 frame 历史

实际 [catalog_log.rs](../rust/catalog_log.rs) 统一持有六类持久索引、cursor、sequence、已确认下限、rebuild、部分 apply 状态和待完成追加操作。catalog 调用方只能只读访问索引，不能分开修改这些状态。Verus 检查 refresh／append 使用的同一份状态转移及成功充要条件；永远拒绝有效操作的实现不能满足这些合约。

可擦除的 ghost 记录描述机器边界内任意有限的有类型 frame 历史，生产执行不分配该历史。每个完整状态的索引等于该历史的顺序 fold，sequence 等于 frame 数量，cursor 等于含 framing 的总字节数。部分 apply 报错时记录精确的已执行操作前缀，cursor／sequence 不前进，且不能确认、继续 replay 或追加。rebuild 清除待确认历史、索引和追加计划，保留已确认下限。恢复确认必须同时匹配实际 replay 的 cursor 和 sequence；追加保存已准入操作，并且只应用该计划一次。

实际 replay Source 现在独占借用原始可变 File，丢弃缓冲、truncate、sync 和最终 seek 都使用该 descriptor。原有路径替换、失败屏障和字节回滚回归覆盖这条路径。Rust 借用检查与这些执行证据仍不能替代完整 Device View 或成功读取的快照语义证明。

本节证明实际 catalog 状态及有类型历史；后文原始历史章节补充了条件化的 payload 到业务操作对应，原生物理持久性仍需连接。append_to 已在 Writer 下证明追加屏障／确认顺序，Apply::recover 已在 Device 下连接恢复屏障／确认位置，实际 Source／Appender 流程现已在前文 File／Buffer／Hooks 契约下证明，原生薄封装与外层上下文对应仍未闭合。路径转换、closure cache、完整锁流程、Journal 业务状态和上层协议／GC 组合仍未闭合；`ENV-REPLAY-APPLY`、`ENV-VERUS-CATALOG` 保留未证明适配状态。

[catalog 联合状态阶段](../verification/measurements/20261025-catalog-log.json)记录 220 项 Verus 正向验证、14 个定向语义负例、58 个 native 测试、883 个安装 wheel 后的 Python 测试，以及 default／Python clippy、示例和协议／WAL campaigns。此次没有重跑全部旧负例或 TLC，不能将历史结果计作本轮完整套件。

## catalog 原始历史与 replay 应用

实际 [catalog_target.rs](../rust/catalog_target.rs) 替代了 catalog replay closure。其 `View` 直接投影实际 `LogState` 历史，不另造一份记录列表。记录与待完成追加计划的可擦除 ghost 字段现在绑定精确 payload 字节；此前只跟踪长度。这不增加运行时 payload 复制或历史字节保留。

Verus 检查成功 apply 恰好追加所传入的字节，并保持联合日志／索引不变量。适配器将当前 staging graph 交给 decoder，再通过已验证的 `replay_record` 应用其返回操作；解码失败、无效操作和不完整状态仍返回错误。共享恢复循环要求并保持 Target 有效性，包括错误返回。既有精确部分 apply、rebuild 和已确认下限性质继续成立。

最初的 Target 阶段只证明原始历史保持，未建立 decoder 业务含义。实际 [catalog_frame.rs](../rust/catalog_frame.rs) 现在替代 `EventDecoder`：成功解码绑定精确解析字节、匹配的 run、字段 schema、语义检查、解析后的身份和构造操作。`Decoder` 接口带有这项后置条件，由该生产函数体在独立 serde、Resolver 和 KeySet 契约下建立。

`catalog_target::interpreted` 将每条原始 frame 与其操作及此前 staging graph 关联；`interpreted_at` 覆盖任意有限历史，`business_fold` 证明完整索引等于这些业务操作的顺序 fold。恢复同时保持确认下限和 run／命名空间解释。部分 apply 失败保留此前完整历史，rebuild 在重放前清空历史；实际 decoder reset 清除 closure cache，命名空间 reset 必须保持同一解释。

创建 `Apply` 时，初始或增量缓存历史必须满足该解释；原生创建／append 调用方、JSON 序列化及路径归一化仍须建立这个前提。原生命名空间／初始缓存对应、Source／Appender、完整外层锁、Coordinator 业务 replay 及跨日志／GC 组合仍未闭合。serde 成功解析是可信关系，不含有效输入必定接受的定理。复现完整正向证明及定向反例：`check_verus.py --case catalog_frame_skips_run --case catalog_frame_wrong_bytes --case catalog_frame_drops_operations --case environment_catalog_decode_meaning_unbound`。本次扩展没有启动性能测试。

[原始解码验证记录](../verification/measurements/20261025-catalog-frame.json)绑定 307 项完整正向 Verus 义务、14 个语义负例、66 项 native 测试和 181 项安装 wheel 的定向回归。两种 Clippy、格式及环境清单通过。另有 7 个 writer 生命周期 TLC 案例覆盖死亡后的在途完成；对当前源码补跑的 15 个环境负例（与上述 14 个重叠 1 个）及 47 项回执／消费故障回归也通过，检查不确定副作用和成功响应边界。本次没有新增性能结果。

[catalog Target 阶段](../verification/measurements/20261025-catalog-target.json)记录确切输入和检查。反例覆盖漏 apply、错误 payload、吞掉解码／apply 错误、改变记录／计划字节、丢失 Target 不变量，以及既有联合状态和追加失败边界。

该阶段通过 241 项 Verus 正向义务、26 个定向语义负例、60 个 native 测试、883 个安装 wheel 后的 Python 测试，以及格式、默认／Python clippy、示例和协议／WAL campaigns；不将旧的完整负例／TLC 结果自动视为当前输入的证据。

## 共享字节 replay、catalog 索引与 GC 快照

两份日志实际调用 [wal_recovery.rs](../rust/wal_recovery.rs)。`read_frame` 证明 SHA 输入分别是 24 字节 prefix、56 字节 header 加完整 payload；`recover` 证明接受记录的顺序、合法残尾、sequence／边界和已确认下限；`finish` 检查丢弃预读、truncate、sync、seek 的顺序。这些定理依赖 [wal_device.rs](../rust/wal_device.rs) 的 Device／Target 接口。SHA 原语是可信边界，真实 [Source](../rust/wal_file.rs) 的 descriptor／buffer View 和 JSON callback 仍需细化。

共享 driver 现在也约束每一种格式错误分支。`format_result`／`malformed_at` 分别判定残缺 magic、header 字段／摘要、非法长度和 trailer 字段／摘要；`recovery_malformed` 将 cursor、序号耗尽及已确认下限错误绑定到实际接受前缀。设备错误单独处理，允许存储回滚。格式错误保持原字节和持久性，因此不能把完整损坏帧静默截断。业务 apply 报错仍可能留下部分效果，其恢复义务单列。

`complete_decodes`／`incomplete_decodes` 排除成功 Device 操作下对合法帧的错误拒绝。对 u64／usize 边界内任意有限日志，`legal_log_has_no_format_error` 证明：合法记录序列加允许的残尾，在已确认下限被覆盖时，不会产生格式错误；`legal_log_recovers_exactly` 证明成功恢复得到的 payload 序列与该日志完全一致。唯一性与前缀引理将逐帧解析连接到整条字节日志；这尚不等于 JSON payload 到 queue／catalog 业务转换的证明。

已复现旧证明缺口：将恢复实现改成始终报告 cursor 非法，旧版 225 项正向义务仍通过；新合约拒绝同一变异。这是旧规格约束不足的证据，不代表原始运行时代码存在无条件拒绝缺陷。[恢复完备性证据](../verification/measurements/20261025-replay-completeness.json)记录确切输入及定向算法／环境对照；工具错误与资源限制不计为成功负例。

本阶段通过 238 项 Verus 正向义务、29 个定向语义负例、60 个 native 测试、格式检查及默认／Python clippy；未据此声称重新运行了安装 wheel 后的 Python 或 TLC 全集。


[catalog_state.rs](../rust/catalog_state.rs) 是 catalog 正常 apply 与恢复使用的同一份实际 Rust 索引实现，具有完整的六类持久索引 `View`：owners、packs、永久 tombstone、pending deletion、staging edges、consumed handoffs。`apply_one` 对应每个有类型操作；`apply` 证明任意有限操作序列的顺序 fold，失败时明确给出已应用前缀，且失败的 reclaim 没有额外效果。重复 tombstone 不会重新安排已有 receipt 的删除工作。JSON 解码先生成操作，再应用；跨 frame 的未确认修改仍依靠 rebuild 清除。

`Indexes::roots` 证明枚举全部 owner 集合，没有按 owner 名称筛选；`Indexes::candidates` 证明实际 HashMap 快照生成的候选恰好是 sealed、无 owner 且无 tombstone 的 pack。两个实际循环均检查终止，不再以“调用者传入准确 PackStatus”替代快照证明。collection 和 reactivation 已调用这些函数。逻辑上的 queue／reader／checkpoint／publication 是否都建立了完整 owner，仍是上层协议及闭包证明义务。

本次调用链检查复现了两类缺陷：追加结果不确定后仍可继续使用 catalog；持有原池锁时可用另一 pool/run 配置打开或操作别的日志。实现现在在写入前标记 rebuild，检查每个入口的有效事务与 pool/run，拒绝嵌套事务，并在 fresh open 后核对设备号／inode。`identities::same_pool`、`same_log` 验证实际比较函数；完整 lock/open/guard 调用位置仍需证明。外部不得在活动事务期间替换共享日志 inode；入口检查不构成分布式 fencing。

[catalog 索引阶段](../verification/measurements/20261025-catalog-indexes.json)检查包括 182 项 Verus 正向验证和 17 个定向语义负例；负例分别漏掉索引更新、吞掉 apply 错误、漏掉 owner/root、伪造 owned/tombstone 快照或忽略 pool/run/device/inode。50 个 native 测试通过。历史 helper／TLC 检查不能自动当作本次完整套件的结果。以上覆盖字节执行函数和实际索引／快照，**尚未闭合完整 JSON 业务 replay、Source、事务调用链、closure cache、coordinator 与物理 GC 的组合细化**。

Device 接口现允许失败的 read／seek／truncate／sync 任意改变字节及持久状态；driver 必须传播错误，不能授权恢复状态。Source 在可失败效果前使缓存快照失效，报错后拒绝复用。另有原生回归捕获 Rust 追加 panic，检查 Journal 仍为 poison，并分别恢复不完整及已同步但未返回的尾部。这些是适配器执行证据，不是完整 Source／Journal 细化证明。

`consumption.rs` 的实际 staging graph 校验现在逐项检查所有新增节点、每个子节点定义，以及不可变子节点列表的一致性。`same_children` 比较完整有序字符串内容，`compatible` 在 replay 应用边之前检查整个 HashMap 更新。Verus 验证这些真实函数体，八个定向负例覆盖五个图检查及三个既有子节点覆盖检查。运行器显式使用与 Cargo 相同的 Rust 2024。JSON／key 语法、认证 manifest 绑定和完整 graph/history 细化仍是独立义务。

[后续 WAL 错误／图校验阶段](../verification/measurements/20261025-wal-errors-staging-graph.json)通过 186 项正向验证、8 个图语义负例、52 个 native 测试及安装 wheel 后的 868 项 Python 检查。此前 20 个 replay 错误语义负例保留为独立执行，不算作新的完整负例套件。

## WAL 持久化与恢复前缀

[`wal/tla/WalRecovery.tla`](../verification/wal/tla/WalRecovery.tla) 将每帧分成 header、payload、trailer 三个抽象单元，并独立保存当前前缀、已同步前缀、客户端已确认事务、replay 位置及下一次写入位置。环境 [`EnvironmentWal.tla`](../verification/wal/tla/EnvironmentWal.tla) 允许崩溃保留两条前缀之间的任意长度，包含不完整帧；写入／同步报错不代表没有效果。安全检查不要求故障最终停止，终止检查另加最终稳定和恢复／重试调度公平的条件。

四个正例覆盖一至三帧及完整损坏。八个算法安全负例覆盖提前确认、漏写 payload、忽略错误继续使用、保留残尾、接受完整损坏、把完整损坏截掉、跳过恢复同步和错误 seek；一个停滞变体必须产生真正的活性反例。三个只改环境的负例分别允许丢失已确认前缀、同步虚假成功和认证虚假成功。共 16 个案例符合预期。

这个模型检查恢复后继续追加的边界，并不把三个单元称为实际字节解析证明，也不证明业务请求只生效一次。字节前缀／损坏检查由 `wal/checks/crash_campaign.py` 与 `tests/test_wal_recovery.py` 提供实际执行证据；checksum、JSON、文件描述符绑定和跨日志协议细化仍有独立边界。下述 `DurableHandoff` 增加跨日志的有限组合检查，仍不等同于 byte／Rust 适配细化。

## 独立日志、交接与 GC 的组合

[`DurableHandoff.tla`](../verification/protocol/tla/DurableHandoff.tla) 从已持久发布并有 staging owner 的封存输入开始，分别记录 catalog 和队列日志的当前／持久状态。每次 append 都经过 header、body、trailer、同步和响应，完整但未确认的 frame 可以在任一日志中独立幸存。恢复必须建立新的持久屏障，才能用恢复结果跳过 retain 或推进清理。文件当前存在与目录项已持久存在另行记录。

模型组合接受、消费 staging 并记录回执、响应、持久退休、prune、mark、unlink、目录同步、回收回执和崩溃；reader、checkpoint、peer 可以交错获得或释放保护。七个正例覆盖一／两个 pack、三种独立 owner、三者同时参与和永久 reader；全部参与的配置检查 257,226 个状态，永久 reader 配置检查 4,990 个状态。十三个算法安全负例覆盖跨日志顺序、漏掉依赖、恢复屏障、删除屏障和 late-retain fencing，一个算法活性负例停止重试。六个环境负例分别丢失 catalog／队列确认前缀、虚假目录同步、取消动作公平、取消最终稳定、阻止应用 holder 结束。27 个案例均得到预期结果。

应用完成／释放与文件系统恢复、调度公平分开。`PermanentActors` 可以永久持有，`ReadersProtected` 仍必须成立；只有应用持有者最终结束时才要求全局回收。另行检查每个永久无主 pack 的最终回收。故意忽略应用释放前提而要求无条件回收，会得到真正的活性反例。

实际 `tests/test_durable_handoff.py` 的 22 个故障窗口覆盖 retain、队列接受、Apply 和提交后清理。两个队列传递带独立 dependency pack 的输入，reader／checkpoint owner 保留到明确释放；最后回收后再次恢复、重放相同请求和拒绝冲突请求。检查只出现一次 Submitted、不再打开被回收的 manifest、不产生新 owner。故障回归通过；它是错误返回／关闭后恢复的执行证据，进程直接退出另由 crash campaign 检查。

`EnvironmentCrossLog.tla` 独立声明完整 frame 的恢复投影和持久化响应。三个 frame 单元与逻辑 snapshot 的对应关系未做机械细化；真实锁、checksum、JSON、writer 重启和任意请求历史仍是边界。本模型有一个交接和有限次 owner 获取／释放；同一引用后来的 restaging 由 `CommitCleanup` 单独检查，旧 attempt 与冲突 completion 由 `Completion` 单独检查。有限组合检查不等于已经证明任意队列／依赖图或无界工作流；无界所有权与重试定理具有自己的前提。

## 验证分层与当前覆盖范围

这些层级区分证据的性质，不代表完成度评分，也不表示覆盖了整个系统。

| 层级 | 已验证／检查 | 未覆盖的边界 |
|---|---|---|
| L0：契约与环境 | 明确持久性、可见性、所有权、fencing、失败返回的契约；见本文及 [FILESYSTEM](FILESYSTEM_zh.md) | 文件系统、OS、硬件、锁、哈希、外部 fencing 是假设；失败的操作可能已经产生效果 |
| L1：有限协议搜索 | TLC 检查读取可见性、所有权转移、reader/checkpoint/回退/GC、小整数、usage 缓存、目录创建／重试、GC 屏障、lease 筛选及 completion／恢复；错误变体必须产生指定反例 | 仅覆盖配置的状态空间；没有从 `.tla` 到 Rust 的机械细化证明 |
| L2：无界数学证明 | Verus 证明任意 owner/pack 集合、任意长度的 retain/release/mark/unlink 模型历史保持所有权安全；控制器的阶段界限、顺序和条件最终完成，以及从稳定 I/O、弱公平推导 GC 删除重试活性 | 串行持久 catalog 动作及完整闭包仍由模型提供；真实 I/O／锁／重试适配层与整体系统进展未证明 |
| L3：实际 Rust 一致性 | 实际 `wal.rs` 追加控制器／frame 边界判定、`typed.rs` layout 相等判定循环、`access.rs` 泛型 token 决策；直接证明 `consumption.rs` 缓存图子节点覆盖循环、`incomplete.rs` 清理控制器／PID 比较、`identities.rs` 缓存／编码判定函数、`pending.rs` 描述／同步状态函数、`finalization.rs` stage／seal 控制器及泛型 handoff 准入／确认、`holding.rs` 并集循环、`directory.rs` 父目录响应分类及屏障控制器、`discovery.rs` 发现决策、`bounds.rs` 函数、`protocol.rs` 提交／删除／receipt 转移及实际 `execute_commit` 效果分派、完整候选选择、恢复授权检查和 lease 过滤去重循环，`reclamation.rs` 索引修改、`stream.rs` 长度状态转移、`replay.rs` 持久前缀 guard／预读容量和泛型 `traversal.rs` 队列操作符合契约／`View` | String HashSet／HashMap key 及 BTreeSet 排序定律是显式前提；原生 CommitIo／GC 效果实现、完整依赖遍历、JSON 字节／路径／业务适配、具体 WAL Device/Target 适配与事务调用链未经 Verus 证明；实际索引和 HashMap 快照循环已单列验证 |
| L4：实现集成 | Rust/Python 回归、字节前缀／损坏测试、提交崩溃窗口、late-retain 拒绝、GC 目录同步失败重试 | 是执行证据，不是对任意 Rust 执行的证明，也不是断电认证 |
| L5：部署与性能 | 四客户端可见性、并发 GC、裸文件／队列带宽，以及真实单次 100 GB 原生发布和读取；见 [FILESYSTEM](FILESYSTEM_zh.md) | 不提供通用延迟上界、冷缓存保证、带宽 SLA 或 TB 级实际 I/O 结论 |

尤其要区分 L2 和 L3：catalog 所有权安全证明针对模型；候选选择和提交／删除控制器已有实际 Rust 一致性证明。外围 catalog 与队列适配代码仍靠审查和故障测试连接。入口是 [`verification/common/verus/verus.rs`](../verification/common/verus/verus.rs)，模型定理位于 [`verification/protocol/verus/control_plane.rs`](../verification/protocol/verus/control_plane.rs)。

此前按工具分类的[目录整理](../verification/measurements/20261025-verification-layout.json)曾完整重跑：823 个 Python 测试、40 个原生测试、全部七个 Rust example 的构建、349 个 TLC 案例，以及完整 Verus 检查（119 项正向验证与 105 个预期语义负例）。清单仍为 42 条契约、41 个消费者、19 个组件层。生产 Rust／Python 与 native 二进制未变，Verus 入口仅调整相对源码路径；这些是本轮新检查，与下文历史沿用证据分开。

此前[离线别名修复记录](../verification/measurements/20261025-offline-alias.json)通过 823 个 Python 测试、349 个 TLC 案例（25 个环境对照、18 个真正活性反例）及 119 项 Verus 正向验证。清单含 42 条契约、41 个消费者、19 个组件层。未变的 40 个原生测试与 105 个 Verus 语义负例经精确输入核对后沿用；Python 路径轨迹实现仍是未证明的适配边界。

此前[离线清理修复记录](../verification/measurements/20261024-offline-guard.json)重建 wheel 并通过 804 个 Python 测试，重跑 332 个 TLC 案例（24 个环境对照、17 个真正活性反例）及 119 项 Verus 正向验证。清单含 41 条契约、40 个消费者、19 个组件层。Rust／native 二进制未变，40 个原生测试与 105 个语义负例经源码核对后沿用；本轮 Python guard 的执行证据与未证明的完整细化单独列出。

此前[名称与数据重试验证](../verification/measurements/20261024-read-lookup.json)重跑 316 个 TLC 案例（23 个环境对照、16 个真正活性反例），以及 119 项 Verus 正向验证。清单含 40 条契约、39 个消费者、19 个组件层。生产源码未变；782 个 Python／40 个原生测试及 105 个 Verus 语义负例沿用同一源码的既有证据，新四机测量单独归档。

此前[描述符复用验证](../verification/measurements/20261024-single-open.json)重跑 782 个 Python、40 个原生测试，302 个 TLC 案例（22 个环境对照、15 个真正活性反例），以及 119 项 Verus 验证和 105 个预期语义拒绝。清单仍为 39 条契约、19 个组件层，消费者增至 38 个；实际 frame 定位算术与尚未整体证明的描述符适配分别记录。

此前[typed 提前校验记录](../verification/measurements/20261024-typed-preflight.json)完成 782 个 Python、38 个原生测试，291 个 TLC 案例（21 个环境对照、14 个真正活性反例），以及 118 项 Verus 验证和 101 个预期语义拒绝。清单含 39 条契约、37 个消费者、19 个组件层。四机 8,400 次调用及同记录交错对照、正常 GC 均通过；新的真实 Rust 匹配循环与 JSON／Python／I/O 适配边界分别记录。

后续[合法小切片归因](../verification/measurements/20261024-read-latency.json)增加测量探针和 `ENV-PERFORMANCE` 证据链接；生产代码、环境契约语义和证明输入未变。重跑读取相关的 48 个 TLC 案例及 118 项 Verus 正向验证，单独保留 14,400 次新读取和正常 GC 的记录。没有重跑全部集成测试／Verus 负例，没有把 libc 调用计时升级为延迟保证，也没有新增完整 I/O 细化证明。

此前[固定 tensor 粒度记录](../verification/measurements/20261024-tensor-granularity.json)重跑 761 个 Python、38 个原生测试，以及 282 个 TLC 案例（20 个环境对照、13 个真正活性反例）、116 项 Verus 检查和 96 个预期语义拒绝，均符合预期。清单有 38 条契约、36 个消费者、19 个组件层级。真实 100 GB 四机与 Python 2 GiB 门槛验证、正常 GC 均通过；实际 span 算术有证明，Python producer、完整 I/O／恢复适配仍未证明。

此前[位置相关范围读取记录](../verification/measurements/20261024-positioned-range.json)完成真实 100 GB、四机逐字节核对及 GC；该轮 273 个 TLC 案例（20 个环境对照、13 个真正活性反例）符合预期，115 项 Verus 检查和 92 个预期语义拒绝通过。清单有 38 条契约、35 个消费者。生产代码及 20 个 Verus 输入不变；核对 67 个生产／测试文件后复用 754 个 Python／38 个原生测试证据，新增 example 另行编译、运行和 lint。

[队列大记录工作流](../verification/measurements/20261024-queue-record-workflow.json)完成实际 100 GB 写入、四机分片认证读取、取消后的 reader 保留与逐个确认后的 GC；另测 0／128／1024 条历史，每组 80 次恢复。该轮 256 个 TLC 案例（十九个环境对照、十二个真正活性反例）符合预期，115 项 Verus 检查与 92 个预期拒绝通过；清单有 38 条契约、34 个消费者。生产 Rust／Python 未变，逐项核对后复用此前 754 个 Python／38 个原生测试证据；新探针另行编译、运行及 lint。

此前提交后清理的完整验证见 [提交后清理记录](../verification/measurements/20261024-commit-cleanup.json)：754 个 Python、38 个原生测试通过；115 项 Verus 检查、92 个预期语义拒绝（四个只改环境），240 个 TLC 案例（十七个只改环境、九个活性反例）符合预期。清单有 38 条契约、33 个消费者。四机旧版 96 例中 64 例留下 staging owner；新版 288 例全部清理，独立 reader 保留，无人工数据删除。完整适配／恢复细化仍未证明。

上一轮完整验证见 [writer 访问记录](../verification/measurements/20261024-writer-access.json)：717 个 Python、35 个原生测试通过；112 项 Verus 验证及 88 个预期语义拒绝（四个只改环境），224 个 TLC 案例（十六个只改环境、七个活性反例）符合预期。清单有 37 条契约、32 个消费者。四机旧版 96 例在观察窗口内停住，新版 480 个流程通过；新版 GC 正常删除 960 个文件，无人工数据清理。token 决策属于实际 Rust 证明，mutex／GIL／FFI 和完整适配不在该证明内。

此前的大文件组合证据见 [100 GB 四机工作流](../verification/measurements/20261023-shared-record-workflow.json)：100 GB 成功发布、四机分片校验及错误交接／回收流程通过；完整 TLC 重跑 218 个案例（十五个只改环境、六个活性反例）符合预期。该轮清单有 36 条契约、31 个消费者，只增加探针与模型，生产 Rust／Python 未改；逐字节核对 Verus 输入、契约及测试源后，沿用下述 693 个 Python、35 个原生测试和 111 项 Verus／85 个语义拒绝的既有证据，没有将其冒充新测试运行。旧 Verus 报告保留其原环境清单哈希，新清单只新增消费者与证据路径。

上一轮完整 wheel 集成验证见 [准备阶段 owner 清理记录](../verification/measurements/20261023-provisional-holds.json)：693 个 Python、35 个原生测试通过；111 项 Verus 验证、85 个预期语义拒绝（四个只改环境），207 个 TLC 案例（十四个只改环境、五个活性反例）符合预期。清单有 36 条契约、30 个消费者。四机旧／新各 480 例，旧版留下 480 个内部 owner，新版为零；独立 reader 始终保留。该轮修复验证时未重跑 100 GB；本轮同一生产源码的原生大文件测量见上一段，此前其他二进制的证据也单独保留。完整适配、模型组合及 writer 重启仍未证明。此前 [staging 消费记录](../verification/measurements/20261023-staging-consumption.json) 保留上一版的验证与回收后重试证据。

此前包含 Python／原生套件的完整集成验证见 [`20261023-gc-parent-repair.json`](../verification/measurements/20261023-gc-parent-repair.json)：101 项 Verus 验证、73 个按预期拒绝的语义变异（四个只改环境）、167 个 TLC 案例（十一个只改环境、两个真正的活性反例），474 个 Python、34 个原生测试通过。新增父目录可执行性／修复模型、实际响应分类函数、共 32 条环境契约、四机 syscall 测试及当前 Rust 核心的真实 100 GB 发布／认证／GC。完整 I/O／锁细化、writer 进程重启和全系统进展仍未证明。此前[未完成写入清理](../verification/measurements/20261023-incomplete-publication.json)、[catalog 编码](../verification/measurements/20261023-catalog-encoding.json)、[路径身份](../verification/measurements/20261023-path-identity.json)和[pending-publication](../verification/measurements/20261023-pending-publication.json)保留各自精确二进制测量及证明范围。

此前[独立文件完成／100 GB 实测](../verification/measurements/20261023-publication-finalization.json)、[publication 冲突重试修复](../verification/measurements/20261023-publication-retry.json)、[目录持久性修复](../verification/measurements/20261023-directory-durability.json)、[GC 自动发现修复](../verification/measurements/20261023-catalog-discovery.json)、[catalog 有界预读](../verification/measurements/20261023-catalog-buffer.json)、[GC 陈旧 stat 修复](../verification/measurements/20261023-gc-stat.json)、[catalog 空刷新及长历史优化](../verification/measurements/20261023-catalog-history.json)、[catalog 尾部丢失修复](../verification/measurements/20261023-catalog-replay.json)、[锁外校验与 100 GB 扫描](../verification/measurements/20261023-unlocked-reactivation.json)、[阶段计时对照](../verification/measurements/20261023-reactivation-timing.json)和[内存修复](../verification/measurements/20261023-reactivation.json)保留各自二进制与验证范围。先前的 8.68 秒异常仍未归因；这些修复同样不能解释那个样本。

新增的 [writer 生命周期记录](../verification/measurements/20261023-writer-lifetime.json)把进程环境独立出来，使 open reservation 的进展缺口成为可执行反例；当时清单共有 33 条契约、26 个消费者。它区分“超时 seal 候选算法”的安全反例和当前策略在 writer 死亡后的活性反例，没有实现崩溃恢复，也没有新增实际 Rust 证明。见[环境层审查](ENVIRONMENT_MODELS_zh.md#writer-存活性与回收资格)。四机执行了 16 个真实暂停／恢复／终止场景；完整形式化重跑包含 171 个 TLC 案例（十一个只改环境、三个活性反例），以及不变的 101 项 Verus 验证和 73 个按预期拒绝的语义变异。本次生产 Rust／wheel 与上述完整集成验证相同，未为模型／文档增补重跑其中的 474 个 Python、34 个原生测试。

## 运行有限状态模型

straw 使用 TLA+ 和 TLC 检查已发布 extent 的读取、所有权转移与 GC 的有限状态模型。运行仓库中的规范及负向对照：

```sh
# 需要 Java 17+、Python 3.10+；不需要安装 Python straw 包。
python verification/runners/check_tla.py
# 离线运行：提供相同的固定版本 jar，仍然校验 SHA-256。
python verification/runners/check_tla.py --jar /path/to/tla2tools.jar --output /tmp/straw-tla
```

运行器从固定 release URL 下载 TLA+ 工具，校验 SHA-256 后才执行。`target/tla/report.json` 记录规范/配置哈希、状态数、退出码和结果。每个案例目录保留实际执行的 `.tla`、`.cfg`、`tlc.log`，包括反例轨迹。`--case NAME` 选择单个案例；完整检查不传此参数。每个案例默认限时 300 秒。超时、解析错误、非预期不变量失败、缺失反例都会使检查失败。安全负向对照必须以 TLC 不变量违反退出（退出码 12），且包含指定不变量和状态轨迹。CI 执行完整检查并上传证据。

TLC 提供有限状态模型检查，不是 TLAPS 归纳证明，也不证明 Rust 对 TLA+ 的细化。下面的 Verus 层单独覆盖部分 Rust 函数和无界模型性质。任何一层都不能据此宣称所有 API 没有 bug；部署假设仍需单独验证。

## 已发布数据读取：`ExtentVisibility.tla`

发布引用前数据必须持久化。作为保守的故障假设（并非 JuiceFS 正常行为声明），模型允许发布后每个 reader 的一次**新打开**观察到长度、header、footer、索引、envelope、payload 和 tensor chunk 中任意一些区域尚不可见。header 分为全零和部分为零两类。区域子集有意保守地覆盖字节/缓存页的组合；达到配置的可见性界限之前，不同次打开的可见性也可以倒退。成功读到 header 不保证其他区域可见。各 reader 有独立状态和重试预算。

模型将哈希抽象为无碰撞的认证。`Corrupt=TRUE` 表示被校验区域持续损坏，不表示任意引用语义错误或非零坏 header（这些情况仍立即失败）。检查的性质包括：

- `NoUnverifiedReturn`：成功返回必须发生在持久化发布之后，并且请求的数据通过认证。
- `NoPrematureFailure`：可重试的校验失败必须耗尽共同预算后才能返回错误。
- `EventuallyTerminates`：在发布、打开、校验动作满足弱公平性的条件下，每次读取最终成功或失败，持续损坏也不会无限重试。
- `VisibleInBudgetSucceeds`：如果最迟在最后一次允许的尝试中，重新打开能读到完整正确数据，那么每次读取最终成功。

`documented_*` 案例设置 `VisibleBy=0`，表示发布后每次新打开都获得完整正确视图，对应较强的文档契约。修复前后的策略在这一假设下都通过。因此，历史策略在延迟视图模型中失败，本身不说明它违反了 JuiceFS 文档契约。

容错检查集包含一个 reader、实际五次重试的配置；两个独立 reader、各一次重试的配置；可见性超过预算的配置；以及持续损坏配置。这不构成任意 reader 数量或任意重试次数的定理。仅有“最终可见”而**没有时间界限**，无法保证有限重试必定成功；耗尽预算可以报错，但不能返回未校验字节。

历史模式分别描述 v0.1.2 之前、v0.1.2 和 v0.1.3 的策略，对应配置分别限制为全零 header、部分 header 和 footer 延迟，均必须违反 `NoPrematureFailure`。它们是版本行为的抽象，不是运行历史二进制。`skip_validation` 对照必须违反 `NoUnverifiedReturn`，防止用无条件成功来满足可用性性质。

### 反例与修复

v0.1.3 的反例为：数据同步、引用发布 → reader 打开文件，header 正确但后续区域尚不可见 → 校验失败且没有重试。只重试 header 无法处理独立延迟的 footer、索引、envelope、payload 或 chunk。新增公共 API 测试在未修改的 v0.1.3 wheel 上复现了 footer 和索引失败。

`rust/store.rs` 现在针对可见性相关失败重试完整的请求读取。header、索引、envelope 和 payload/chunk 共用原来的五次等待（100、250、500、1000、2000 ms），不会按阶段嵌套预算。失败尝试的文件和临时 buffer 在等待前释放；重试重新打开并校验。session 可以复用同一完整描述符的已认证索引，但每次仍校验 envelope、record 和触及的 chunk。checksum 失败无法区分陈旧字节和持续损坏，因此两者都会重试；持续失败最终仍返回 `CorruptData`。身份/schema 错误、非零坏 header 和其他 I/O 错误不会因此变成成功读取。3.85 秒是重试等待总和，不是文件系统 I/O 截止时间。manifest/依赖遍历可能包含多次分别受限的读取。

| 模型动作 | 实现 | 回归证据 |
|---|---|---|
| Sync / Publish | `Store::write_prepared`、`finish`、staging | 已有发布与崩溃测试 |
| Open / Check | `Config::inspect_once`、`frame_with_index`、`read_frame`、`read_frame_range` | `tests/test_extent_visibility.py`：延迟的 header/footer/索引/frame/envelope/payload/manifest，以及 tensor chunk |
| 共用预算的 Retry | `retry_extent_with_wait`、`Reader::index_once` 与读取入口 | 原生测试 `one_retry_budget_covers_header_index_envelope_and_payload` 不依赖定时器，逐次改变不可见区域 |
| 拒绝损坏数据 | 每次对请求的 record/chunk 认证 | 持续损坏、非零坏 header、缓存索引测试 |

延迟区域测试通过修改本地测试文件模拟读者观察，并不证明某种文件系统实际会产生所有这些观察。官方 JuiceFS 文档描述默认 close-to-open 一致性和 fsync 刷新；关闭客户端 writeback 时，数据先上传、再提交元数据。文档没有把任意的 post-fsync 零填空洞规定为正常行为。open cache/writeback 会改变保证。这里的 footer/索引反例证明的是该故障模型下重试策略的局限，不是新复现的 JuiceFS 缺陷。参见[部署契约与测量方法](FILESYSTEM_zh.md#可见性契约与测量)。[FILESYSTEM_zh.md](FILESYSTEM_zh.md) 说明部署假设。

## 所有权与 GC：`Ownership.tla`

模型包含两个队列、两个 pack（一个 publication 的 manifest/依赖闭包）、一个独立 reader、一个 checkpoint、一次崩溃，每个队列至多一次 checkpoint 回退。pack 可分别 seal。持久化 catalog 更新在此抽象中是原子动作；队列 WAL 写入/同步/回复与 GC 扫描/标记/删除是分开的动作。catalog 锁阻止 retain/release 与候选选择、tombstone 持久化交错。恢复时清理未提交队列根，保留已提交根和独立 reader。

检查类型、持久化队列所有权、活跃 reader/checkpoint 保护、传递可达性、已确认 WAL 的持久性、先 seal 后回收，以及先持久化 tombstone 后删除。六种负向对照分别去掉先 retain 后 WAL、依赖保护、GC 锁、tombstone 顺序、reader 保护、回退时重新 retain；每种必须违反指定的不变量。这些人为变异用于检查规范的检错能力，不是新发现的六个代码 bug。

| 模型动作 | 代码对应 |
|---|---|
| Write / Sync / Stage / Seal | `Store::write_prepared`、`Config::stage`、`Catalog::pack` |
| Adopt / QueueWrite / QueueSync / Reply | Coordinator 的 retain 与 `Journal::append` |
| Pin / Unpin / SaveCheckpoint | Catalog 持久化 owner 集；调用方确认读取确实结束 |
| Rewind | 消费者状态 WAL 写入前的 `Catalog::retain_reactivated` |
| Scan / Mark / Unlink | `Catalog::transaction`、`collect` |
| Crash | 队列 WAL 重放与所有权对账 |

对建模的正常所有权协议没有发现新违反。catalog 的字节级崩溃行为继续由 `verification/wal/checks/crash_campaign.py` 检查，不能把模型中的原子动作当成实现证明。已有独立 Python 所有权模型也继续保留。

尚未覆盖的边界包括任意依赖图及环、task lease 调度/epoch、任意请求历史、checkpoint 替换、多次并发崩溃、内存安全、哈希碰撞、文件系统和 OS 内部。模型不承诺 GC 活性：没有明确结束信号的 reader 或崩溃 writer 可以永久保留数据。修改协议时，需要一起维护这些假设、代码对应关系和反例回归测试。

## GC 删除：unlink 之后的失败也要覆盖

`GCDeletion.tla` 分开表示持久化 tombstone、unlink 和父目录同步。失败可能已让文件名消失，但未持久化这一变化；崩溃可能恢复未同步的目录项。模型允许稳定之前任意多次重试。在故障最终停止、GC 动作满足弱公平性的条件下，永久失去所有 owner 的 sealed pack 最终持久回收。`DurableCompletion` 要求同步目录后才能完成；`DeleteAfterMark` 要求先 tombstone 后 unlink。跳过缺失文件的目录同步、先 unlink 后标记的变体必须违反对应不变量。这是单 pack 的活性检查；`Ownership.tla` 另行覆盖所有权交错，不宣称一般 GC 活性。

此前发现了真实实现缺口：unlink 成功但目录同步失败后，下一次 `collect` 因为 `stat` 返回 `ENOENT` 而跳过同步。现在 `Catalog::collect` 使用经过验证的删除控制器，缺失的 pack 也必须经过 SyncParent。回归测试注入连续两次同步失败，再检查成功重试且不重复计数。持久 tombstone 仍会拒绝 late retain，其他错误继续上报。父目录存在、存储路径由库管理仍属于部署契约。当前先完成全部待处理 tombstone 的 unlink，再对每个不同父目录同步一次。已不存在的名称仍贡献其父目录。`delete_batch_finished` 验证每个成员都处在等待 SyncParent 的阶段，且适配层报告所有屏障成功，才能完成整批；仅整次调用成功时返回回收文件数／字节数。错误会终止调用并保留 tombstone 供重试，重试不会重复计数已删除名称。

`GCBatchDeletion.tla` 用三个永久无 owner 的 pack、两个目录（其中两个 pack 共享目录）探索稳定前任意次失败／重试。独立持久化环境的 `EnvSyncedDirectory` 表示一次屏障只持久化该目录内、此前发生的名称变化。正例有 144 个状态，检查标记顺序、完成持久性及条件回收；三个算法负例分别过早同步、重试时忽略缺失名称的目录、漏掉一个目录，均必须违反 `DurableCompletion`；它们是错误的批量方案，不表示旧逐文件同步实现不安全。运行时回归覆盖第一个／第二个目录屏障失败、部分 unlink 后失败，以及名称全不存在时的重复调用。

Rust 批量完成检查循环对任意 slice 长度证明。路径分组、目录覆盖完整性和真实 syscall 成功分发仍属于适配层义务，尚未用 Verus 证明；此前单 pack 无界活性证明也尚未与这个多 pack 适配层机械组合。仅批量同步仍保留历史 stat／扫描，见[对照测量](FILESYSTEM_zh.md#批量目录同步对照2026-10-23)。

### 持久回收凭据

全部相关父目录同步之后，GC 追加并同步可选 `reclaimed` catalog 事件。Replay 将路径从待删除索引和 pack 候选中移除，但永久保留 tombstone，继续拒绝 late retain 和路径复用。后续调用跳过对应 payload 的 stat／unlink／目录屏障。仅名称缺失还不够：缺失或撕裂 receipt 会保留安全重试；幸存的完整 receipt 在 catalog 恢复时同步后才能复用。空调用不追加 receipt，仍然加锁、刷新并同步 catalog。

`GCReceipt.tla` 检查这一顺序、跳过前删除已持久化、永久 fencing，以及稳定／公平前提下最终完成。新增独立 `EnvStatPresence` 环境（`ENV-GC-STAT`），允许 stat 的存在／缺失观察持续陈旧；stat 不需要最终变新才能完成删除，但调用本身仍需返回，unlink 的成功／NotFound 仍需保证实际不存在。单 pack 正例有 64 个状态；四个负例分别在目录同步前写 receipt、把缺失当 receipt、删除 tombstone、因 stat 缺失而跳过 unlink，必须违反指定不变量。最后一个对照复现旧适配分支，产生 `ReceiptIsSafe` 反例。这是有限模型证据，没有把单 pack 无界删除定理扩展到 receipt 恢复。

旧实现把 stat NotFound 直接解释为 Unlink 成功，然后同步目录并写 receipt；文件实际存在时，屏障只会持久化仍存在的名称。修复后所有 pending 路径都执行 unlink，stat 只用于诊断计数。没有取得大小但 unlink 成功时，文件数加一、字节数加零；`reclaimed_bytes` 不是存储容量的精确审计值。原生回归覆盖实际存在／缺失、unlink 失败、屏障失败及重开恢复；四机注入一次错误 stat 的旧版 8 例均留下孤儿，新版 100 例均真正删除。注入不证明部署曾自然出现该故障；已存在的错误历史 receipt 不会被这项修复自动修复。

实际 `reclaim_*` 控制器证明 SyncParents 先于 RecordReceipt；`reclamation.rs::mark/retire` 证明实际 HashSet 更新：pending 始终是 tombstone 的子集，重复 tombstone 不重新安排已完成工作，receipt 不接受未标记路径，也不删除永久 fence。契约显式要求 `ENV-STRING-KEY`，即固定版本 vstd 尚未证明的标准 String 哈希／相等定律。JSON 解码、syscall 分发、frame 持久性与 replay 到索引的对应关系仍未证明。

`tests/test_gc_receipts.py` 覆盖 receipt append 切点、完整 receipt frame 的每个字节前缀与逐字节损坏、重复 tombstone、未标记 receipt、重启和 late-retain 拒绝。四机交错检查上一版批量二进制忽略 receipt、新版继续完成旧版删除；不代表任意版本互通。首次非空 GC 新增一次 catalog append／sync；永久 tombstone 和日志仍增长，冷 replay 仍随历史增长。[测量](FILESYSTEM_zh.md#持久回收凭据对照2026-10-23)覆盖热重复调用，不提供延迟保证。

## Lease 筛选与 completion／恢复

`LeaseSelection.tla` 穷举三个任务、长度至多四的过滤列表、1..4 的批量上限，要求分配不重复、不浪费分配名额、遵守批量上限，并在弱公平性下终止。保留重复过滤项的 legacy 变体必须违反 `UniqueAssignments`。

这复现了一个真实 Rust bug：`acquire(max_tasks=2, task_ids=["a", "a", "b"])` 可能返回两个 `a` 的 lease，后一个立即覆盖前一个。现在 `rust/selection.rs::first_occurrences` 在发出 lease 前去除重复调度键，保留排序顺序，按不同任务计算批量名额。Verus 证明了整个实际循环：输出与输入等长，每个布尔值恰好对应完整调度键的首次出现，证明没有过滤列表长度上限。`tests/test_lease_selection.py` 检查重复输入、批量大小、heartbeat 授权、priority/control/prefix 过滤和恢复。

`Completion.tla` 分开表示接收判定、完整 WAL 写入、同步、内存 apply、回复和恢复，覆盖一个任务、两个 attempt、两个 epoch、两个 submission ID、两个逻辑 digest 和一次崩溃。该有限空间允许任意次重复请求。恢复可以保留失败 append 后幸存的完整 frame，并在使用前同步，也可以只保留此前持久前缀。检查已确认记录的持久性、每任务至多一个已确认 completion、当前 lease 才能接收新结果、跨 epoch 重放匹配 receipt、冲突拒绝，以及弱公平性下的返回／恢复进展。物理重排被抽象掉，重试由逻辑 digest 标识。

五种变体分别把 lease 检查移到 receipt 查询之前、跳过授权、同步前回复、丢失持久恢复、忽略内容冲突，每种都必须违反指定不变量。这是检错能力对照，不是五个新 Rust bug。对应代码是 `Coordinator::valid_lease`、`complete_task`、提交控制器和 `Journal::recover`；已有过期、幂等、丢回复、torn WAL 测试及提交切点回归覆盖这些路径。**完整 completion 判定和 replay 仍是有限模型／测试证据**，没有宣称已完成整个 API 的无界 Verus 证明。去重循环已有实际代码证明，JSON 到调度键的转换和调用方仍在边界之外。

## Catalog 重放失败与未确认缓存

当另一个客户端写完合法 catalog frame、但在 sync 前报错时，当前客户端可能先重放它并推进内存游标，然后自己的恢复 sync 也失败。如果存储随后丢掉这个未确认尾部，而客户端进程仍在，旧实现会把游标前方的变化误判为已确认日志被截断；同一对象不断重试仍返回 `UnsafeRecovery`，新建对象却能恢复。定点 syscall 故障在旧 wheel 上复现了这个问题；它不是观测到 JuiceFS 服务真实断电。

现在 catalog 独立记录本对象已确认持久的字节下限。存在未读尾部时，在解析／apply **之前**标记缓存可能包含未确认修改；即使 apply 只改了一部分索引就失败，下一次加锁、fresh open 后也会从零重建 owners、packs、永久 tombstone、pending deletion、closure cache 和 sequence。文件长度或最终完整前缀低于已确认下限时仍拒绝，且不截断文件。只有恢复 sync 成功之后才确认新位置，随后才允许调用方修改 catalog。

空刷新没有未读尾部，已确认的缓存未被修改，sync 失败后可复用它；下一次操作仍加锁、fresh open、检查长度／新 frame 并同步，不能忽略其他客户端随后追加的状态。此前无条件失效会在约 46.63 MiB 历史上引入一次完整重放。本次只避免这种冗余工作；首次打开、未确认修改后的恢复仍完整重放，成本随历史增长，没有新增时间／内存上界或线格式变化。

`CatalogReplay.tla` 检查一个已确认前缀，以及可选的未确认 owner frame。带尾部／无尾部正例分别有 28／8 个状态；检查确认不领先于持久状态、返回缓存与完整前缀一致、不误报损坏、无尾部时不安排重建，以及稳定／公平前提下完成。五个算法对照覆盖保留旧游标、重建时残留 owner、提前确认、部分 apply 后才标记，以及空刷新失败也无条件重建。最后一个违反 `NoUnneededRebuild`，这是避免冗余工作的策略性质，**不是延迟保证**。

模型中的 `PartialApplyAndRepair` 显式组合部分内存副作用与外部移除失败的未确认尾部；它不代表 straw 自动修复坏 frame，也不证明永久损坏会自行消失。实际损坏 frame 仍先报错且保留字节。这是单次串行 refresh 的有限模型，没有组合任意并发 append、字节 framing 或完整 GC／队列恢复。

实际 `replay.rs::start/prepare/accepts/confirm` 对全部 `u64` 位置和布尔标记由 Verus 证明：prepare 保持持久下限，重建标记等于旧标记 OR 本次可能修改；无新修改不会清除既有重建义务。只有不小于下限的位置才被接受，确认下限单调，拒绝不改状态。五个语义变异必须被拒绝。调用方何时标记、I/O、JSON apply、全部缓存清理和锁仍是未整体证明的适配层。

原生回归保留失去 retain／release／seal／tombstone／receipt 尾部、重复失败及真正丢失／不完整已确认前缀的检查；新增空刷新失败后看到 peer 的 release／seal、仍拒绝已确认前缀丢失，以及部分 apply 失败后的残留 owner 清理。四机新二进制另通过 40 次丢失／幸存未确认尾部回归。Linux fsync 故障 shim 不链接进库，也不属于 Verus 证明。

## 整数边界与统计缓存

`ExtentArithmetic.tla` 穷举小位宽无符号数，检查 frame 结束位置和 usage 总量必须精确可表示，否则拒绝。历史变体复现了 frame 长度的内层加法未检查、usage 求和回绕。`UsageCache.tla` 包含预留、完成、取消、消费确认和缓存重算：缓存必须精确或不存在；溢出应使缓存失效，不能中断已持久化事件的应用。TLC 使用小位宽抽象，Rust/Python 回归使用真实 `2**64 - 1`，以及跨 2 GiB、4 GiB、2 TiB 的合法稀疏偏移。

修复前实际复现了三类错误：合法校验和的索引能携带回绕的 frame 长度；publication 的 token 总量回绕为零；有无缓存时队列 usage 都可能回绕为零。现在无法表示的 publication/manifest 总量在写入前报错。诊断性 usage 查询显式报告溢出；调度、持久化完成和恢复仍能继续，已提交结果不会丢失。这是线格式整数的表示边界，不是恢复旧的逻辑积压配额。

范围读取原来会分配整个 checksum chunk。tensor 的 checksum 数量有上限，大 tensor 的单个 chunk 因而可能很大。现在校验只需最多 1 MiB scratch，另外还需请求结果、索引和 metadata 的内存。所有涉及的 chunk 仍需完整校验，之后才返回结果。`tests/test_extent_bounds.py` 覆盖远端字节损坏、缓冲区交界、高偏移、溢出/恢复，并在只允许额外 32 MiB 地址空间的进程里读取 64 MiB chunk 中的 16 字节。完整 record 读取仍需容纳完整返回值；大 tensor 应使用范围读取。大 chunk 下的小切片仍有**读取放大**，内存有界不会消除这项 I/O 成本。

## Checkpoint 回退的完整 payload 校验

`Catalog::retain_reactivated` 之前通过 `Reader::read` 校验每条可达记录，随后丢弃返回值。64 MiB 的直接记录和传递依赖在额外地址空间限制为 32 MiB 的新进程里均触发分配失败、SIGABRT。现在该路径使用 `Reader::verify`，完整读取并计算每条 payload 的 SHA-256，但只分配最多 1 MiB scratch；成功校验全部待恢复根之后才追加 owner，随后才能推进队列 WAL。公共完整 record 读取仍需分配返回数据。

`ReactivationScan.tla` 检查单次尝试中两条记录、每条 0..3 字节、缓冲区 2 字节、所有损坏位置集合及每次读取的完整／短读／I/O 错误。2,724 个正例状态满足 scratch 有界、完整字节覆盖、认证后授权、失败不授权和弱公平条件下终止。四个变体分别分配整条记录、跳过尾部、忽略损坏、接受读取错误，必须违反对应性质。长度和缓冲区数值只用于有限搜索，不是实际配额；1 MiB 是算法的 scratch 策略，不是环境提供的数值。

`EnvironmentRead.tla`（`ENV-READ`）独立描述读响应；哈希检测损坏仍依赖 `ENV-AUTH`。模型从完整闭包和受保护的 source owner 开始，未组合真实锁／图遍历／WAL 恢复，且没有证明 syscall 一定返回。已有可见性模型负责重试预算，本模型不新增重试界限。

实际 Rust 的 `verification_capacity` 对全部 `u64` 长度证明返回 `min(length, 1 MiB)`，与 `next_buffer` 的有界、严格推进和精确累加证明共同约束扫描。外围 `read_exact`／SHA-256 循环及 catalog 适配层仍未整体用 Verus 证明。原生回归覆盖空记录、缓冲区边界、尾部、短读和重试；Python 回归覆盖受限内存、直接／传递依赖、后续记录损坏时 owner 与 WAL 均不修改，以及修复后重试与 GC 保护。

这一上限只约束 payload scratch；manifest、index、依赖图和 catalog 仍需要内存。完整校验需要读取全部可达 payload。内存修复时扫描仍持有 catalog 锁；后续锁外校验见下一节，两轮测量保留各自二进制和适用范围。

## 锁外校验与恢复授权的重新检查

Coordinator 的 Retain 阶段现在分三步：锁内筛选缺失 owner 并检查保护来源；释放文件锁和进程内 catalog mutex 后完整校验；重新获取锁、刷新 catalog，再检查每个待恢复闭包和永久 tombstone，全部通过后才追加 owner。普通根的 retain 仍在最后一次事务中、队列 WAL 之前完成。`Config::verify_reactivation` 返回不可由调用者修改的 `VerifiedReactivation`，绑定绝对池路径、run ID、原始 owner／完整引用以及实际校验过的引用 digest。它不是持久 owner 或 reader pin，不授权读取，也不允许跨池复用。

GC 可以与扫描交错：打开失败可以结束本次尝试，已打开描述符也可能在 unlink 后读完；后者不能凭旧快照恢复已回收数据。提交前必须重新检查。初次快照中已有 owner 的根不会重复扫描；若它在两阶段之间消失，且本次没有针对该引用的校验结果，就拒绝并要求重新尝试。替换成另一个合法的保护来源可以成功。发生在提交之前的这些失败不写 owner 或队列 WAL，没有临时持久 pin 需要恢复／回收。实际 append 失败的部分副作用仍由原有恢复协议处理。

`ReactivationRace.tla` 的两个 pack、165 个正例状态覆盖 source owner 的释放／重新保留、GC mark／unlink、校验失败及 unlink 后校验完成；检查无悬空恢复、授权时存在保护来源、校验身份匹配、无部分恢复，以及弱公平前提下结束。四个算法对照复用旧快照、在检查与 append 之间解锁、省略保护来源、接受不匹配的校验身份；另一个环境对照只削弱 catalog 串行化。针对旧快照、提前解锁和环境失锁的三个对照只检查 `TypeOK` 与 `NoDanglingRestore`，以观察真正的悬空引用，而不被更早的 source-owner 前提违反截断；过滤范围记录在报告和实际 cfg 中，正例检查全部性质。

实际 Rust 的 `protocol::reactivation_permitted` 完整循环由 Verus 证明：已验证、闭包非空、每个成员都有 owner 且没有 tombstone，恰好构成允许条件。证明覆盖任意长度状态数组，但真实 catalog 到快照的转换、私有校验凭据的构造／绑定、I/O、锁及与 WAL 的组合仍未做整体 Rust 证明。模型中的原子持久 catalog、不可变 extent 身份和完整闭包是明确前提；本模型未机械组合前一节的逐字节扫描模型或完整崩溃恢复。

Python 回归在扫描前／后暂停，让独立进程完成 release、GC、保护来源替换或无关写入；检查失败时 WAL／owner 不增加，并通过重启确认 cursor。原生回归检查多根全部重新检查后才 append、跨池／run 凭据拒绝，以及已有 owner 消失后的未验证根拒绝。新增两个扫描切点也加入恢复回归。

`Config::retain_reactivated` 是释放锁进行校验的便捷入口；保留的低层 `Catalog::retain_reactivated` 必须在调用者事务中使用，因此仍会持锁扫描，作为兼容路径和对照。Coordinator 使用新路径。闭包 metadata 遍历、catalog replay／sync、最终检查与 append 仍在锁内，锁等待没有时间保证；持续竞争或保护来源丢失仍可拖慢或使恢复失败。测试与测量不等于文件锁公平性或完整 API 的时延证明。

## 依赖图遍历、闭包缓存与 GC

先前 `Reader::validate_inner` 和 catalog 闭包计算递归遍历依赖。在独立测试进程中，合法的 2,048 层 inline manifest 链配合 512 KiB 栈、以及 16,384 层链配合默认 8 MiB 栈，两个操作都以 SIGSEGV 退出。替换为堆上的工作队列后，16,384 层链在 512 KiB 栈下通过校验、retain、释放 staging、保护可达 pack、最后 GC 的全流程；没有新增深度准入上限。

`rust/traversal.rs` 的实际泛型 `start/schedule/next` 已用 Verus 证明：首次遇到身份才入队、同时保存对应数据、pending 中身份唯一且都在 seen 内、出队保留 seen 并移除正确元素。它适用于任意数据类型和队列长度，依赖显式 `ENV-STRING-KEY` 库契约。生产路径直接使用这些函数；JSON 到身份／边的解码、外围遍历循环的全局闭包正确性、文件读取和内存分配仍未做完整 Rust 证明。

`DependencyTraversal.tla` 穷举三个节点的全部有向图及缓存子集，包含循环、共享依赖和不可达节点。15,104 个正例状态检查结束时结果恰为传递闭包、节点不重复展开，以及有限图／弱公平前提下终止。遗漏后继、重复入队、把缓存闭包缩为当前节点三个变体必须分别违反指定性质。模型验证引用节点闭包，真实节点到物理 pack 的映射和所有权协议组合仍是适配层义务；它没有把 `Ownership.tla` 扩展为任意图的端到端证明。实际有效的本地 ordinal 仍必须先于父 manifest，模型中的循环是额外容错探索。

Catalog 复用完整缓存闭包，最终仍检查所有路径的永久 tombstone。冷遍历只缓存所请求根的完整结果，避免为一条链的每个中间后缀另存闭包；显式保留很多不同根时，缓存集合总大小仍可能增长。校验只对根的直接 records 检查 task／attempt，依赖沿用独立身份；返回值仍为根的直接 records。测试覆盖冷／热缓存、只收集不可达 pack、另一个 owner 保护依赖、已回收路径的 late-retain 拒绝，以及深处畸形逻辑引用。

环境 `ENV-GRAPH` 要求有限、不可变的引用图、完整的后继枚举和完整缓存闭包；通过有限模型不证明这些条件由真实 JSON／I/O 实现满足。堆内存随图、索引和缓存增长；这次修复消除了按依赖深度增长的 Rust 调用栈，不承诺常量内存、任意 JSON 嵌套深度或延迟上界。

## 原生流式 payload：两次遍历与长度快照

`PayloadStream.tla` 分开建模 envelope 哈希与实际写入。输入环境允许空 chunk、短／长遍历、变化的长度声明、忽略 callback 错误；两个阶段各至多两次 callback，长度域为 0..2，都是 TLC 搜索边界，不是 API 上限。正例 127 个状态检查：成功时实际字节数与 envelope／frame 长度一致、失败不能变成成功，以及有限 provider／弱公平条件下返回。哈希相等抽象为长度相等这个必要条件，没有证明真实字节认证。

回归在修复前原生代码上复现了：声明 5 字节却两次输出 3 字节仍返回引用；provider 忽略 callback 错误后发布仍成功；再次调用变化的 `len()` 使 frame 与 envelope 不同。现在只捕获一次长度，分别检查哈希与写入遍历，超长 chunk 在写入前拒绝，短遍历在写 footer 前拒绝，保留首次 callback 错误。第二遍失败仍可能留下未发布尾部，不保证回滚；先前已发布 extent 不受影响。三个有限模型负例分别取消长度检查、丢失 callback 错误、重新读取长度，均产生指定反例。

`rust/stream.rs` 的四个实际函数 `start/accept/fail/complete` 已用 Verus 对所有 `u64` 值证明：精确累加且不越过声明长度、无整数溢出、失败状态不可恢复、只有精确长度且未失败才能完成。这是状态转移证明；`payload.rs::visit_exact` 的 trait／closure 分发、首次 Error 保存、JSON 字段读取及真实 I/O 尚未进行整体 Rust 一致性证明，靠原生集成回归连接。Provider 必须终止；guard 不能抢占任意自定义 Rust，也没有为它建立超时。环境契约见 `ENV-STREAM`。

## Verus：实际 Rust 与模型的证明

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustc-dev,llvm-tools
python verification/runners/check_verus.py --download
# 或提供已经安装的固定版本：
python verification/runners/check_verus.py --verus /path/to/verus
```

运行器固定 Verus `0.2026.10.18.7d82994` 并校验 release 压缩包 SHA-256。`rust/bounds.rs`、`rust/protocol.rs`、`rust/selection.rs`、`rust/reclamation.rs`、`rust/stream.rs`、`rust/replay.rs` 和 `rust/traversal.rs` 使用官方 [Verus 注解语法](https://verus-lang.github.io/verus/guide/exec_attr.html)，由 `cfg(straw_verus)` 启用。验证入口直接导入生产文件；普通 Cargo 编译相同可执行函数体，去掉证明注解，不增加运行时依赖。输入副本、源文件哈希、工具版本和日志保存在 `target/verus`。

六个算术／容量函数覆盖全部 `u64` 输入：

| 函数 | 证明性质 | 实际用途 |
|---|---|---|
| `add` | 可表示则返回精确和，否则返回 `None` | result/usage 汇总和完成缓存 |
| `frame_end` | base + frame header + envelope + payload 精确或拒绝 | 已认证索引的边界验证 |
| `adjust` | 精确加减或溢出／下溢拒绝 | active reservation 缓存 |
| `clip` | 读取缓冲区内的精确钳制位置 | 请求输出与缓冲区的交集 |
| `verification_capacity` | 精确为 `min(length, 1 MiB)`，非空记录容量为正 | 完整 payload／chunk 校验 scratch |
| `next_buffer` | 正数、有界、不越界的推进 | 流式 checksum 校验 |

新增 `protocol.rs` 的实际代码证明：

| 函数组 | 证明性质 | 实际调用方 |
|---|---|---|
| `execute_commit` | 在 `ENV-COMMIT-EFFECTS` 下证明实际调用、错误传播、apply 隔离及成功条件；原生 effect 对应与 unwind 单列 | `Coordinator::commit` |
| `commit_start/next/advance/finished` | `View` 与规范一致；Retain → AppendWal → Apply → ConsumeStaging；错误停止本次尝试；四步成功才完成 | `Coordinator::commit` |
| `reactivation_permitted` | 已验证且闭包非空，每个成员均有 owner 且无 tombstone；完整实际循环 | 持锁并刷新后的 `Catalog::retain_verified` |
| `candidate`、`Indexes::{roots,candidates}` | 枚举所有实际 owner 集合；候选恰为实际索引中 sealed、无 owner、无 tombstone 的对象；循环终止 | 持有 catalog 锁的 `Catalog::collect`；replay 到索引以及完整逻辑根的建立另行细化 |
| `delete_start/next/advance/finished`、`delete_batch_finished` | 只有 tombstoned pack 才能发出操作；Unlink 在 SyncParent 之前；失败步骤不能报告完成；批量成功要求每个成员等待同步，且所有屏障成功 | `Catalog::collect`，包括文件已不存在的情况 |
| `reclaim_start/next/advance/finished` | SyncParents 先于 RecordReceipt；错误停止本次尝试；两个 effect 都成功才完成 | `Catalog::collect` 完成阶段 |

控制器的成功表示适配层报告每个指定操作成功。Verus **没有**证明适配层成功就等于 I/O 持久，也没有证明未验证的分发代码调用了正确操作。实际调用方通过共享 effect enum 分发，集成测试检查这一边界。错误可能留下部分持久效果；控制器不会回滚，也不会盲目重新执行结果不明的提交。

`control_plane.rs` 证明 retain、release、mark、unlink 保持 `live ⇒ present 且无 tombstone`，并对任意长度历史归纳。`candidate_correspondence` 在快照准确的前提下，把实际 eligibility 规范接到模型候选集合。retain 的集合必须已经包含完整依赖闭包；这没有证明 JSON 遍历、catalog 编码或锁的实现。

[`retry_liveness.rs`](../verification/protocol/verus/retry_liveness.rs) 替换了旧的直接 `progresses` 前提及其辅助定理。新定理对任意自然数阶段数使用生产控制器的 `initial`／`advance` View，区分 Idle、成功／失败效果、Crash 和调用方 Retry。`ENV-VERUS-PROGRESS` 明确要求效果响应最终稳定、效果调度与调用方重试分别满足弱公平；证明先导出稳定期 rank 非增及最终下降，再递归得到完成。失败 rank 为 `limit + 1`，重试回到 `limit`，成功效果逐步下降到零；不限制故障期长度或失败次数，也没有毫秒上界。

两个数学实例分别证明：经历失败／重试后四阶段完成的历史确实满足前提，以及环境恢复但调用方永不重试时完成仍不会发生。六个语义负例破坏重启、效果推进、I/O 公平、重试公平、错误终止及崩溃终止；每个检查报告记录具体定理过滤范围。真实 I/O、锁等待、有效输入、缓存响应及恢复到这些事件的映射未证明；“文件系统恢复”本身仍不能自动推出所有应用阶段成功。

单独的 [`gc_liveness.rs`](../verification/gc/verus/gc_liveness.rs) 已证明跨尝试的删除重试活性，**不依赖 `progresses`**。起点是假定目标 pack 已有持久 tombstone、永久无 owner。模型使用实际删除控制器的 View 转移，显式包括部分副作用后的 I/O 失败、崩溃、停顿和调用者重试；I/O／崩溃响应、最终无故障和 I/O／重试弱公平由 `environment.rs` 提供。稳定后 rank 依次为失败 3、重启 2、unlink 后 1、同步后 0；不限制稳定前失败次数，也不提供耗时上界。另证明了一个满足前提且完成的实例，以及“存储恢复、调用者永不重试，删除仍未持久化”的无限反例。没有自动 GC 重试后台服务。真实 stat/unlink/fsync、锁、replay 适配、可信 NotFound 与所有权组合仍未证明，详见[环境模型](ENVIRONMENT_MODELS_zh.md)。

较早一轮正例报告 **101 verified, 0 errors**，计数包含派生方法，不是 101 个公共 API。七十三个负向对照破坏父目录响应分类、未完成清理／重启及 PID 检查、编码判定、身份缓存查询／绑定／冲突检查、pending 同步认知、finalization 顺序／完成、publication hold 并集／排序前提、目录屏障转移／完成检查、GC 发现决策、重放预读容量／缓存／持久前缀 guard、恢复授权检查、算术／校验 scratch 上限、提交顺序、失败处理、tombstone／同步屏障、候选选择、lease 过滤去重、模型所有权保护、重试重置、批量完成检查、receipt 顺序／索引更新、流式长度／失败状态、遍历队列重复／丢项，或 GC 同步／稳定／公平环境，每个都必须产生语义验证失败。四个 GC 模型／环境变异针对指定的直接推论定理，报告记录具体过滤范围；正例始终验证完整 crate。启动、解析、超时、SMT 资源耗尽、工具链错误不算负例通过。运行器复制并哈希全部输入，禁止 straw 证明文件中的 `assume`、`assume_specification`、`admit` 和省略函数体。独立审计的 serde_json 库模块声明四个不透明外部类型、透明的实际 Value 枚举和十五个基础接口规范及两个仅用于 Value 的编码／观察定律；collections 库模块另列四个 HashMap/HashSet 消费迭代原语、mem::replace 及两个不透明类型，均不省略 straw 可执行函数。Verus/vstd 对标准库 `Vec`、`BTreeSet`、`HashMap`、`HashSet` 的契约、显式 String key／排序前提、SMT 求解器与编译仍属于可信边界。CI 执行 Verus、TLC 并保留证据。

新增集成回归覆盖 WAL 已持久化后，在 Apply 前后以及 ConsumeStaging 前失败：恢复后保留唯一 completion 及其存储 owner。可能部分完成的 Apply 会将旧 owner 标记为不可继续，必须恢复。Journal append 现在在写入之前拒绝空／畸形事件列表；此前可能确认一个 replay 会拒绝的 frame。共用结构检查已有测试，但 JSON framing/replay 尚未纳入 Verus 证明。

FFI、分配成功、解析、密码学、完整读取循环、OS／文件系统内部均不在实际 Rust 证明范围内。两个工具都不能证明物理带宽或延迟 SLA。

## Anvil 方法：已经落地的部分与剩余目标

[Anvil（OSDI 2024）](https://www.usenix.org/system/files/osdi24-sun-xudong.pdf)用一致性证明连接实际执行步骤和状态机，再在模型与环境上证明时序性质。[论文 artifact](https://github.com/anvil-verifier/anvil/tree/osdi24)区分可执行与规范接口；[verus-tla](https://github.com/anvil-verifier/verus-tla)提供时序定义和引理。

straw 已采用执行步骤／模型／性质分离的结构，首先接入提交和 GC；使用原生 Verus 规范和小型 rank 证明，**没有**移植 Kubernetes 环境，也没有引入 `verus-tla` 依赖。持续变化的队列需要逐请求进展、持久接收和全过程所有权安全。

尚未证明的端到端目标：

- 接收安全：请求只有一个持久 receipt；恢复保留它；旧 lease 不能改变它；所有 reader／队列／checkpoint 依赖不被回收。
- 任务进展：持续有资格的任务在 worker、调度器、存储持续进展时最终进入允许的终态。优先队列需要逐任务防饥饿条件，进程弱公平不够。
- 完整回收：永久无 owner 的 sealed pack，在反复调用 GC 且存储最终正常的条件下最终删除；包含真实依赖遍历、锁、catalog 持久化和重试的完整 Rust 证明。崩溃 writer 的 open pack 不自动满足此前提。
- 恢复：故障停止后，新 fenced coordinator 完成 replay，并依据持久证据解析不确定请求；安全性还必须覆盖稳定之前的故障期。

可信边界必须包括文件／目录同步、torn journal 前缀、锁、新打开可见性、失败返回与 owner fencing。测量可以检查这些前提、评估性能，不能证明它们。有限读取重试只保证已校验结果或明确错误；无界最终可见性不能推出预算内成功，p99 也不是活性定理的时间上界。

当前已实现实际算术和提交／GC 控制器一致性、无界 catalog 模型安全、条件控制器进展、有限模型与部署检查。完整 API 一致性、replay、lease／receipt 唯一性、consumer 转移、调度公平性及端到端恢复／回收证明仍未完成。通过检查不表示全部 Rust 或全部控制面行为都已证明。

## Catalog 预读与追加游标

重放使用最多 64 KiB 的 `BufReader`，其输入通过 `Take` 限制为锁内 stat 得到的剩余长度。buffer 和共享文件游标的 clone 只在有后缀时创建，每次 refresh 结束或失败即丢弃。截断、同步仍按最后完整校验／apply 的 offset 进行，之后显式 seek 到该边界才允许追加；底层文件游标可能已经预读到未完成尾部，不能拿来代表有效前缀。

`CatalogReadBuffer.tla` 将预读位置、已消费字节、完整 frame 边界、缓冲剩余、截断／同步和追加分开。`EnvironmentRead.tla` 单独允许 refill 返回任意正长度短片段。带尾部、无尾部、单字节缓冲三个正例分别有 74／50／53 个状态；四个负例检查无界缓冲、重试保留旧 buffer、以预读游标作为有效前缀，以及省略追加前 seek。有限模型只覆盖固定大小的已认证 frame、一个不完整尾部和至多一次读取失败；其失败次数是搜索边界，不是重试配额。它没有组合任意 byte framing、校验和、catalog cache／持久下限和 peer 并发的完整模型。

Verus 证明实际 `replay::buffer_capacity` 对全部 `u64` 输入等于 `min(remaining, 65536)`，且仅在空后缀时为零。64 KiB 是算法策略，并非测出的文件系统常数，也未证明为最优。它只约束预读缓冲；单个 JSON frame、owner／闭包索引和整个进程的内存仍未有界。标准库 `BufReader`／`Take`、共享描述符游标及最终 seek 的实际分发仍在证明边界之外。

原生回归在超过 64 KiB 的历史和 frame 上测试完整尾部的每一个字节前缀：恢复后立即追加、重开、核对序号及 owner；另检查 refill 边界附近损坏不会被静默截断。已有失败 sync／部分 apply／丢失未确认尾部回归保留，四机另通过 40 次尾部丢失／幸存回归。[测量](FILESYSTEM_zh.md#catalog-有界预读对照2026-10-23)验证减少读取调用的效果，不提供任意历史长度的延迟界限。

## GC 自动发现与所有权交接

旧 `Config::gc_enabled()` 每次调用 `storage.log.exists()`：陈旧 NotFound 或被 `exists()` 吞掉的 I/O 错误都使它返回 false。于是 `catalog()` 返回 `None`，retain 的公开调用仍正常返回，但没有写 destination owner。源 owner 根据成功返回释放后，另一个客户端可以删除 payload。这一反例对新对象和已缓存 catalog 的对象都成立，不是 GC unlink 的 stat 问题。

现在先按 `discovery.rs::decide` 决定是否需要探测。显式启用或同一 Config 家族已经要求 GC 时，不再重新探测；否则用 fresh read-only open，成功后记住要求，非 NotFound 错误必须返回错误。该标记由 Config clone 共享，并在探测结束后再次读取，避免覆盖另一个 clone 刚确认的状态。自动发现后的 catalog 初始化只能打开已有文件，不能因名称消失而创建空日志。Rust `Config::gc_enabled` 改为 `Result<bool>`，调用方需处理错误，例如 `if config.gc_enabled()? { ... }`。

Verus 直接验证实际 `decide` 的完整输入表：已要求／显式启用优先；未查询要求 Probe；Present 要求 GC；Absent 才允许 legacy；Failed 必须不可用。四个语义变异分别忘记已知要求、忽略 Present、把 Failed 当 Absent、跳过初始 Probe。证明没有覆盖 atomic 的共享／内存顺序、真实 File::open、错误到 Observation 的映射和完整调用链。

`GCDiscovery.tla` 连接发现、原子持久 retain、源 owner 释放与 GC 删除。自动冷发现正例 6 个状态，已知／显式／legacy 各 5 个。三个算法负例和一个仅修改 `EnvironmentCatalog.tla` 的环境负例都必须得到 `NoLostAcknowledgedData` 反例；这些负例显式只检查 TypeOK 与最终数据丢失性质，避免在更早的所有权不变量处停止。正例检查全部性质。这里 catalog retain 是上层原子动作，未与真实 replay、锁、所有权全模型组合。

`ENV-GC-DISCOVERY` 明确前提：池模式在参与者运行前建立；首次自动发现的 fresh-open NotFound 必须真的表示没有 GC catalog。模型不支持把仍有活跃 legacy 客户端的池在线迁移到 GC。若首次 open 也能虚假报 NotFound，环境负例表明新算法仍可能漏 retain；需要 GC 的参与者应显式传 `online_gc=True`，而不是把这处缺口当作已证明。自动发现成功后，打开缺失的 catalog 会失败，不会降级为 legacy。

原生回归检查探测错误、跨 clone 记忆、探测期间其他 clone 启用，以及已发现但尚未打开／已缓存 catalog 时名称消失。两个 Python 回归在旧 wheel 上均失败，分别覆盖已知 catalog 消失与冷发现 ELOOP；新 wheel 通过。四机探针验证真实 payload 在交接后仍保留，并检查 open EIO 报错后源 owner 保留、重试后才能交接；[证据与测量](FILESYSTEM_zh.md#gc-自动发现与交接故障2026-10-23)区分注入故障和自然现象。

## 创建目录、失败屏障与发布

目录已经存在，可能只是上一次 mkdir／sync 失败后的副作用。旧 `mkdir` 在名称存在时直接成功：首次创建 `pool/raw` 后同步失败，重试可以创建后续目录、同步 payload 及叶子目录并返回成功，却没有同步 `pool`。因此 `raw` 目录项没有持久保证；崩溃后，即使 payload 字节已同步，也可能无法再访问已确认的数据。

现在 `mkdir` 调用 `create_dir_all`、解析实际路径，再从叶子目录到文件系统根逐一同步，包括已经存在的名称。任一屏障失败就返回错误，重试重新执行完整链。它覆盖 payload 目录、catalog 初始化及可写 journal 打开。没有引入进程内“已持久目录”缓存，因此重复小文件写入会增加屏障，开销单独测量。

Verus 验证实际 `directory.rs` 的四个控制函数：返回下一个尚未完成的屏障、仅成功后递增、失败后停止、全部完成才允许成功。证明覆盖所有 u64 计数／状态；四个负向变异分别直接初始化为完成、忽略失败、重复索引、接受未完成链。路径枚举、`create_dir_all`、`canonicalize`、索引／I/O 循环与控制器的完整对应关系仍未证明。

`DirectoryDurability.tla` 区分可见名称和已持久的父目录项。创建失败可以留下任意前缀，sync 失败可以已经产生持久副作用；崩溃移除不可达目录项，后续 payload／叶子同步不能补齐祖先。新建、已存在、已持久及单层目录的正例分别探索 318、300、72、90 个状态。四机实际父链包含 13 个目录（含根），据此增加 Depth=12 的案例，探索 2,730 个状态；这不是最大支持深度。安全性质要求确认数据具有持久路径；进展依赖故障最终停止、弱公平及调用方重试。另一个正例以未持久化的部分祖先开始（312 个状态）。三个算法变异、一个针对部分祖先的配置和一个仅改变目录屏障环境的变异均产生已确认数据丢失反例；负例只选 TypeOK 和 NoLostAcknowledgedData，以越过更早的路径持久性违反，正例检查全部性质。部分祖先反例从 raw 已存在但未持久化开始，后续不再注入错误；旧算法只持久化子项 2、3，遗漏 raw 的项 1，发布确认后崩溃使 payload 不可达。这与实测的重试缺失屏障直接对应，仍抽象了 syscall 顺序及完整 publication。

`ENV-DIRECTORY` 要求有效有限目录链、稳定的根／目录身份／symlink 别名／挂载拓扑，以及使用期间没有外部替换或删除目录。所有解析后的祖先都需要可打开并同步，包括池外祖先；解析目标路径不等于证明外部维护的别名目录项已持久。尚未组合证明真实文件系统命名空间、publication framing 和完整队列。

原生回归逐一检查所有祖先屏障失败、已存在目录的完整重试、实际父链枚举和普通文件拒绝。公开 API 探针对私有 `raw` 目录注入一次 fsync EIO，并记录真实目录同步；观测到的是屏障缺失／补齐，没有实际掉电或数据丢失。[测量与原始证据](FILESYSTEM_zh.md#目录持久化与发布开销2026-10-23)保留这一区别。

## 不确定发布、冲突重试与 GC

中断的发布用私有 `publication:<writer>:<id>` holder 保护外部依赖。旧实现先替换 holder，再检查重试的身份／内容；用空集合或其他依赖重试时，虽然返回 `IdempotencyConflict`，原保护却已移除，GC 实际删除了完成原发布所需的源 pack。目标发布尚未成功返回。这是 straw 所有权适配层的 bug，无需违反 catalog 串行化或 sync 保证。

`publish` 与 `record_set` 现在将先前根和新根取并集，累积到内部 holder；成功完成 staging 交接后才释放。普通公开 `retain` 仍替换指定 owner 的集合。失败或放弃的尝试可能无限期留下额外根；没有新增超时、自动 abort 清理或 catalog 格式变更。

Verus 直接证明 `holding.rs` 循环在任意输入大小下精确保留并集，条件是 `ENV-STRING-ORDER`。循环不变量保留所有旧根，并包含已处理的新根。两个语义变异分别忘记旧根、忽略新根；一个环境变异移除 String 排序前提。闭包计算、BTreeSet 到 Vec 转换、序列化、append 持久性及完整发布适配仍未证明。它是实际 Rust 的渐进证明，不代表整个新增 `Catalog::hold` 方法已证明。

`PublicationRetry.tla` 的三个有限配置分别检查更改依赖、空重试和原根跨两个 pack（42、6、42 个状态）。模型从原 hold 已持久化开始，catalog 动作视为原子持久，GC 可在协议步骤间交错。替换 hold、拒绝时释放、staging 前释放这三个变异均须违反 `NoLostDependency`；负例选择 TypeOK 和数据丢失性质，以越过更早的所有权违反，正例还检查旧根保护、完成交接和最终完成。活性依赖正确重试与弱公平性；未组合 catalog 不确定 frame 恢复、闭包展开、pack 封存或全部发布失败路径。

四个公开 API 回归在旧 wheel 上失败、修复后通过。[四机观测](FILESYSTEM_zh.md#publication-重试与-gc2026-10-23)分别记录真实依赖删除、callback 注入的不确定返回、正确重试和放弃的旧版 fixture 手动清理。

## 独立文件发布完成与 GC

`segment_target_bytes=0` 时，旧 `write_records` 路径将文件登记为 open 并 stage 引用，却没有封存 catalog 项。`record_set` 通过这条路径写 manifest，数据与 manifest 都可能一直 open。独立文件不在当前 packed-file 槽位中，所以显式 `seal()`、`close()`、释放所有权和重新打开都不能修复；`publish` 已有封存步骤。这是成功完成、无 owner 文件被永久保留的问题，不是数据提前删除。

共享完成适配层现在先 stage 根，再封存独立输出，最后才返回。`write_records`、`publish` 直接调用；`record_set` 通过内部 manifest write 使用它，随后先建立结果 staging，再消费临时 record staging。Packed 输出只 stage，继续可追加。重试从两个幂等步骤重新开始，封存报错但已持久生效时也不生成重复输出；publish 的 operation hold 在完成之后才释放。未改变 catalog 格式，也未自动修复历史 open 项。

实际 `finalization.rs` 控制器有 `View` 及 start／next／advance／finished 可执行函数。Verus 证明它们符合 Stage／Seal 转移、进度有界、失败后停止，以及完成需要相应的一步或两步。四个语义对照分别提前 seal、漏掉 seal、忽略失败、过早接受完成。外围 I/O 分发、root／segment 提取和完整 publication 适配仍未证明。

`PublicationFinalization.tla` 从数据已持久化、open 登记已完成开始；stage／seal 报错可有两种持久结果，重试重复完整序列，GC 可在步骤间执行。独立文件的不稳定／稳定配置分别为 24／6 个状态，packed 为 18／6。四个负例暴露释放后无法回收、staging 前删除、忽略 seal 失败或提前封存 packed 输出；它们选择 TypeOK 加指定后果，允许越过更早的不变量违反。正例还在 I/O 最终稳定、弱公平、调用者重试及最终显式释放／packed close 的条件下检查最终回收。这个有限模型不证明任意多文件 publication、完整 catalog 恢复或后端对象删除；下层 GC 模型仍独立存在。

十八个 API 回归覆盖三种 API、packed／standalone 模式，以及 seal／catalog sync 前后失败。旧 wheel 在两个独立文件完成检查中失败，publish 和 packed 对照通过。[四机与 100 GB 证据](FILESYSTEM_zh.md#独立文件发布完成与-gc2026-10-23)分别记录二进制、所有权、真实文件大小和清理过程。

## 完整文件、同步不确定与同 writer 重试

旧独立文件路径在文件 sync 及 after_data_sync callback 之后才设置 pending；任何一处报错都会忘记已经完整写好的私有文件。同 ID 重试生成新 UUID 文件，原 partial／catalog-open 项则无限期留下；packed 模式此前已在 sync 前保存。问题是重复写完整文件、丢失存活 writer 的重试 fencing，不是成功文件屏障的持久性保证失效。

两种模式现在均在初次文件屏障前保存完整描述。实际泛型 `Pending<T>` 保留身份／内容／引用／路径／记录数元组，初始同步状态未知，只有成功响应才单调确认。独立文件重试在结果未知时同步原 partial（若先前 rename 已生效则打开同一最终路径），之后校验完整 extent 并完成目录持久化；即使先前同步已确认，重试也不能省略 checksum 校验。Packed 保留原来的完整检查／重新同步行为。重试不再写 payload chunk，但输入哈希与完整文件扫描仍随数据量增长。

Verus 对实际泛型 Rust 的 start／record_sync／needs_sync 证明描述精确保留、初始未知、仅成功可单调确认、未知时必须同步；四个变异破坏这些契约。外围 Option 赋值、响应分类、路径与描述绑定、syscall 顺序和字节校验没有被这些函数证明；TLA+ 和实现探针分别提供对应边界的证据。

`PendingPublication.tla` 检查 standalone／packed 的同步前、同步中、同步后报错，以及同步未知／已确认时的损坏。同步报错允许已经持久生效。九个有限正例检查只保留一个完整文件、fencing、持久且通过校验的返回，并在公平重试／I/O 最终稳定条件下最终成功或拒绝损坏。四个算法对照分别过晚保存、信任失败、跳过必要屏障、漏校验；一个独立环境对照让成功文件 sync 撒谎。负例选择 TypeOK 加指定后果。未证明进程重启恢复、缺失路径／rename 的进展、完整 frame／描述建立之前的失败、任意重试输入语义或最终 catalog／GC 组合；历史放弃的 partial 不自动修复。

旧 wheel 的六个 standalone 回归失败，六个 packed 对照通过。新版十六个案例还覆盖损坏拒绝、重试没有 payload 写入、正确同步次数与最终 GC。[四机证据](FILESYSTEM_zh.md#pending-文件与同步错误2026-10-23)同时使用真实 fsync 拦截和同步成功返回之后的 callback 失败。

## 命名空间身份与 GC 保护

此前池内符号链接引用可读、retain 也成功，但 owner 保存别名字符串，GC 则按另一条路径登记同一封存文件；释放原 staging 后便可能误删仍有 reader 持有的数据。现在引用闭包、pack、tombstone 和 receipt 统一使用解析后的池内相对路径身份。Replay 也归一化旧路径写法，包括删除后仍保留的悬空别名。新事件保存归一化 key，不重写旧日志字节，也不改变公开引用。

[`NamespaceIdentity.tla`](../verification/storage/tla/NamespaceIdentity.tla) 通过 `EnvironmentNamespace.tla` 消费 `ENV-NAMESPACE`。五个正例覆盖规范路径、reader／writer 别名、旧 owner／tombstone 重放，分别检查 10／10／10／9／5 个状态。六个算法对照分别遗漏 owner、pack、replay、fence、tombstone、receipt 的归一化，必须违反指定性质；第七个对照只改环境身份响应，违反 `NoLostOwnedData`。模型包含一个物理文件、三个名称和串行持久 catalog 动作。最终回收性质依赖弱公平及显式 owner release，尚未组合任意依赖图、字节重放和 I/O 失败。

Verus 直接导入 [`rust/identities.rs`](../rust/identities.rs) 的两个实际函数：在 `ENV-STRING-KEY` 下，lookup 返回已保存身份；remember 仅插入缺失绑定，保留已有绑定并拒绝冲突值。三个语义变异必须失败。这**没有**证明路径解析、前缀构造、readlink 响应真实性、缓存调用点或 catalog 到模型的对应关系。缓存命中不会重新解析并检测外部 retarget；绑定稳定仍是环境要求。

20 个 Python 回归在旧 wheel 全部失败、最终 wheel 全部通过；两个原生测试覆盖缺失／悬空路径和四种旧 catalog 路径字段。Python 案例包括跨 2 GiB、4 GiB、2 TiB 的小 extent 稀疏偏移，不能称为真实 TB 传输。四机别名、旧日志、跨客户端证据和冷重放成本见 [FILESYSTEM](FILESYSTEM_zh.md#命名空间身份与-gc2026-10-23)。Bind mount／inode 等价、任意命名空间变动、旧版不安全 GC 混跑和已丢失数据修复仍不在保证内。

## Catalog 编码与重放工作量

新事件先归一化 owner、pack、tombstone、receipt 的全部路径，再写 `path_encoding=1`。Replay 在应用字段前检查类型／版本，对版本 1 做相对路径词法校验后直接使用其身份。旧事件仍解析名称，不重写已有日志；因此只消除新事件路径的重复查询，并保留别名所有权修复。

[`NamespaceEncoding.tla`](../verification/storage/tla/NamespaceEncoding.tla) 的新格式、旧格式、未知编码正例分别有 11／6／2 个状态。四个对照分别漏掉一个字段的归一化、误信旧别名、接受未知编码、重复解析已记录身份，必须违反 `CertifiedKeys`、`DecodedIdentities`、`UnknownRejected`、`NoResolvedLookups`。最后一项是工作量性质，不是墙钟时延上界。四字段模型尚未组合完整图／GC／崩溃，弱公平只提供条件解码完成。

Verus 直接证明 `identities::encoding` 区分缺失、版本 1 和其余全部 u64 值；三个判定变异失败。它不证明 JSON 类型处理、producer 归一化、framing 或 replay 适配。原生测试让原始别名经过 writer，检查越界路径在 catalog I/O 前拒绝，及删除后的 receipt 重放。12 个 Python 案例检查新字段、八种非法标记和三阶段旧别名恢复；前一版失败于九个新协议断言、通过三个兼容案例。这表明协议新增能力，不表示旧编码契约下又发现了九个 bug。此前别名及稀疏大偏移回归仍保留。

首次 Python 全量检查还暴露一处测试竞争：50 ms 恢复定时器可能在延迟启动的 reader 之前触发，案例实际上没有经历陈旧视图，却因没有重试日志失败。读取前停顿 80 ms 可复现这个缺口。夹具现转发 native stderr，并在观察到首次重试后恢复；即使操作没有读取，退出时也恢复字节。新增回归检查延后 reader 不会提前消除故障。这是测试同步，不是文件系统时延界限的证据。

## 未完成写入的清理与 writer 复用

完整 extent 描述尚未建立时，旧实现直接返回写入错误。独立模式留下 open catalog 项和 `.partial`，普通 GC 不回收；packed 模式在原有已发布前缀后留下死尾部，成功重试又写在死尾部之后，旧数据仍被持有时无法单独回收这些字节。

`Store::write_prepared` 现在成功 open／定位后、写入首字节前保存私有描述和原长度；完整描述交给既有 pending-sync 流程后才清除该状态。失败时 packed 截回准确原长度并同步文件；standalone 删除 partial、同步父目录，然后 seal 从未暴露的最终名称。任一清理失败保留描述并阻止新写／seal，后续调用从首个幂等步骤重试。seal 响应失败后，peer GC 可能已写永久 tombstone，重试此私有退役动作视它为完成条件；一般 pack 注册仍拒绝 tombstone。

Verus 直接验证实际 `incomplete.rs` 函数（新增七项检查，含派生义务），包括描述不变、restart 重置、效果顺序、失败停止、完成条件及 PID 相等判定。六个语义变异必须失败。Python writer 的写入、seal、close、metrics 和 catalog 操作在获取继承 mutex 前检查 PID，防止 child 等待已不存在的拥有线程或清理 parent 的私有尾部；这个检查位置及实际 syscall 分发没有被 Verus 证明。

`IncompletePublication.tla` 从失败写入及正确的 process-local 描述开始，区分当前／持久长度和名称，允许错误已有副作用、peer GC 与存储崩溃。三个正例分别有 252、444、136 个状态；检查前缀长度保留、guard、持久清理后复用、seal 顺序及永久 tombstone 的幂等退役，在最终稳定和弱公平／调用者重试下最终可复用。五个算法负例和一个仅改变 truncate 环境的负例违反指定性质。模型只用长度抽象已确认字节；字节内容保留依赖 `ENV-INCOMPLETE`，不是由长度不变量证明的。

42 个新增 Python 案例在旧 wheel 上全部失败、在修复版通过：24 个 API／模式／失败阶段组合、八个清理重试及 peer GC 边界、三个稀疏大偏移、七个 fork 入口。稀疏起点为 `2**31-5`、`2**32-5`、`2**41-5`，验证 u64 路径及前缀保留，不代表物理 TB 吞吐量。四机另比较真实 2 KiB／8 MiB payload，并在 syscall 前／成功后注入 EIO，见[测量](FILESYSTEM_zh.md#未完成写入与清理重试2026-10-23)。

进程崩溃丢失描述、遗弃的 publication hold 和旧版历史残留仍可能安全泄漏；新名称的登记／创建／seek 失败另由下述准备阶段 guard／模型覆盖。存储崩溃动作保留 live-client 描述，不覆盖 writer 重启；不证明完整锁／catalog／byte-I/O 细化或全部恢复。该修复不扫描、自动封存或清理历史遗弃 writer，也没有新增 TTL 回收依据。

## 缺失父目录与 GC 完成

writer 在实际建目录前登记 open pack。若创建失败，正常 close 可以 seal 这个空 reservation；没有 payload，不代表父目录一定存在。旧 GC 的 unlink 得到 NotFound 后，永远无法打开／同步该父目录，每次收集都报错且不能写 receipt。它可能已删除同批其他文件，因此“调用报错”也不等于没有回收副作用。

实际 GC 现在区分打开成功、NotFound 和其他错误。存在时保留单次目录 fsync；NotFound 时调用已有 `mkdir` helper，补建并同步；全部屏障完成后才进入 receipt。错误仍保留 tombstone／pending 索引供原客户端或新客户端重试。Verus 现直接检查 `gc_io::classify`、`sync_parent` 分派及 mkdir 调用的 `directory_io::sync_all` 有序循环；语义负例拒绝错误分类、跳过／吞掉屏障错误及遗漏修复。完整路径枚举、原生句柄绑定和 repair 持久性的聚合关系仍未证明。

`GCParentRepair.tla` 从永久无 owner、已 tombstone 的名称开始，检查实际缺失及故障期内错误的目录缺失响应、创建部分副作用、同步错误、崩溃和重试。四个正例有 8／38／5／28 个状态；删除持久后才能写 receipt，最终稳定／弱公平／调用者重试下完成。旧策略两个案例分别在一开始就无故障、以及故障最终停止后形成无限重试循环，**违反活性性质 `EventuallyRetired`**。这不是把不进展改写成一个人为错误标记。另两个算法对照和一个仅改变 sync 环境的对照违反 `ReceiptIsSafe`。

运行器支持独立的 temporal negative：配置必须只有指定的一个 PROPERTY，TLC 必须以 13 退出，报告该性质名称及带循环／stuttering 的反例。安全负例仍要求退出码 12 和指定不变量；解析、超时、错误名称均不算成功。原有模型的输入与结果保留。

12 个新增 Python 案例中，旧 wheel 有 11 个失败，正常已有父目录对照通过；修复版全部通过。覆盖缺失前缀、2 KiB／8 MiB 数据、仍被持有的根、修复不同阶段失败、新客户端 replay 和完成后的零重复 I/O。四机观察到修复调用包含 12 次目录屏障（含根），因此另增 `DirectoryDurability` 的 Depth=11 案例；这是部署夹具的边数，不是深度上限，也不是两个模型的组合证明。

`ENV-GC-PARENT` 单列创建权限、幂等 mkdir 和稳定目录身份。模型的 durable absence 不承诺空占位目录持久存在；它不模拟外部替换已有目录再恢复旧绑定。新增目录路径尚未机械细化到无界 GC 活性定理；未关闭／崩溃 writer 的 open 项、独立 writer 早期登记失败及历史泄漏仍未恢复。

## 第一个 payload 字节之前的登记与打开失败

上一版 wheel 在 catalog 登记已经生效但调用报错时丢掉新文件名，即使正常 close 仍留下 open reservation。公共 API 复现在 packed／standalone 两种模式、三个完整 frame／sync 切点成立，后续三次 GC 均不能退役。这是存活 writer 的状态记账 bug，与崩溃 writer 的边界不同。

`Store::write_prepared` 现在先保存新私有名称，再在可失败的准备流程内登记、创建／打开并取得 offset；失败时先释放已打开的描述符，再执行 unlink、父目录修复／同步及 seal。清理报错保留 guard，既有 pack 不走整文件删除路径。准备成功后保留同一身份，在写第一个 header 字节前记录回滚 offset。

Verus 验证实际泛型 `preparing`／`capture_prefix`，包括身份保留、精确 u64 prefix 与模式选择；三个新增语义变异必须失败。真实 guard 调用位置、seek／open 响应和完整 Store／catalog 分发仍未证明。

`PublicationPreparation.tla` 的稳定／不稳定正例为 6／54 个状态；另有旧版遗失 reservation、两个清理／顺序错误，以及一个 unlink 环境反例。它在最终恢复、弱公平和存活调用方重试前提下检查最终解决，没有证明进程重启，也未组合完整文件、所有权与目录模型。

新增 34 个回归在旧 wheel 全失败、修复 wheel 全通过：12 个登记不确定后 close／retry、7 个准备切点、3 个既有前缀、10 个清理／peer GC、2 个 mkdir 阻塞。部分旧版失败是因为尚不存在新故障 hook，不能宣称发现了 34 个独立 bug。原有 12 个缺失父目录 GC 回归现在重建历史空 reservation 状态，因为当前 setup 清理已经修复自己的父目录。

## 完整 publication 的交接与请求入口

`rust/finalization.rs` 的实际泛型 `Handoff<T>` 保存精确描述与 API；`handoff`、`allows`、`acknowledge`、`same_api` 分别证明初始化、请求准入、确认后保持值／API 和枚举相等关系。没有替换正例函数体，也没有新增 assume／admit。四个语义负例分别放行其他请求、忽略 API、初始即非 active、确认后仍 active，必须验证失败。实际 String 相等、调用位置、JSON／内容哈希和完整 Store／catalog 适配仍未证明。

运行时在完整描述生成后、sync 前建立 handoff，跨越 pending sync、staging、standalone seal、record_set 外层 staging／内部 raw owner 消费及 publication hold 释放。pending 检查先于成功缓存查找；不同 API／ID 及关闭 writer 的请求在新建依赖 hold 前拒绝。record_set 直接使用内部准备写入，避免借用公共 write_records 而过早确认。

`PublicationHandoff.tla` 有六个正例（3 API × 2 文件模式），五个算法负例（旧版、过早确认、错误时遗忘、忽略 API、漏释放），一个只改成功 staging 环境的负例。检查安全性和条件活性；原 bug 是重试身份遗失／泄漏，不能把这个反例称为已确认数据丢失。环境与组合边界见 [ENV-HANDOFF](ENVIRONMENT_MODELS_zh.md#完整文件到成功返回的交接)。

新增 43 个 Python 回归：40 个数据 sync／catalog／seal／内部交接切点，加已回收缓存重放与关闭 writer 的两个入口。旧 wheel 42 失败、1 个已回收缓存控制通过；修复后全部通过。原 106 项聚焦套件有一次 fixture 未考虑 partial→sealed 合法重命名而产生 3 个断言失败，修正断言后通过；它不是三个新的产品缺陷。最初 Verus 枚举派生相等未建立所需对应，改用实际穷举 `same_api` 后证明通过；该求证失败也单独保留。

## Staging 消费的持久依赖图

`rust/consumption.rs::covered` 的实际循环证明：成功当且仅当每个子 key 都出现在已有图或本次新增节点集合中，依赖显式 String key 定律。循环还证明遍历有界，正例没有替换函数体。三个语义负例分别接受缺失节点、忽略同 frame 新节点和跳过扫描。实际 consume 重用已有 Verus 证明的泛型 frontier 去重／展开；引用到 key、JSON、真实边、循环调用顺序和原子 frame 仍由适配层提供，未证明完整 consume 方法。

`StagingConsumption.tla` 模拟丢失一次完整释放响应、GC 删除根、独立 reader 保留叶子、重新 staging 叶子及 fresh replay／重试。正例深度 1、3、12 含 2、6、24 个逻辑节点。检查持久图闭合／精确、独立 reader 安全、完成时全图 staging 已消费和条件最终返回。旧版不存图与 replay 遗失图各产生真正的活性反例；漏节点和忽略缓存子边产生安全不变量反例；仅修改环境的依赖遗漏对照违反 `ConsumeComplete`。实际 packed 同文件可读性未细化到这些节点。

新增 61 个 Python 回归覆盖六个 catalog 切点、三种 API／两种模式、共享根、线性直接边、重新 staging 的依赖、读错误、旧日志及无效图。旧 wheel 44 失败、17 通过；其中新格式／新 hook 检查不代表 44 个独立旧缺陷。另有原生回归验证未确认 frame 丢失时同时重建图和 owner。初版把直接 raw 根也写入持久图；四个 1／512-record 成本断言在该版失败，最终版将 raw 根与 manifest 遍历分开，原始根不保存无用节点。另测 raw 根同时被 manifest 引用及空 manifest，防止错误去重或漏记空图。此适配优化没有改变 Verus／TLC 输入，复核输入哈希后复用形式化全量结果，重新执行最终 Python／原生套件。第一次 TLC 聚焦运行的三种正例通过，但运行器拒绝同时指定活性负例和 invariant filter；中间运行曾为兼容该限制移出“释放后必有图”不变量。最终审查恢复了正例和 replay 活性反例的全部不变量；运行器现在允许活性对照显式选择已配置不变量的子集，强制保留 TypeOK，并把范围写入报告。只有旧版活性对照排除 ReleasedHasRecipe，以单独检查已确认存在的进展缺陷；其他五条不变量仍检查。最终完整 TLC 重跑验证这份更强的配置，早期日志和归档分别保留，不算产品缺陷。

## 准备阶段失败后的依赖 owner 清理

`publish`／`record_set` 原来在校验或写入前建立内部 `publication:*` hold。失败后保留依赖供重试是已有契约；问题是存活 writer 没有记住这些准备 owner，即使显式 close 也无法清理。修复在 hold 前加入准备集合，多个失败请求不会互相覆盖。完整文件建立后只将当前请求的 owner 移交 handoff；显式 seal／close 清理其他未完成准备。轮换使用内部 seal 路径，保留准备保护。

清理将一个 owner 移到可重试的当前槽位，释放成功才清空；失败后不覆盖身份，下一个操作先完成已开始的清理。`holding::finish` 的实际泛型 Rust 函数经 Verus 验证：给定成功标志，仅在其为真时清空 Option，否则保持原值。两个语义变异分别无条件清空、始终不清空，必须被拒绝。待清理集合、调用位置、成功标志与真实持久化的绑定及完整 Store 方法尚未证明。

`ProvisionalHold.tla` 的 hold／build／complete 正例分别有 74／68／26 个状态，包含 tracked-owner、准备／完整交接保护、结束后无遗留 hold 及条件完成。七个负例覆盖旧版遗失 owner、过晚记录、清理失败丢描述、误清理完整交接、轮换误清理、报错即释放依赖和后请求覆盖旧准备。第二个请求输入可用、catalog 原子动作和弱公平仍是抽象前提；没有任意图／共享 pack／进程死亡的组合证明。

新增 81 个 Python 回归，旧 wheel 68 失败、13 通过；新 hook 缺失不代表独立产品缺陷。与相关旧测试共 247 项通过，覆盖多请求、源 staging 释放后的原请求重试、清理错误、seal／close 和完整不确定交接保护。显式 seal／close 放弃未完成准备，之后不保证这些原请求仍可重试；已完整的不确定交接继续保留。

第一次修复将早期报错直接视为放弃。虽然聚焦测试和较窄模型通过，完整套件发现原有 `test_incomplete_destination_publication_keeps_dependency` 失败（676 通过、1 失败）。这是实现／模型遗漏已有重试契约，不能修改旧测试掩盖。最终版保留失败准备并增加 `eager_error` 负例；中间二进制、模型、测量和失败日志独立归档，不作为最终证据。更早一次测试误加载系统同版本 wheel，已作废并保留日志；私有安装后检查 native 路径。最初 Verus 命令使用不支持的过滤参数，首次直接证明又发现 Option 后置条件的引用类型写法错误，修正契约后实际函数通过，没有替换正例函数体。

## 单记录四机读取与错误交接的组合模型

`SharedRecordWorkflow.tla` 从一个已经持久发布的 source record 开始，连接准备 hold、完整输出之前失败、释放源 staging、多个 reader、原请求交接、另一失败准备的 close、丢失消费响应、GC 和新客户端释放重试。一个 payload 文件和一个 manifest 是抽象节点；不是在 TLC 中展开 100 GB 字节。ReaderCount 的 1／2／4 正例有 15／17／29 个状态，四 reader 允许失败的正例有 44 个状态。

`EnvironmentRead.tla` 的完整／短读／错误响应在这里抽象为一次 reader 的整个分片最终成功或失败；不展开内部五次重试。reader 失败终止该工作流并保持准备 owner，不保证错误后完成回收。仅完整 reader 返回才移除 pending reader；全部返回后才允许继续。无读错误配置检查最终 done；允许读错误的配置检查最终 done 或 failed，并检查失败状态仍受保护。弱公平作用于响应动作，**真实 syscall／网络请求最终返回仍未证明**，没有从测出的 p99 推导它。

六个算法对照分别清理失败 reader 的依赖、过早清理失败准备、提前越过 reader 屏障、缺失 result owner、close 遗留 owner、丢失消费图；最后一个必须产生真正的活性反例。只改变环境展开边的对照必须违反 `ReleasedHasRecipe`。这组合了特定阶段的抽象所有权契约，仍未证明字节 framing、真实锁、call placement、完整崩溃恢复或任意多文件图的机械细化。新增原生探针用于执行证据，不是 Verus 已证明代码。

## writer 忙碌访问与 callback 等待环

发布调用 Python fault hook 时仍持有 native writer mutex。此前 `metrics`、`seal()`、`close()` 等待同一个 mutex；hook 调用它们时无法返回。另一个等价场景是 writer 的 hook 等待另一调用线程发信号，而该线程先请求 writer。修复让六处 native writer 访问统一经过非阻塞 helper：busy 返回 `ResourceLimitExceeded`，poison 返回 `RuntimeError`，拒绝时不关闭／封存或修改进行中的 writer。独立读取和 catalog 操作不使用这个 writer mutex。

`WriterAccess.tla` 的正常／后续 poison 两个正例各有七个状态。阻塞算法负例要求真正的 `EventuallyFinished` 活性反例（三个状态）；busy 时修改 writer 违反 `BusyPreservesWriter`（三个），错误接纳 poison 违反 `PoisonDenied`（六个）。只改环境、错误发放第二个 token 违反 `ExclusiveAccess`（三个）。正向进展场景与探针一样，由 callback 捕获 busy 异常后正常返回；异常向外传播后的 publication 中止／清理属于另一条路径。模型将其余 I/O 抽象为弱公平下的完成步骤，没有证明真实 I/O 或任意 callback 总会终止。

Verus 检查 `rust/access.rs::decide` 的实际生产函数体，覆盖任意泛型 token 和 poison 标志：成功时返回精确的原 token，无 token 时 Busy，poison 时 Poisoned。三个语义变异分别忽略 poison、拒绝有效 token、误分类 busy。`std::sync::Mutex::try_lock`、结果映射、guard 析构、GIL／FFI 和完整 Python 适配仍是独立的对应义务。集成测试覆盖重入、另一线程上的暂停 writer、2 KiB／8 MiB、standalone／packed、已提交前缀、后续发布和最终 GC；不声称其他锁上的 callback 已普遍支持重入。

## 接收事务后的 staging 清理

队列原先先 retain，再持久化 WAL、应用状态，最后释放 staging。最后一步失败后，同 ID 重试会直接返回业务缓存结果；新进程只重放业务状态，因此也漏掉清理。任务取消后仍留下 staging owner。现在接收 frame 记录 `StagingHandoff`，运行中和恢复时保留待清理项，在新操作／缓存结果前处理；catalog 将完成回执与释放及依赖图放在同一 frame。

仅补重试还不够：回执已持久化但响应丢失后，publisher 可以再次 staging 同一引用。历史交接重放必须只检查已完成 ID，否则可能释放这份新保护。恢复对历史 ID 共享一次 catalog refresh／临界区，已有回执不读 manifest、不追加 frame；未完成项确认 queue owner 仍在，再补清理。恢复拒绝错位、重复、与 queue／sequence／业务内容不匹配或没有根的标记。未确认 catalog 回放被丢弃时，回执索引与 owner／图一起重建。

`CommitCleanup.tla` 有八个正例，涵盖释放前错误、已释放后错误、成功、原实例／恢复、后来 staging 和独立 reader。七个算法对照包括旧版漏清理与 replay 遗失回执的两个真正活性反例，以及失败丢 pending、漏回执、重复消费、只记回执和过早 prune。只改环境的虚假回执必须违反 `CompletedConsumption`。模型从一个已 retain 的根开始；WAL／catalog 原子 frame 是适配契约，没有完整多队列／依赖图／恢复细化。

Verus 验证 `rust/consumption.rs::pop_completed` 的实际泛型函数：确认成功才移除准确的最后一项，错误时保持整个 Vec；`record_receipts` 的实际循环在 String key 定律下将集合变为原集合与输入回执的并集。四个语义变异分别失败时移除、永不移除、忽略新回执和遗失旧回执。验证器计数是 115 项检查，不表示 115 个完整 Rust 方法已验证。实际确认标志、哈希／JSON、调用位置、目录与文件持久化、锁及完整 Coordinator 仍未机械证明。

37 个新增 Python 回归覆盖错误切点、依赖图、重启、缓存／GC 屏障、后来 staging、已回收 manifest 历史及无效回执；原生回归覆盖标记身份／顺序／旧格式和未确认索引重建。旧 coordinator 对新 WAL 标记报 `UnsafeRecovery`；新 coordinator 可恢复旧日志，但不能自动释放其中有歧义的 staging。更详细的责任划分见 [环境模型](ENVIRONMENT_MODELS_zh.md#已提交事务的清理与后来的-staging)。

## 实际队列恢复与大记录 reader 的组合验证

新增原生 `queue_record_probe` 用一个真实 tensor record 及一个 manifest 经过生产 Coordinator：接收后、释放 staging 前报错，关闭并重新打开；生成真实已接收／取消的历史；向四个 reader 发 lease，再取消任务并再次恢复。GC 必须保留所有 reader 的输入。四机以不重叠的完整 checksum chunk 读取，supervisor 逐个确认成功 reader 并再次 GC；最后一个确认后才回收数据与 manifest。数据全部回收后再次恢复，历史回执检查不得打开 manifest 或追加 catalog frame。

`QueueRecordWorkflow.tla` 的七个正例覆盖 reader 数 1／2／4、历史数 0／128／1024 和允许读失败。七个算法负例包括遗失清理意图、漏掉最后确认、回放遗失回执的三个真正活性反例，以及取消／恢复遗失 reader、过早 prune、清理失败 reader 的安全反例。两个只改环境的对照分别提前允许 reader 确认、虚假报告交接回执。模型显式保存各历史交接 ID 的回执，并在 reader 恢复和 GC 后恢复中逐项重放；不能只用历史计数替代这些义务。它组合特定工作流的抽象步骤，并非整个控制面与存储实现的机械细化。

本轮没有修改生产 Rust／Python。新增 Rust 文件是测量程序，未声称它经过 Verus。实际生产证明输入保持不变，仍完整运行 Verus；754 个 Python／38 个原生测试的既有证据仅在逐项核对生产源、测试、Cargo.lock 和 manifest 的依赖／feature／library 配置一致后复用，新增 example 单独编译、执行和 lint。恢复历史的时间、峰值 RSS 与数据读写带宽单列于 [FILESYSTEM](FILESYSTEM_zh.md)，不变成环境时限。

## 大记录范围复制与认证

`RangeRead.tla` 补充实际范围读取循环的有限协议搜索：每个 checksum chunk 分为多个有界 buffer，完整认证后才能对外返回，内部输出只追加与请求相交的片段。每个位置使用不同标签，避免重复数据让错位复制仍然看似正确。正常配置包括尾部短 chunk、chunk 大于文件、1 单位 buffer，以及长度范围内全部空／非空区间；另有持续损坏与短读／错误响应。

七个正例验证安全性及终止；“不该失败时不得失败”与终止一起排除无条件拒绝策略。九个算法负例覆盖不受限 buffer、复制整个 buffer、相对／绝对偏移混淆、跳过认证、漏掉尾部 chunk、提前返回、接受 I/O 错误、拒绝合法输入，以及停在认证阶段的真正活性反例。只修改认证环境必须违反 `NoCorruptReturn`。这不等于从 TLA+ 到 `read_frame_range` 的机械细化。

[位置相关数据记录](../verification/measurements/20261024-positioned-range.json)将每个全局 64 位 word 映射为不同内容，逐 chunk 计算不同的 SHA-256；四个原生 reader 在生产认证之外逐字节核对结果。独立 Python 实现核对小负载的完整数据／checksum，以及远端每个请求的首尾字节。真实 100 GB 与此前稀疏 TB 偏移证据分开。实际 Rust 核心和 Verus 输入未变，新增测量 example 不在生产证明范围。

## 固定 tensor 粒度与实际 Rust 区间证明

`TensorGranularity.tla` 检查新记录固定粒度、摘要完整性、按编码粒度认证、空区间和新记录的认证量上限。配置把长度限制在 0–13、基础 chunk 取 1／2／3，并用三个摘要的旧预算缩小搜索空间；这些数值不是生产配额。六个算法对照分别模拟重新增大新记录的 chunk、按新粒度解释旧记录、漏认证开头、把右端点当闭区间、空请求读 payload、截断摘要列表。旧策略违反的是新指定的成本界限，并非据此发现了返回坏数据的缺陷。

实际 `rust/bounds.rs::authenticated_span` 被 `Config::read_frame_range` 调用，同一函数体由 Verus 验证。它对所有 u64 输入判定合法区间和非零 chunk；对合法非空请求返回完整被触及 chunk 的边界，末端裁剪到 payload 长度，计算无溢出。额外认证量最多 `2 × (chunk - 1)` 字节，单字节请求最多一个 chunk，空区间为零。四个 Rust 语义对照破坏开头对齐、右端点、尾部裁剪、空区间，必须被证明器拒绝。辅助的模运算 lemma 来自固定 Verus release 的 vstd，不添加未经证明的公理或省略函数体。

4 MiB 由 Python producer 选择。Python 代码、metadata／checksum 绑定、JSON 解码、实际 seek/read 和完整复制循环未被该 Rust 算术证明覆盖；`RangeRead.tla` 仍提供独立有限状态检查。新记录的单次 payload 成本界限不包含 envelope/index、重试或物理对象读取。旧记录按自身编码粒度适用对应界限。测试涵盖超过 512 个摘要、跨策略同 ID 冲突及原元数据重试、旧粒度读取、同 chunk 未返回区域的损坏拒绝。详见[测量与验证记录](../verification/measurements/20261024-tensor-granularity.json)。

## Typed 引用的校验顺序

旧路径在 native 返回 payload 后才比较 `TensorRef` 字段；64 MiB 错误引用在 reader 只有额外 16 MiB 地址空间时，先得到分配失败，而非预期的 typed 不匹配。错误 shape 的空切片直接成功。修复把 dtype、shape、length 绑定移入已认证 envelope 与 payload 分配之间，并让空切片执行相同检查；本地 shape／字节数关系在 Python 发出 native 请求前检查。

L1 的 `TypedRead.tla` 穷举长度 0／6 的合法区间和四种布局，允许分配失败；检查错误描述没有 payload 工作、成功返回字段匹配、错误分类、正常描述不被错误拒绝、空请求没有 payload，以及弱公平下结束。三个正例、五个算法对照（一个真正活性反例）、一个只改解码环境的对照覆盖这些义务。小整数用于搜索，16 MiB 的实验内存余量没有成为 allocator 契约。

L3 的实际 `rust/typed.rs::matches` 证明长度、dtype 字符串及每个 shape 维度相等当且仅当返回 true；维度循环没有固定 rank 限制。五个语义对照忽略长度／dtype／rank／维度，或拒绝合法描述，必须被 Verus 拒绝。此证明不覆盖 Python shape 乘积、JSON 到 `Layout` 的映射、调用点位置、完整 I/O 或任何数值时延保证。字节级认证与范围复制仍依赖独立 `RangeRead` 模型和既有测试，不能因 layout 匹配就省略 checksum。见[测量与验证记录](../verification/measurements/20261024-typed-preflight.json)。

## 一次读取复用描述符

`ReadAttempt.tla` 在最多两次重试的有限空间中检查普通读取、冷／热 session、独立缺失区域和持续损坏。五个正例检查一次成功取得描述符、失败等待前释放、认证后返回、界限内不误报和弱公平下终止。Open 抽象成功取得描述符；`open_committed` 内部的 NotFound 重试及其他 open 错误不在此模型中，不能把成功描述符数量当成任意故障下的 syscall 数量。五个算法对照分别重复打开、跨重试保留描述符、跳过 envelope／payload 认证和停止重试；最后一个必须产生真正的活性反例。一个只改环境的对照破坏 fresh-open 可见尝试界限。重复打开违反的是成本策略，不是旧版正确性缺陷；模型没有导出毫秒上界。

实际 `bounds::frame_position` 的 Verus 合约覆盖全部 u64 参数，证明 extent／frame 的表示边界和绝对起点；四个语义变异去掉 extent 容纳检查、extent 地址溢出检查、使用错误 origin 或遗漏 frame header，必须被拒绝。`Config::frame_with_index` 在 seek 前调用它。描述符与索引的真实绑定、缓存 key 比较、Drop／syscall、JSON 和完整 I/O 控制流仍不属于这项算术证明。

`inspect_opened` 在索引检查成功后返回同一个文件；普通读取与 cold session 使用它完成 envelope／payload 读取。warm session 打开新文件并使用已认证索引。session 不保存描述符；所有错误均在 retry wait 前释放本次文件。两个原生回归检查传入描述符在路径不可用后仍可供读取，以及普通／冷／热 session 的失败视图不跨等待保留、下一次尝试看到新的完整视图。测试使用私有文件的 rename 模拟 per-open 视图，不能据此放宽生产命名空间不可变契约。

## 名称查询与数据重试组合

[`ReadLookup.tla`](../verification/storage/tla/ReadLookup.tla) 新增 14 个案例：七个正例覆盖小预算、实际 5／4 次重试预算、零重试、名称／数据超出界限、持续损坏和其他 open 错误；六个算法对照分别提前放弃缺失、重试其他错误、超额名称重试、重置外层预算、缺失却成功、停止外层重试。最后一个必须产生真正的活性反例。另一个只改 `EnvironmentLookup.tla` 的对照破坏名称可见界限，必须违反 `NoFailureWithinContract`。

正例检查总调用／等待预算、等待前释放、认证后成功、其他错误立即停止、失败原因对应耗尽的那套预算，以及弱公平下终止。实际预算正例检查 5,399 个状态；最多 30 次打开和 4.75 秒请求等待只属于这一读取策略，不是存储响应时间定理。模型不假设 p99 为硬界限，也不处理永不返回的 syscall。

本轮未修改生产 Rust／Python，现有 21 个 Verus 证明输入逐一保持不变；119 项正向验证重新执行，105 个语义负向对照复用上一轮同一输入的证据，不能算作新执行。782 个 Python／40 个原生测试也仅在生产／测试源码一致性核对后沿用，新的四机 syscall 注入另行记录。未新增完整 I/O 细化或实际 Rust 定理；详见[测量记录](../verification/measurements/20261024-read-lookup.json)。

## 离线孤儿清理与共享所有权

[`OfflineCleanup.tla`](../verification/offline/tla/OfflineCleanup.tla) 新增十六个案例。七个正例覆盖 catalog、队列命名空间、真正孤儿、root 历史引用、未停机、闭包检查失败，以及报告后新增共享标记。八个算法负例分别使用 stat 授权、忽略 open 错误、跳过重新检查、忽略 queues、忽略停机要求、删除 retained pack、忽略闭包失败和停止清理进展；最后一个必须产生真正的时序反例。另一个仅修改环境的负例使 fresh-open NotFound 撒谎。正例允许删除失败已有副作用，终止性质只要求最终完成或明确拒绝，不承诺自动重试／全部孤儿回收。

旧策略的 `NoOwnedDeletion` 反例对应实际 Python `inspect.py` 的 `Path.exists()` guard，而不是 Rust 删除控制器。两个模型中的共享保护来源分别对应 checkpoint owner 和另一队列已接收的输入；即使所有进程停止也不能删除。新 helper fresh open 两个标记，未知错误抛出 `UnsafeRecovery`；删除前仍重新生成、比较报告。真实 Python 控制流由回归和 syscall 注入检查，尚未做 Python 到 TLA+ 的机械细化。

本轮 Rust 及二十一个 Verus 证明输入未变；119 项正向验证重新执行，105 个语义负例和 40 个原生测试经源码核对后沿用。Python 生产代码变化另行重建 wheel、运行完整 Python 套件。不能把既有 Rust 证明说成已覆盖这个 Python guard；[完整记录](../verification/measurements/20261024-offline-guard.json)分别保存证据。

## 离线清理中的别名保护

[`OfflineAliases.tla`](../verification/offline/tla/OfflineAliases.tla)新增 17 个案例：9 个正向（四种别名形态，分别有／无独立规范路径根，加上解析未知）、7 个算法负例和 1 个轨迹不完整的环境负例。`AcceptedReadable` 同时检查真实文件与必经路径绑定；`NoTargetSubstitution` 防止把删除孤儿别名变成删除其目标；`NoUnknownDeletion` 禁止解析未知时删除；`SuccessfulCleanup` 检查成功完成时确实删掉孤儿。unlink／sync 失败允许已经产生副作用；`Terminates` 指完成或明确拒绝，不承诺无条件回收，停滞变体必须产生真正活性反例。

算法负例分别使用原始名字、只保护最终文件、只保护最终祖先、无条件删 partial、unlink 解析目标、忽略解析错误和停止进展。“只保护最终文件”的负例需要目录别名：指向该文件的普通文件别名已经被身份比较保守保留，因此最初链式文件别名配置正确地没有产生反例。修正后的配置及原始失败检查均归档，避免将负例测试未触发误报为验证通过。

[`inspect.py`](../src/straw/inspect.py)现在解析池内规范身份，并追踪被接受引用经过的绑定，包括链接和 `..` 之前的目录；sealed／pack 与 partial 都受保护检查。报告的 `retained_paths` 改为规范物理身份，删除列表仍保留枚举原路径。成功缓存仅在本次检查内使用。新增 19 个 Python 回归覆盖别名读取、记录／依赖闭包、独立孤儿、悬空别名、目标保留和解析未知。模型提供完整轨迹，Python 解析器及其模型对应关系尚未由 Verus 证明。

[测量记录](../verification/measurements/20261025-offline-alias.json)分别记录本轮 wheel／Python／TLC 检查及未变的 Rust 证明边界。Rust／native 和 21 份 Verus 输入均未变；重跑 119 项正向验证，105 个语义负例与 40 个原生测试经精确输入核对后沿用。没有新增完整文件系统或恢复流程证明。

## 运行成本与验证

普通构建不包含 proof／spec 代码和 ghost 历史，但配套的实现修复与重构仍可能增加运行成本。[Release wheel 对照](FILESYSTEM_zh.md#验证工作中的运行性能对照2026-10-25)同时保留改善和变慢的结果，包括 JuiceFS catalog 重放时延上升。吞吐和尾延迟属于独立部署验收；Verus 通过或小规模 benchmark 都不能证明所有负载无回退。本次测量不扩大实际代码的证明范围。
