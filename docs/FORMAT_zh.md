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

Index 包含 `version`、`run_id`、`segment_id`，以及由 `{offset, envelope}` 组成的有序 `records` 数组。每个 offset 指向 frame 的长度字段。Frame 必须恰好覆盖 header 与 index 之间的区域，不允许空隙、重叠或多余尾部。在返回记录之前校验长度、文件大小、framing、身份和 checksum。上限为 10,000 条记录、每个 envelope 64 KiB、index 8 MiB；默认每条记录 256 MiB、每次写入 512 MiB。

`SegmentRef` 包含 run ID、相对 POSIX 路径、segment ID、extent 大小、body checksum、校验算法和格式版本。拒绝绝对路径、`..`、空路径组成部分及逃出本地 run 根目录的符号链接。`RecordRef` 额外包含从零开始的 ordinal。客户端自行选择挂载根，引用中不会包含机器绝对路径。

独立 sealed publication 先写私有文件、flush/fsync、检查 close，再 rename 为唯一 sealed 路径，并 fsync 目录。新目录及其父目录也需要同步。Rename 报错时，先验证同一 publication 身份及 body checksum，再重试目录 sync；无法确定的结果返回 `IndeterminateCommit`。文件/目录 sync 失败时不返回成功。每个数据 writer 拥有自己的文件；启用 GC 的池还依赖共享 catalog 的 advisory lock。

## 有序记录集

`RecordSetRef` 是有界控制元数据，包含一个 manifest RecordRef、逻辑 digest、记录数、载荷字节数和 token 数。字节/token 预算传递性地包含声明的依赖，共享依赖可能被保守地重复计算。其 `record-set.v1` manifest 记录包含 `version`、有序 `records`、有序 `dependencies`（其他 RecordSetRef）以及 `digest`。

普通 publication 使用 `record-set.v2`，manifest 与记录处在同一 extent。本地成员表示为较早的记录 ordinal；本地依赖包含较早的 manifest ordinal 和有界逻辑描述符。公开 reader 将它们归一化为普通 RecordRef/RecordSetRef。外部子集仍可使用 `record-set.v1`。两条路径都在 Rust 中计算逻辑 digest。

逻辑 digest 是对 Rust serde_json 生成、按对象键排序的紧凑 JSON 计算 SHA-256：

```
{"records": [envelope, ...], "dependencies": [logical dependency digest, ...]}
```

因此改变物理布局不改变逻辑身份；顺序、记录身份、task/attempt 元数据、codec、token 数和载荷内容都仍然影响身份。外部依赖必须已经持久化，内嵌依赖必须在 extent 内先于父 manifest 出现。整个结果由协调器一次事务接收；共享物理 segment 不会合并任务。Manifest/依赖遍历上限为 10,000 个根。

协调器在可信 writer 的约定下校验小型 manifest 和 index。Reader 对实际读取的字节计算 checksum。孤立 sealed 数据不会被推断为已逻辑接收。

## 权威日志

只有外部保证唯一的协调器可以写 `control/journal.log`，恢复时重新打开该文件。消费者通过协调器 API 访问，不能跨挂载持有长期描述符并自行 tail。

每个事务包含：

| 部分 | 编码 |
|---|---|
| Prefix | 8 字节 `SLMTXN01`、u64 sequence、u64 JSON payload 长度 |
| Header checksum | Prefix 的 32 字节 SHA-256 |
| Payload | 非空的类型化状态转换事件 JSON 数组，至多 8 MiB |
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

`tensor.v1` 保存小端连续 CPU 字节，其 envelope 包含 dtype、shape、可选 kind、4 MiB chunk 大小及每个 chunk 的 SHA-256。Reader 校验 shape/dtype 对应的字节数，并验证与请求行切片相交的全部 chunk。载荷不压缩，bfloat16 通过 PyTorch 支持。生命周期由 publication、队列、reader 或 checkpoint 的显式所有权决定；单独的 Python 引用不会保留 pack。

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
body = compact JSON {run_id, owners?, packs?, deleted?}
trailer = sha256(header | body) | "STRGEND1"
```

初始 sequence 为零，body 上限 64 MiB。应用事件前必须验证 header checksum、sequence、body checksum 和 trailer。完整帧损坏是致命错误。最后一个不完整 frame 可在排他锁下截断；允许新修改依赖幸存的完整前缀之前，必须先 sync 该前缀。

`owners` 替换指定 owner 的完整传递性 pack 路径集合，null 表示删除 owner；`packs` 用 false 表示 open、true 表示 sealed；`deleted` 追加持久化 pack tombstone。这些是持久化持有者集合，不是由 Python 析构驱动的整数引用计数。封存后不能重新打开写入，tombstone 后不能新增保留关系，已回收路径永不复用。

命名空间队列元数据位于 `queues/<sha256(JSON(queue_id))>/`，载荷引用仍相对于公共池根。队列存储 owner 前缀包含控制根身份，只有该队列可对自己的前缀做状态协调。Checkpoint、显式 reader、staging 和其他队列所有者相互独立。队列 lease 还在可重放状态中保留输入读取根；超时、取消和 epoch 变化后继续保留，直到实际读取完成或确认 worker 退役。

持久化顺序、文件系统假设、重放保留终止及已知安全泄漏情况，见[协议与验证](PROTOCOL_AND_VERIFICATION_zh.md)。
