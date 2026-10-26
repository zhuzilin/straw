# 打包记录与队列日志

[English](FORMAT.md)

所有整数 framing 字段均为无符号小端。JSON 使用 UTF-8、按键排序和紧凑分隔符，拒绝非有限数值。核心存储原始字节和显式配置的 codec 名称，不会 unpickle 载荷。未知 framing/schema/codec 版本必须报错。格式版本 1 不压缩，offset 指向物理字节；修改这些规则需要新版本。

## 追加 pack 中的不可变 extent

每次 writer 启动拥有一个 UUID 目录。Segment target 非零时，路径为 `raw/<writer-prefix>/<writer-UUID>/<pack-UUID>.pack`。每次 publication 追加一个符合下述 framing 的自包含 extent。`SegmentRef.version=2` 用 `(path, offset, size, checksum)` 定位，`size` 是 extent 长度，不是当前整个文件的长度。后续追加不改变之前的 extent。下一次 publication 将超过 target 时轮换；单次超大 publication 可能超过目标。重启的 writer 使用新的实例身份，不复用可能已中断的尾部。

Target 为零时使用独立 `.sealed` 文件，`SegmentRef.version=1`、offset 为零，并严格校验完整文件大小。Reader 接受两种表示。Pack publication 在返回引用之前 fsync 文件和目录。Sync 结果不确定时 writer 停止接受其他写入，直到以同一 publication 身份重试并确定结果。不会为每个 sample 分配临时文件、manifest 文件或目录。

字节布局：

| 部分 | 编码 |
|---|---|
| Header | 8 字节 `SLMSEG01`，u32 version = 1 |
| 重复 frame | u32 envelope 长度、u64 payload 长度、envelope JSON、payload 字节 |
| Index | 下述 JSON 对象 |
| Trailer | u64 index 长度、32 字节 index SHA-256、32 字节 body SHA-256、8 字节 `SLMEND01` |

Body SHA-256 覆盖 trailer 之前的全部字节，包括 header、每个 frame 和 index。Index SHA-256 仅覆盖 index JSON 字节。Envelope 包含 `version`、`record_id`、`codec`、`metadata`、`tokens`、`length`，以及载荷 SHA-256 的十六进制 `checksum`。允许空载荷，使用空字节的 SHA-256。不允许空物理 segment；空逻辑结果用显式 manifest 表示，含零条记录。

Index 包含 `version`、`run_id`、`segment_id`，以及由 `{offset, envelope}` 组成的有序 `records` 数组。每个 offset 指向 frame 的长度字段。Frame 必须恰好覆盖 header 与 index 之间的区域，不允许空隙、重叠或多余尾部。在返回记录之前校验长度、文件大小、framing、身份和 checksum。Envelope 长度受 u32 字段限制；payload／index 长度使用 u64，frame 算术检查溢出。当前实现没有 10,000 条记录、64 KiB envelope、8 MiB index 或 256／512 MiB record／publication 准入上限。Index／envelope JSON 和完整 record 读取仍需要对应内存，流式 payload 不会让这些分配变成有界。

原生自定义 `Payload` 的 `len()` 只捕获一次，envelope、frame 和统计共用这个值。哈希与写入遍历都必须恰好产生该数量的字节，内容 checksum 也必须相同；超长 chunk 在写入前拒绝，短遍历在 footer 前拒绝，provider 即使忽略 callback 错误也不能把发布变成成功。此前写入可以留下未发布尾部；不承诺回滚或抢占任意 provider 代码。

`SegmentRef` 包含 run ID、相对 POSIX 路径、segment ID、extent 大小、body checksum、校验算法和格式版本。拒绝绝对路径、`..`、空路径组成部分及逃出本地 run 根目录的符号链接。`RecordRef` 额外包含从零开始的 ordinal。客户端自行选择挂载根，引用中不会包含机器绝对路径。

独立 sealed publication 先写私有文件、flush/fsync、检查 close，再 rename 为唯一 sealed 路径，并 fsync 目录。新目录及其父目录也需要同步。Rename 报错时，先验证同一 publication 身份及 body checksum，再重试目录 sync；无法确定的结果返回 `IndeterminateCommit`。文件/目录 sync 失败时不返回成功。每个数据 writer 拥有自己的文件；启用 GC 的池还依赖共享 catalog 的 advisory lock。

## 有序记录集

