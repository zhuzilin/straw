# 使用 straw

[English](USAGE.md)

先安装匹配的 [wheel](BUILDING_zh.md)。下面使用公开 Python API；存储和队列协议由 Rust crate 实现。[Examples](../examples) 中提供了仅需 CPU 的完整程序。

## 打开存储池

```python
from straw import SharedFilesystemStore

store = SharedFilesystemStore(
    "/shared/new-job", "new-job",
    online_gc=True,
    segment_target_bytes=1024**3,      # Rotation target, not a hard file-size limit.
    max_record_bytes=16 * 1024**3,
    max_buffer_bytes=16 * 1024**2,
)
```

每个新任务使用唯一目录/run ID；同一个池的参与者共享目录和 run ID。跨多次 publication 复用 writer；进程 fork 后创建新的 store/coordinator 对象。`close()` 封存 writer；`seal()` 封存当前 pack，后续仍可向新 pack 写入。Store 和 coordinator 都支持 `with`。

应从创建池时就启用在线 GC。不能把已有但未跟踪所有权的队列直接切换为 GC 模式，并假定旧引用已经获得保护。GC 是可选能力；catalog 一旦存在，所有参与者都必须遵循它的规则。这些 API 不会创建调度器，也不会启动自动 GC 线程。

## 大记录与有界写入缓冲区

`max_record_bytes` 只保留为兼容参数，不再限制单条记录。`max_buffer_bytes` 仍限制 native writer 临时复制的 payload 内存，不限制整个 record 或 publication。每块最多复制 4 MiB，大记录采用双缓冲复制/I/O 流水线。调用方不需要自行拆分 sample。元数据/索引、调用方已有输入、Python 序列化和整条读回不包含在 scratch 预算内；记录数、依赖数、元数据和索引不再有固定容量配额。

发布返回前不要修改输入 buffer。写入时会再次校验 payload，检测到变化会在提交前拒绝。分块 I/O 保持原有 record 格式及张量切片 API，不会暴露部分记录，也不改变持久化、重试、所有权和 GC 顺序。pack 大小是软轮转目标，大 record/publication 可以超过目标。旧文件仍使用其中保存的 checksum 分块大小；reader 同样不再受旧 record 大小参数限制。

## 进程与线程并发

API 是同步的，straw 不会启动 I/O 进程池。应用可从多个进程、多台机器访问同一个池。每个 writer 需要独立的 store 实例和 pack 写入流，并在多次发布之间复用实例。不要将可变 store/coordinator 对象跨 fork 继承后继续使用。

Rust 在 native I/O、checksum 和日志操作期间释放 GIL，Python 线程可并行执行这些操作。Python 序列化、张量转换和输入缓冲区复制仍有开销。每个 store 同时只允许一个写操作；同实例的重叠写入会报 `ResourceLimitExceeded`，因此需由调用方串行化，或为每个 writer 创建独立实例。引用受到保护期间，reader 可以并发读取。

元数据操作并非完全并行：每个队列只有一个受外部 fencing 保护的协调器，catalog 修改和 GC 共享跨客户端锁。增加进程数不会消除这些串行点或逐事务 fsync 延迟。在语义允许时合并小 publication；异步应用通过有界 executor 调用同步接口。[多进程示例](../examples/multiprocess.py)和 [benchmark 拓扑](BENCHMARKS_zh.md)展示了不同的执行方式。

## 记录与自定义数据

```python
import json
from straw import Record

ref = store.publish([
    Record("text", "hello".encode()),
    Record("config", json.dumps({"temperature": 0.7}).encode(), codec="json.v1"),
], submission_id="publication-1")
records = list(store.read(ref))
assert records[0].payload == b"hello"
assert json.loads(records[1].payload) == {"temperature": 0.7}
```

`Record.payload` 接受 `bytes`、`bytearray` 或连续的字节 `memoryview`。读取返回 `Record`，含字节载荷、codec 名称、JSON 元数据和 token 数。ID、元数据、codec 和记录顺序都会参与逻辑身份计算。元数据必须是合法且不含非有限数值的 JSON；大块内容应放进 payload。

内置名称 `bytes.v1`、`json.v1` 不会替你执行序列化。打开 store 时，用 `codecs=("bytes.v1", "json.v1", "document.v1")` 声明自定义名称，再写 `Record(..., codec="document.v1")`。应用负责编码、schema 校验和解码。straw 校验 envelope、配置的 codec、帧和 checksum；不会导入载荷中的类或执行 unpickle。自定义 codec 的含义变化时应升级版本。

`publish_many([Publication(...), ...])` 将多个逻辑结果放进一次持久化写入，分别返回引用。依赖可以引用已有 `RecordSetRef`，也可以用整数索引引用同次调用中较早的 publication。物理打包不会合并任务身份。

`RecordSetRef` 指向有序 manifest。用 `store.manifest(ref)` 获取记录/依赖描述符，`store.validate(ref)` 获取经过校验的成员引用，`store.read_record(record_ref)` 读取单条记录。`read(ref)` 返回该 manifest 中的有序记录，不会自动展开依赖的载荷。允许空的逻辑 publication，但它的 manifest 仍占用字节。

通过 `dataclasses.asdict(ref)` 序列化引用，用 `RecordSetRef.from_dict(value)` 恢复。引用中是相对路径，不是载荷字节。应用不应自行拼接 pack 路径或 unlink 记录。

## 张量

```python
import numpy as np
import torch
from straw.tensor import publish_tensors

# A tensor store must explicitly include the tensor codec.
tensors = SharedFilesystemStore(
    "/shared/new-tensor-job", "new-tensor-job",
    codecs=("bytes.v1", "json.v1", "tensor.v1"), online_gc=True,
)
features, scores = publish_tensors(tensors, {
    "features": np.arange(24, dtype=np.float32).reshape(6, 4),
    "scores": torch.ones(6, dtype=torch.float32),
}, submission_id="features-1")
assert features[1:3].shape == (2, 4)
assert features.load().shape == (6, 4)
```

支持的 dtype：`uint8`、`int8`、`int16`、`int32`、`int64`、`float16`、`bfloat16`、`float32`、`float64`、`bool`。存储为小端、连续、未压缩数据。CUDA 张量在发布前复制到 CPU；辅助接口不编码 autograd 历史、稀疏布局、对象数组或复数 dtype。空张量和标量可完整加载，按行切片要求存在首维。`.load()` 返回 CPU PyTorch 张量；需要时显式调用 `.numpy()` 或 `.to(device)`。

`ref[start:stop]` 读取连续行，校验与范围相交的 4 MiB 数据块。只支持步长为 1 的切片。这是文件读取，不是 mmap 视图。在 PyTorch 支持的环境里，`load(pin_memory=True)` 可请求 pinned CPU 内存。大张量需要相应调整 store 的记录/发布大小限制；默认限制仍然适用。

批量恢复同一 publication 中的多个张量描述符：

```python
from straw.tensor import TensorRef

dependency, ordinal = features.share(tensors, submission_id="share-features")
restored = TensorRef.from_record_set_many(tensors, dependency)
assert restored[ordinal].shape == features.shape
```

返回映射按 ordinal 包含张量成员，省略非张量记录。`from_record_set(store, ref, ordinal)` 恢复单个成员；多个张量共享 extent 时优先批量恢复。`TensorRef` 还携带本地挂载根目录；切换挂载位置时，应针对目标 store 重建引用，而非直接修改路径字段。

## 共享与写时复制

多个命名空间队列可共用同一个池：

```python
from straw import Coordinator, TaskSpec

a = Coordinator(tensors, queue_id="stage-a", namespace=True, exclusive_owner="owner-a")
b = Coordinator(tensors, queue_id="stage-b", namespace=True, exclusive_owner="owner-b")
a.submit_tasks("a-1", [TaskSpec("task-a", input_ref=dependency)])
b.submit_tasks("b-1", [TaskSpec("task-b", input_ref=dependency)])
```

这些队列所有者保留同一批 pack。`TensorRef.share` 本身只返回描述符；在目标 publication/队列持久化接管引用前，保留源所有者。新应用 manifest 引用共享张量时，使用 `dependencies=[dependency]`。依赖必须属于同一个池/run。

`features.updated(tensors, replacement, rows=slice(...), submission_id=...)` 会物化并写入新的张量版本。原张量保持不可变，未修改的张量继续共享已有引用。完整示例见[双队列张量共享](../examples/tensors.py)。

## 任务与已接收结果

[任务队列示例](../examples/work_queue.py)包含完整流程：