`RecordSetRef` 是有界控制元数据，包含一个 manifest RecordRef、逻辑 digest、记录数、载荷字节数和 token 数。字节/token 预算传递性地包含声明的依赖，共享依赖可能被保守地重复计算。其 `record-set.v1` manifest 记录包含 `version`、有序 `records`、有序 `dependencies`（其他 RecordSetRef）以及 `digest`。

普通 publication 使用 `record-set.v2`，manifest 与记录处在同一 extent。本地成员表示为较早的记录 ordinal；本地依赖包含较早的 manifest ordinal 和有界逻辑描述符。公开 reader 将它们归一化为普通 RecordRef/RecordSetRef。外部子集仍可使用 `record-set.v1`。两条路径都在 Rust 中计算逻辑 digest。

逻辑 digest 是对 Rust serde_json 生成、按对象键排序的紧凑 JSON 计算 SHA-256：

```
{"records": [envelope, ...], "dependencies": [logical dependency digest, ...]}
```

因此改变物理布局不改变逻辑身份；顺序、记录身份、task/attempt 元数据、codec、token 数和载荷内容都仍然影响身份。外部依赖必须已经持久化，内嵌依赖必须在 extent 内先于父 manifest 出现。整个结果由协调器一次事务接收；共享物理 segment 不会合并任务。Manifest／依赖遍历没有 10,000 个根的准入上限；实现使用去重的堆工作队列，图／索引／缓存内存仍随输入增长，但不再为每层依赖增加一个原生调用栈帧。

协调器在可信 writer 的约定下校验小型 manifest 和 index。Reader 对实际读取的字节计算 checksum。孤立 sealed 数据不会被推断为已逻辑接收。

## 权威日志

只有外部保证唯一的协调器可以写 `control/journal.log`，恢复时重新打开该文件。消费者通过协调器 API 访问，不能跨挂载持有长期描述符并自行 tail。

每个事务包含：

| 部分 | 编码 |
|---|---|
| Prefix | 8 字节 `SLMTXN01`、u64 sequence、u64 JSON payload 长度 |
| Header checksum | Prefix 的 32 字节 SHA-256 |
| Payload | 非空的类型化状态转换事件 JSON 数组；body 长度字段为 u64，当前实现没有 8 MiB 上限 |
| Commit trailer | Prefix + header checksum + payload 的 32 字节 SHA-256，8 字节 `SLMTEND1` |

Sequence 从零开始且连续。第零个事务将 run ID、queue ID、schema、limits、lease 时长和 backend profile 绑定到 `run.json`。没有 journal 轮换。Batch acquire 和 batch submit 各使用一次事务/sync。单任务完成原子地包含终结任务状态、结果引用、receipt、去重键和 accepted-log 位置。

`Submitted` 还可包含 producer ID 和有界、带版本的生产者状态，使数据集游标推进与任务提交成为同一持久化事务。`Yielded` 保存不可变 continuation 输入，将任务退回 pending，但不接收输出。主动 continuation 不消耗失败重试预算；过期、撤销、失败的尝试会消耗。每次新 acquire 都推进授权 generation，并更换 attempt/token 身份。

运行中的实例串行执行 validation → append → fsync → apply → reply。任何不确定的 append/sync 都会使实例进入不可继续写入的 poisoned 状态。恢复拒绝完整帧 checksum 错误、sequence 缺口、损坏 header 或变化的 run 身份；只可截断最后一个不完整事务。暴露恢复结果之前先 sync 完整幸存事务，包括响应曾丢失的事务。恢复服务前，持久化新 epoch 及对旧未完成 lease 的撤销。

## 状态与保留

任务输入和完成结果、batch plan/output、不透明消费者状态及 checkpoint 根都是显式 RecordSetRef。Fetch cursor、连续 processed 前缀、batch-ready 状态和 checkpoint 覆盖范围各自独立。不连续的未完成工作放入带版本的不透明消费者状态。

可选 `progress_ref` 指向一条 `json.v1` 记录，包含 `version: 1`、`processed_positions`（processed 与 fetch 游标之间不重复的稀疏位置）、`finished_batches`（不重复的 ready batch ID）。唯一指定的 `training` 流通过连续前缀和稀疏位置释放生产预算。Finished batch 无需登记 checkpoint 即可释放运行时 ready 容量；启用 GC 后，回收可进一步释放其数据根。恢复消费者视图不会使已删除数据重新出现，checkpoint 必须提前保留依赖。独立重放游标不是 pin。