1. 发布输入记录，以稳定请求 ID 提交 `TaskSpec(task_id, input_ref=...)`。任务元数据和生产者游标状态应为小型 JSON。
2. `acquire(worker_id)` 返回带输入与 lease 的 assignment。区分 `empty`、`backpressured`、`end_of_input`、`draining`。
3. 在 lease 的 reader 所有权保护下读取输入。发布输出时，元数据包含该 lease 的 `task_id`、`attempt_id`。
4. `complete_task(lease, submission_id=..., result_ref=..., result_digest=...)` 将接收结果写入日志，返回稳定 receipt。响应丢失后，使用完全相同的身份/引用重试，或调用 `lookup_submission` 查询。
5. `read_commits(cursor, limit)` 返回有序结果页和下一游标。数据尚被保留期间，多个 reader 可独立重放。

响应不确定时，物理发布重试 ID 和逻辑完成 ID 都应保持稳定。在同一幂等键下改变内容会报错。不要假设 `submission_id` 会对任意写入进行全局去重；队列接收记录才是权威事实。崩溃的计算可能重新执行。

通过 `heartbeat` 续租。用 `save_task_progress`/`save_task_progress_many` 在保留 lease 的同时持久化中间输入；`yield_task` 或 `release_tasks` 把未完成任务退回队列，不消耗失败重试预算。`fail_task` 与超时会消耗尝试预算。只有全部字段都对应同一个完整、一致的前缀，才能发布 continuation。

`Limits` 的字段仅用于兼容旧调用方及队列恢复身份，不再限制 pending/in-flight 任务、结果字节/token/记录数、ready batch 或控制消息。应用通过 acquire 请求数量和自己的调度器控制并发，原有使用量统计保留。移除了单结果 4 GiB、累计 64 GiB、1 亿 token、1 万记录/依赖节点、64 KiB envelope、256 KiB 控制元数据、8 MiB 索引/manifest/事务日志以及 64 MiB catalog 事务等配额。恢复旧队列仍应传入与原先相同的 Limits 字段、codec、lease 时长、run ID 和 queue ID。

保留真实格式和正确性检查：u32 envelope 长度、u64 计数、文件边界与校验和、task/attempt 归属、lease、失败重试数及同一 writer 不可重入。移除逻辑配额不代表无限内存/磁盘；元数据仍需占用内存，payload 写入继续使用有界 scratch。

## 生命周期与在线 GC

| 所有者 | 建立方式 | 结束方式 |
|---|---|---|
| Publication staging | 发布成功 | 队列接管，或 `release_publications` |
| 显式应用/checkpoint | `store.retain(unique_owner, refs)` | `store.release(owner)` |
| 临时 reader | `with store.pin(refs): ...` | 确认读取真正结束后退出 |
| 队列输入和持有 lease 的 reader | Submit / acquire | Completion/yield/failure，并在必要时显式确认旧 reader 已停止 |
| 已接收结果 | 队列接收 | 指定消费者确认处理完成，GC 结束相应重放保留 |
| Ready batch | `batch_ready` | 所有 rank/reader 读完后，在消费者进度中列入 `finished_batches` |
| 已登记 checkpoint | 用有效根调用 `register_checkpoint` | `release_checkpoint` |

保留关系是持久化 owner 集合，不是 Python 引用计数。GC 不依赖 `del`、析构或 lease 超时。`retain` 会替换该 owner 的全部根；需要独立保留的版本必须使用不同 owner ID，并先保留新版本再释放旧版本。不要使用内部的 `queue:`、`staged:` owner 名称。

`store.collect_garbage()` 回收已经没有 owner 的封存 pack。队列应调用 `queue.collect_garbage()`，让已确认消费的队列历史先持久化结束其根的保留，同时检查每个存活队列根都有所有权。所有权缺失、数据损坏或存储错误必须上报给应用，不能当成可忽略的清理告警。

协议 v1 使用指定消费者 ID **`training`** 统计消费，即使应用本身不是训练。用 `open_consumer` 打开，再通过 `save_consumer_state` 提交已发布的状态引用、`fetch_cursor` 和 `processed_cursor`。处理完成确认必须覆盖所有必要 reader。可选的 `progress_ref` 是一条 `json.v1` 记录，包含 `version: 1`、用于稀疏确认的 `processed_positions` 和用于 batch 读取完成确认的 `finished_batches`。其他 consumer ID 保留自己的已保存状态，但不会自动分别保留每条已拉取结果。主处理游标推进前，应为更慢的重放 reader pin 数据。示例包含一个 checkpoint，演示处理完成后仍可阻止回收。

拉取不等于确认处理完成；模型更新完成不等于 checkpoint 已持久化。结束历史重放保留后，旧 receipt 元数据可能仍在 WAL 中，但载荷已经不可用；应提前保留 checkpoint/重放所需根。仅关闭/释放 consumer token 不会丢弃已保存的消费者状态。

队列 lease 过期、取消和恢复，都无法证明 reader 已停止。`outstanding_reads()` 返回仍存在的读取 lease。监督程序只有确认读取已完成或进程已死亡，才能调用 `release_task_reads` 或 `retire_worker`。不确定状态的 reader、遗留 open pack 和 staging 会被保守保留，不会按 TTL 删除。

## 恢复与 checkpoint

重新打开 `Coordinator(..., recover=True, exclusive_owner=<真实外部保证>)` 前，先停止旧协调器，并保证它无法恢复运行。恢复会重放 WAL、持久化新 epoch 并拒绝旧 lease。稳定的已接收 receipt 保留，未完成尝试可能重试。进程重启后重新创建客户端对象。[多进程示例](../examples/multiprocess.py)在恢复和重放前，会 kill 并 join 自己的协调器进程。

应用 checkpoint 必须先保留全部引用数据，再发布最终持久化指针。记录精确的应用版本、队列/消费者位置和数据依赖；只有各组件均持久化后，才发布已提交 manifest。随后显式释放旧 checkpoint。straw 无法推断外部优化器或数据库提交是否持久化。

`queue.snapshot()` 是检查用产物，不是权威启动 checkpoint，也不会传递性地 pin 数据。队列始终从日志恢复。高级 `register_checkpoint` 的 batch checkpoint schema 见[格式文档](FORMAT_zh.md)，它仍要求应用完成最终提交。

所有存储/协调器调用都是同步的，异步应用应使用有界 executor。控制通道由应用选择；`straw.rpc` 中的小型 JSON HTTP 通道面向示例和私有任务网络，不是托管的多租户服务。详见[应用集成](APPLICATIONS_zh.md)。

## 批量读取会话

连续读取不可变 publication 时，可共用一个 `with store.read_session() as reader:`。Reader 提供 `validate`、`manifest`、`read`、`read_record`、`envelope` 和带校验的张量读取。将它传给 `TensorRef.from_record_set_many(store, publication, reader=reader)` 与 `tensor.load(reader=reader)`，可避免重复解析同一 extent 的索引。

Rust reader 只保留最后一个已认证索引，以完整 extent 描述符区分身份。每次 manifest/record 读取仍校验 frame 和载荷；张量读取仍校验触及的 chunk。每次 `validate` 都重新检查逻辑描述符与 task/attempt 权限。切换 extent 或开启新会话后重新认证索引。整个读取过程必须由持久 owner 或 `store.pin` 保护：读取会话本身不保留存储所有权。退出上下文会关闭会话，fork 后继承的会话拒绝所有操作。

发布依赖校验、批量续跑进度、消费者快照及 GC 遍历在 Rust 内部也复用这种有界 reader。文件格式、WAL 提交顺序、容量限制和所有权释放规则均未改变。

`store.read(publication)` 同样在迭代器生命周期内持有读取会话。释放数据 owner 前应读完或关闭迭代器。不同迭代器会重新认证索引。


### 持久化任务排序

`TaskSpec.priority`（默认 0）降序，其次 `scheduling_key`（默认 0）升序，再按 FIFO；两个字段均为有符号 64 位整数。排序字段及提交/归还顺序会从 WAL 恢复。Rust coordinator 维护有序 pending 索引，选取任务无需读取 payload。

先发布续跑输入，再调用 `yield_tasks(updates, request_id=...)`。每项包含 `lease`、`input_ref`，可同时替换 `priority`、`scheduling_key` 和 `metadata`。整批校验后只提交一次 WAL；所有 lease 同时结束，任务进入其排序键的 FIFO 队尾。任何 stale lease 都会拒绝整批。相同请求重试幂等；只有确实需要另一个输入版本时才重新发布。与此不同，`save_task_progress_many` 保留 lease。

`pending_tasks()` 返回按领取顺序排列的任务 spec，可用于暂停后的快照；暂停边界和输入引用持有由应用协调。这些 API 需要 `straw-queue>=0.1.1`。