`TaskSpec.control` 默认为 false。Control task 的生产估算为零，使用单独有界的 pending/in-flight 池（`control_tasks`，默认一个）。必须显式 `acquire(control=True)` 才能获取它，可在生产预算满时完成对已有数据的归集。普通 worker 的默认 acquire 不会获取 control task。其完成结果仍进入同一 accepted log 和统计。这是有界的排空路径，不是绕过生成任务预算的无限通道。

应用可把过滤、组 batch 和 checkpoint 分支决策写入不可变 manifest，再由消费者状态引用。任务语义、来源信息和恢复 checkpoint 的含义由应用负责。

版本 1 重放完整 journal；snapshot 是不可变检查产物，不是权威启动位置。去重历史不会裁剪。离线清理保留日志中所有历史引用及其依赖；任意记录可达，就保留整个 segment。

## 数值元数据兼容性

Python 和 Rust 对部分浮点 JSON 数值的格式不同。逻辑哈希与 manifest 都由 Rust 生成。客户端必须使用 native digest 实现，不能从 Python 序列化的 JSON 重新计算哈希。

## 类型化张量

`tensor.v1` 保存小端连续 CPU 字节，其 envelope 包含 dtype、shape、可选 kind、编码的 chunk 大小及每个 chunk 的 SHA-256。Reader 校验 shape/dtype 对应的字节数，并验证与请求行切片相交的全部 chunk。载荷不压缩，bfloat16 通过 PyTorch 支持。生命周期由 publication、队列、reader 或 checkpoint 的显式所有权决定；单独的 Python 引用不会保留 pack。

新的 `tensor_record`／`publish_tensors` 记录固定使用 4 MiB chunk，包括超过 2 GiB 的 tensor。此前 producer 会将 chunk 倍增，使摘要数不超过 512。Reader 继续使用记录中编码的大小，因此旧记录无需改写不可变元数据即可读取。新记录中 N 字节的非空区间最多认证 `N + 2 × (4 MiB - 1)` 字节 payload；单字节区间最多认证 4 MiB。原生空区间不认证 payload。这些界限不包含元数据读取、内存分配和重试次数；旧记录／外部生成的记录按其编码大小计算对应界限。

摘要列表随 tensor 大小增长：100 decimal GB 需要 23,842 个摘要，相应增加 envelope／index 的内存和处理成本。元数据属于 publication 身份的一部分：在新策略下重新生成大 tensor 的元数据，可能与同 submission ID 的旧尝试冲突。结果尚不确定的重试须沿用原元数据；更换 submission ID 是另一个操作，不是原操作的重试。

## 已登记的 batch checkpoint

`register_checkpoint(id, ref)` 校验一条包含下列字段的 `json.v1` 记录：

```text
version: 1
checkpoint_id: the same id
consumer_state: a RecordSetRef
batch_ids: array of known ready batch IDs
training_dependencies: array of RecordSetRefs
```

外层 publication 必须把 `consumer_state` 和全部 `training_dependencies` 声明为 dependencies，仅在 JSON 中编码描述符不够。应用专有字段可描述其持久化 checkpoint。核心登记并保留这个根，但无法验证外部模型或优化器文件。`release_checkpoint` 显式释放根。

## 共享存储所有权 catalog（可选）

启用 GC 的池只增加一个 `storage.log`。它同时是追加日志和稳定 advisory-lock inode；任何客户端仍存在时，不要替换它。事务获取分布式锁后重新打开 I/O 句柄。每个 frame 使用 56 字节 header 和 40 字节 trailer：

```
header = "STRGC001" | u64-le sequence | u64-le JSON-length
         | sha256(first 24 header bytes)
body = compact JSON {run_id, owners?, packs?, deleted?, reclaimed?}
trailer = sha256(header | body) | "STRGEND1"
```

初始 sequence 为零，body 长度字段为 u64。当前 catalog 实现没有 64 MiB body 上限，重放时会分配完整 frame。应用事件前必须验证 header checksum、sequence、body checksum 和 trailer。完整帧损坏是致命错误。最后一个不完整 frame 可在排他锁下截断；允许新修改依赖幸存的完整前缀之前，必须先 sync 该前缀。

`owners` 替换指定 owner 的完整传递性 pack 路径集合，null 表示删除 owner；`packs` 用 false 表示 open、true 表示 sealed；`deleted` 追加持久化 pack tombstone。这些是持久化持有者集合，不是由 Python 析构驱动的整数引用计数。封存后不能重新打开写入，tombstone 后不能新增保留关系，已回收路径永不复用。

自动加入已有 GC 池使用 fresh open，而非 stat 存在性查询；一旦 Config 家族要求 GC，后续缺失或错误不能把它降回 legacy。自动发现后的 catalog 只能打开已有文件，不能重建空日志。首次 open NotFound 仍需真实反映未启用 GC，池模式需在参与者启动前建立；活跃 legacy 池在线迁移不在协议保证内。需要 GC 的参与者应显式启用 `online_gc=True`。

可选的 `reclaimed` 数组记录已经完成持久删除的 tombstoned 路径。写入者必须先成功同步所有相关父目录，再追加并同步 catalog frame，才撤销待重试义务。Replay 只接受已 tombstone 的路径，将其从待删除和 pack 扫描索引移除，但永久保留 tombstone。完整但结果不确定的 receipt 可以在正常 catalog 恢复中被同步确认；不完整／缺失 receipt 仍需重试。名称不存在本身不是 receipt。 stat NotFound 可能陈旧，不能替代 unlink；必须实际尝试 unlink，并且其成功／NotFound 需要保证名称当前不存在，再完成父目录屏障。之后的 GC 不再 stat／unlink／同步这些已完成 pack，也不追加空 receipt。

这是原有校验 frame 中新增的 JSON 字段。此前批量同步实现会忽略它，安全地重复删除检查；新客户端也能读取没有 receipt 的旧 catalog 并完成剩余工作。四客户端混合二进制检查覆盖两个方向，但只为指定二进制提供证据，不代表任意外部／未来 catalog 写入者均兼容。Tombstone 内存和 catalog 历史仍会增长；没有加入在线日志压缩或有界冷重放成本。


命名空间队列元数据位于 `queues/<sha256(JSON(queue_id))>/`，载荷引用仍相对于公共池根。队列存储 owner 前缀包含控制根身份，只有该队列可对自己的前缀做状态协调。Checkpoint、显式 reader、staging 和其他队列所有者相互独立。队列 lease 还在可重放状态中保留输入读取根；超时、取消和 epoch 变化后继续保留，直到实际读取完成或确认 worker 退役。

持久化顺序、文件系统假设、重放保留终止及已知安全泄漏情况，见[协议与验证](PROTOCOL_AND_VERIFICATION_zh.md)。

目录名称存在不等于持久。每次目录准备均创建所需目录、解析路径并同步从叶子到文件系统根的完整祖先链；失败后重试也不能跳过已存在名称。命名空间／别名／挂载必须稳定，所有实际祖先都需要可打开同步。文件及其直接父目录的同步不能替代祖先目录项屏障。

内部 `publication:<writer>:<id>` owner 累积未完成／被拒绝尝试的根并集；冲突重试不能替换掉旧保护。仅在成功结果已持久 staging 后释放该 holder；公开 `retain` 仍替换指定 owner 的根。该策略沿用已有 replacement frame 编码，没有新增格式字段。放弃的尝试可能无限期保留额外根；peer 不得修改其他 writer 的私有 owner ID。

独立文件（`segment_target_bytes=0`）完成时，先 stage 引用，再将 catalog 文件状态封存；适用于原始 `write_records`、`record_set` 的内部 manifest write 和 `publish`。封存报错即使已持久生效也返回错误，应以同一 publication 重试，staging 继续保护。Packed 发布保持 open，直到轮换或显式 seal。历史 open 项不自动修复，因为它仍可能属于不确定／活跃 writer。

完整 publication 的私有描述必须在尝试文件 sync 前保留；同 writer 重试复用该描述，未知同步结果需要补做文件屏障，再完成独立文件 rename／目录持久化。可沿用已确认的文件同步，但重试仍校验完整 extent；pending 期间拒绝另一个新身份。这是完整 framing 之后的进程内恢复，不恢复任意更早的部分写入，也不复活已放弃 writer 的描述。历史孤立 partial／open 项继续保留。

GC catalog 路径 key 对池内稳定符号链接使用解析后的相对路径身份；所有权闭包、`packs`、`deleted`、`reclaimed` 均归一化。Replay 也归一化旧原始写法，包括目标已 unlink 的名称。Frame 布局不变，不重写旧字节；公开引用和 staging owner 摘要仍沿用原编码。相关别名／前缀必须在历史重放期间保持稳定；原先缺失的私有前缀允许创建为普通目录／文件。所有 GC collector 都需要修复后的策略，旧 collector 仍可能删除别名 owner 持有的文件。这不是 inode／bind mount 身份统一，也不修复已丢失数据，见[命名空间环境契约](ENVIRONMENT_MODELS_zh.md#命名空间契约与实现边界)。

### 已记录的命名空间编码

新 catalog body 包含 `path_encoding: 1`。Writer 写入该标记前，必须归一化 `owners`、`packs`、`deleted`、`reclaimed` 的全部路径，即使内部调用者自行提供标记也不能跳过；同一身份对应冲突 pack 状态时拒绝。Reader 在应用任何字段前检查标记的 JSON 类型和版本。版本 1 的 key 仍须通过相对路径词法校验，之后直接进入索引，无需再次查询文件系统。该标记是 producer 契约，并非签名，也不能证明任意外部日志写入者已经正确归一化。

没有标记的旧事件继续走 resolver，包括悬空别名。null、布尔、字符串、非整数、超出 u64 或未知数值版本均拒绝。旧字节不重写，旧历史仍有查询成本。前一版已修复身份问题的二进制忽略新字段，对已归一化 key 再解析；混合版本实测只覆盖指定二进制。更旧的原始路径 GC 仍不安全。Frame checksum 覆盖标记与 key，framing 不变；仍要求稳定命名空间及合规 producer。虚假版本 1 标记若包含别名，会拆分所有权身份，违反 producer 契约。

### Staging 依赖图缓存

新 catalog 消费 frame 可包含 `staging_edges`：映射 `staged:<引用 SHA-256>` 到直接子 staging key 数组，叶节点数组为空。该字段与 `owners` 的释放更新写入同一完整 frame；replay 先检查节点 key、每条边的目标定义以及与已有节点的冲突，再恢复图。引用图不可变，因此重复／重新分组的释放和重新 staging 的依赖可复用它，根 manifest 已删除也不必重新读取。

旧 frame 无此字段仍能重放；第一次消费尚可读取的旧引用时补记图。旧版消费并回收了未记图的 manifest 后无法补救。旧客户端忽略附加字段，它自己的无图释放仍可能不能重试；混合版本不获得统一的重试保证。格式保留既有 checksum/frame 布局。直接边避免额外存储每个节点的传递闭包，但索引／日志随唯一引用增长，没有回收这些记录或 compaction；这不是常量内存方案。

直接传入的 raw record 根已经明确其唯一 staging owner，释放时不新增图节点；若它被 manifest 依赖，则作为该图的叶节点记录。空 manifest 仍需要保存自己的空边定义。

## 队列 staging 交接凭据

在线 GC 启用且事务含 record-set 引用时，接收事务末尾增加唯一 `{"type":"StagingHandoff","id":...}` 事件，不增加独立 WAL frame。`id` 为规范 JSON `[storage_prefix, sequence, business_events]` 的 SHA-256 小写十六进制；business_events 不含标记，包含接收和去重事件。恢复检查标记位置、唯一性、身份及至少一个根；队列路径身份沿用已有稳定命名空间契约。

Catalog 对应的释放 frame 包含 `consumed_handoffs: [id]`，与 `owners` 的删除更新及新增 `staging_edges` 一起持久化。所有 ID 必须为 64 字符小写十六进制；带该字段的 frame 必须包含对象类型的 `owners`，其所有值为 null。已完成 ID 的重试不追加 frame、不读取 manifest，也不消费后来新建的 staging。回执索引随未确认 catalog 状态一同重建；不能把仅在缓存里出现过的 ID 当成持久完成。

恢复先处理未确认交接，再提交新 epoch；运行中在新操作或缓存响应前经过同样屏障。未标记的旧事务照常恢复，不自动补消费，因为日志不能区分旧泄漏与后来的 staging。旧 coordinator 不认识新事件，会拒绝恢复这种队列日志；不要降级该队列 writer。既有 catalog reader 忽略扩展字段的兼容性不等于旧 coordinator 支持新 WAL。日志、图和回执持续增长，尚无 compaction。
