# 文件系统部署

[English](FILESYSTEM.md)

straw 使用**已挂载的 Linux/POSIX 文件系统**，依赖真实文件、目录、追加写、文件/目录 fsync 和 advisory file lock。它不会发起对象存储 API 请求，也不替代文件系统客户端。单机可用本地文件系统，多机需要访问同一份共享池内容。

## 0.1.4 候选版性能对照

[发布对照记录](../verification/measurements/20261026-release-comparison.json)在同一台机器、同一 JuiceFS 挂载上，按旧版／新版／新版／旧版顺序比较 0.1.3 与 0.1.4 候选版。每轮使用新目录并开启在线 GC，发布及认证读取 1 GiB（32 × 32 MiB），追加 500 个 WAL frame，重放 2,000-frame catalog，并执行任务接受、退休和回收。WAL 与 catalog 恢复各运行五次。

| 指标 | 0.1.3 | 0.1.4 候选版 | 变化 |
|---|---:|---:|---:|
| 发布吞吐 | 134.93 MiB/s | 107.00 MiB/s | −20.7% |
| 认证读取吞吐 | 314.18 MiB/s | 322.91 MiB/s | +2.8% |
| 500 次同步 WAL 追加 | 8.395 s | 8.094 s | 耗时 −3.6% |
| 接受 64 个任务 | 2.843 s | 3.832 s | 耗时 +34.8% |
| 回收 24 个 pack | 153.84 ms | 161.28 ms | 耗时 +4.8% |

表中数值是每个构建两轮进程运行的中位数；GC 先取每轮三次回收的中位数。全部阶段成功。未清空缓存，紧随写入的读取和重复恢复可能命中客户端／服务端缓存；不能据此推断多机峰值带宽或延迟上界。两轮新版均出现发布及任务接受回退。此次比较的是包含持久化与 writer fencing 改动的完整构建，未做 profile，尚不能归因。记录保留全部样本及确切 wheel、原生库和探针哈希；下文历史测量仍只代表各自的构建和负载。

## 0.1.4 候选版：四节点读取单条 100 GB 记录

独立的 [100 GB 测量](../verification/measurements/20261026-release-100gb.json)由一个节点发布一条真实记录，payload 为 100,000,000,000 字节，内容随字节位置变化。四个 reader 分工覆盖全部字节，并逐字节核对独立计算的预期值。它与上面的 32 MiB publication 对照属于不同负载，不是版本间比较。

| 阶段 | 实测结果 |
|---|---|
| 发布前 checksum 准备 | 113.501 s，排除在 publication 计时之外 |
| 单节点 publication | 358.855 s；265.755 MiB/s |
| 四节点读取及逐字节 oracle 的总 wall time | 55.275 s；合计 1,725.333 MiB/s |
| 各节点 native 完整分片读取吞吐 | 593.8–625.8 MiB/s |

publication 本身仍包含位置相关 payload 的重新生成、认证和同步。读取总 wall time 包含调度、边界切片和逐字节 oracle；各节点 native 计时不含随后执行的 oracle。缓存未清空，这些逻辑吞吐不是冷读或后端存储带宽上限。单次 publication 不能提供 publish p99。

释放所有权后，正常 GC 报告回收一个文件、100,003,195,567 字节（包括 framing 和元数据）。这表示文件系统命名空间中的删除已持久化，不代表已经测得对象存储后端物理释放空间的时刻。报告保留确切探针源码与原生二进制哈希。

## 四节点可见性采样

[可见性探针](../verification/measurements/20261026-release-visibility.json)使用一个 writer 和三个远程 reader，每种模式发布 1,000 次、每次 64 KiB。共 6,000 次读取全部成功，没有失败重试。从 fsync 返回到收到校验成功回复的实测 p99，保持 writer 打开时为 9.515 ms，关闭 writer 后为 9.035 ms。该指标包含通知、fresh open、读取、hash 校验和回复，不是纯文件系统可见延迟，也不是必然延迟上界。两种模式使用同一挂载且未清空缓存；所有探针进程正常退出，临时数据已清理。

## 必须满足的约定

| 操作 | 必须提供的语义 |
|---|---|
| 文件 fsync | 在部署声明的故障模型下，成功确认使之前写入的字节持久化 |
| 目录 fsync | 可以使新建名称和删除操作持久化；不能忽略失败 |
| 其他客户端 sync 后重新打开 | Reader 能获取已提交 extent/日志前缀 |
| 跨客户端 advisory lock | 所有 catalog writer/collector 在同一个稳定 inode 上串行化 |
| 唯一队列所有者 | 监督程序阻止两个协调器同时写同一队列 WAL |
| 不可变文件身份 | Pack 名称唯一，封存后不再修改，也不复用 |

Catalog 使用一个固定的 `storage.log` inode。客户端仍存在时不要替换或重命名它。事务持锁后会重新打开 I/O 描述符，以读取新内容；持有锁本身不意味着所有文件系统缓存均已失效。

远端 unlink 后允许暂时存在过期的文件属性缓存：删除已经 tombstone 的文件时，ENOENT 是幂等结果。其他错误仍必须报错。

所有协调器应使用相同、规范化的绝对池挂载路径；队列 owner 身份目前包含其控制根路径。相对记录引用可以通过其他挂载根读取，但不能用这种方式迁移运行中协调器的所有权命名空间。变更本地根时，应针对读取 store 重建 `TensorRef` 描述符。

为防御可能暴露陈旧或不一致视图的部署，reader 容忍长度、header、索引和数据分别延迟可见，并对相关校验失败做有限次 reopen/revalidation。这超出了 JuiceFS 的默认契约，不表示正确部署一定会产生这些延迟。文件系统必须在该重试预算内提供完整正确视图，才能保证成功；没有界限的最终可见性只能保证持续错误不会被当作有效数据返回。WAL/catalog 仍要求持锁后重新打开能读到完整已提交前缀；不要把 extent 的重试策略推及日志截断恢复。参见 [TLA+ 假设](FORMAL_VERIFICATION_zh.md)。

## JuiceFS 与其他分布式文件系统

使用直接客户端挂载，并针对实际客户端版本和存储服务验证锁、同步、恢复与可见性语义。显式 JuiceFS profile 需要提供部署声明：

```python
from straw.backend import FilesystemBackend
from straw import SharedFilesystemStore

root = "/shared/new-job"
backend = FilesystemBackend(root, profile="juicefs", declaration={
    "direct_mount": True,
    "writeback": False,
    "open_cache": 0,
    "readdir_cache": False,
    "client_version": "your deployed version",
    "durability_description": "your verified metadata/object-store durability contract",
})
store = SharedFilesystemStore(root, "new-job", backend=backend, online_gc=True)
```

这些是**运维提供的声明**，不是自动发现的挂载配置。将占位字符串改成某个版本号并不等于完成服务认证。默认 `local` profile 使用同样的 POSIX 调用，但不要求 JuiceFS 声明；profile 名称不能证明底层挂载的文件系统类型。

信任某个部署之前，在隔离目录上运行[多客户端故障/GC 检查](VERIFICATION_zh.md)。SIGKILL 进程后，内核和存储服务仍在运行；它不能证明掉电、元数据服务故障切换、客户端断连或设备写入重排下的保证。这些故障需要文件系统/厂商单独认证。

一手资料：[JuiceFS 缓存文档](https://juicefs.com/docs/community/guide/cache/)、[POSIX 兼容性说明](https://juicefs.com/docs/community/posix_compatibility/)、[Linux fsync 约定](https://man7.org/linux/man-pages/man2/fsync.2.html)。

`FilesystemBackend` 提供部署声明和测试故障 hook，载荷 I/O 在 Rust 中运行。继承该类并不能实现任意云存储后端。新 I/O 后端必须在 native 层实现并验证同样的持久性约定，或明确采用另一套协议。

## 可见性契约与测量

官方[社区版缓存文档](https://juicefs.com/docs/community/guide/cache/)描述默认 close-to-open 一致性，并说明 fsync/fdatasync/close 会刷新写入，使其他客户端可见。[云服务/企业版缓存文档](https://juicefs.com/docs/cloud/guide/cache/)同样描述 close-to-open 一致性，以及关闭 writeback 时先上传数据、后提交元数据。open cache 会削弱跨客户端立即可见的保证；writeback 可能使元数据先于上传完成而可见，此时读取可能阻塞或失败。两份文档都没有把成功同步后的任意零填空洞规定为正常默认行为。TLA+ 区域空缺模型用于保守容错，不能由模拟反例推断 JuiceFS 实现存在缺陷。归因前还需记录实际客户端版本和挂载配置。

可运行有界的原始读取探针，只依赖 Python 标准库：

```sh
python verification/storage/probes/extent_visibility_probe.py \
  --parent /shared/test-tmp --samples 1000 --output /tmp/visibility.json
# 使用能访问相同路径的不同 SSH 客户端，沿用已有 SSH 信任和认证配置。
python verification/storage/probes/extent_visibility_probe.py \
  --parent /shared/test-tmp --hosts HOST_B HOST_C \
  --samples 1000 --output /tmp/visibility-cross-client.json
```

探针创建两个私有 append-only pack，使用 straw 的 header/frame/index/footer 布局，每次发布都 fsync。每次追加前先让 reader 缓存尾部/EOF，再比较 writer 保持打开与 close/reopen 两种方式。通知通过 pipe/SSH 在 fsync 后发送，不使用可能掩盖传播延迟的文件系统标记，也不经过 straw 内置重试等待。reader 每次重新打开，独立校验长度、header、footer、索引和完整 extent 哈希。不注入故障，不清缓存，不改变挂载。默认写入约 128 MiB，`--max-write-bytes` 默认以 512 MiB 限制本轮实际写入，可显式设置。`--start-offset` 可在 extent 前创建至多 8 TiB 的稀疏空洞；它只验证寻址，不代表全量 I/O。仅在 worker 停止后清理新建的私有目录；远端运行失败时保留目录，因为 SSH 断开无法证明远端进程已退出。

`fsync_to_ack_ms` 使用协调者的同一个时钟，无需跨机器对时。其 p99 是包含通知、调度、读取、哈希和回复开销的**观测上界**。按 reader 顺序收集回复还可能抬高后续 reader 的上界。`reader_probe_ms` 不含这些控制通道开销。失败尝试和区域/checksum 错误单独统计；成功读取的分位数不掩盖超时数量。没有陈旧读时，应报告“未观察到”，不能说传播延迟精确为零。本机多进程共享一个挂载/客户端，测出的 p99 不是跨客户端 p99；新路径也不代表冷后端存储。实测 p99 和最大值都不是最坏情况保证，也不能直接作为 TLA+ 的活性假设。

## 文件数与空间规划

- 长期使用的 writer 把多次 publication 及其内嵌 manifest 追加到 pack，默认轮换目标为 **1 GiB**。不会为每个 sample 创建临时文件或旁路 manifest 文件；`publish_many` 也能减少事务开销。
- 每次 writer 启动都有独立目录和当前 pack；writer 越多，未填满的 pack 越多。每个 sample 都创建 writer 或 seal 会失去打包效果，应在进程/worker 范围内复用。
- 下一次 publication 将超过目标大小时先轮换。单次超大 publication 保持连续，因此可能超过目标。除目标大小外，也要配置 record/buffer 限制；目标大小不是配额。
- 每个队列有 `run.json` 和一个 `control/journal.log`。命名空间队列共享数据 pack 与池级 `storage.log`。用户 owner 名称是日志条目，不是文件。可选 trace 每个进程增加一个追加文件。
- GC 只删除完全失去引用的封存 pack，一条存活记录就可能保留整个 pack。必要时将保留周期差异很大的数据写入不同 writer 流；目前没有存活记录搬迁/整理。
- 崩溃的 writer 可能留下 open pack/staging 所有权；空 writer 目录和 WAL/任务历史也会随运行时间增长。超时无法证明它们已经死亡。

根据形状/dtype 和保留的活跃版本估算张量字节数，再加上 framing、未填满 pack、checkpoint、reader、排队结果和 WAL。Pack 数量近似为数据量除以目标大小，再加各 writer 的未满 pack；publication 大小和显式 seal 都会变化，因此这不是严格数学上界。队列逻辑预算可能重复计算共享数据，也不限制物理占用。

应用应设置准入/保留预算与文件系统配额。Benchmark 有显式 `--max-files`、`--max-gib` 防护。每个有界任务使用新池，删除池前停止全部参与者。引用仍存活时，不要通过直接删除 pack 文件来强行满足配额。

## 带宽验证与实测参数

正确性证明不提供带宽保证。小操作应看延迟和每秒操作数；大 publication 应看持续有效载荷带宽、内存和读取放大。判断“够快”之前需要定义工作负载和目标：record 大小、发布批次、客户端/并发数、fsync 边界、读取 fanout 和缓存状态都会影响结果。

标准库探针分别测同步写、跨客户端原始读、带校验读；最后一项可能复用第一次读的缓存：

```sh
python verification/storage/probes/filesystem_bandwidth_probe.py \
  --parent /shared/test-tmp --hosts HOST_A HOST_B HOST_C HOST_D \
  --mib-per-host 2048 --streams 4 --sync-mib 16 --output /tmp/raw.json
python -m straw.benchmark run --root /shared/new-queue-benchmark \
  --hosts HOST_A HOST_B HOST_C HOST_D --writers 4 --readers 4 \
  --record-bytes 16777216 --gib 8 --max-gib 10 --max-files 128 \
  --segment-mib 256 --report /tmp/queue.json
```

队列测试包含发布、协调者接受和重叠的跨客户端环形校验读。读写速率使用同一端到端时长，不是独立设备读写带宽，不能相加后称为存储容量。新路径且不清缓存的测试不能认证冷对象存储性能。

[2026-10-23 测量清单](../verification/measurements/20261023.json)保存源码/二进制/报告哈希及原始报告归档位置。四个 JuiceFS 客户端共完成 10,800 次同步发布、32,400 次跨客户端可见性校验，未观察到陈旧视图或重试。最初的 64 KiB 矩阵轮换四台 writer；新增尺寸/偏移测试使用一台 writer 和三台远端 reader。无法发现实际客户端版本及 writeback/open cache 配置，因此不构成厂商认证。

| Payload / 起始偏移 | 每种模式发布次数 | fsync 到校验回复 p99：保持打开 / close-reopen |
|---|---:|---:|
| 2 KiB / 0 | 1,000 | 10.38 / 8.16 ms |
| 1 MiB / 2 GiB − 5 | 200 | 15.52 / 13.46 ms |
| 16 MiB / 4 GiB − 5 | 100 | 57.73 / 46.05 ms |
| 2 KiB / 2 TiB − 5，仅稀疏边界 | 100 | 3.38 / 14.04 ms |

这些数字校准测试场景和性能预期，**不用于设置 `VisibleBy`，不证明最大延迟，也不能据此缩短重试等待**。文档契约对应的 fresh-open 场景仍使用 `VisibleBy=0`；延迟视图场景仍是独立的保守故障假设。下方 100 GB 实写替代了原计划中的 TB 全量测试；TiB 稀疏测试只保留寻址覆盖。

| 四机工作负载，合计 8 GiB payload | 观测合计速率 |
|---|---:|
| 原始同步写，每机 1 流，每 16 MiB fsync | 982 MiB/s |
| 原始同步写，每机 4 流，每 16 MiB fsync | 3,683 MiB/s |
| 队列＋环形校验读，16 MiB/publication，每机 1 writer/reader | 每个方向 369 MiB/s |
| 同上，每机 4 writers/readers | 每个方向 430 MiB/s |
| 队列＋环形校验读，64 MiB/publication，每机 1 writer/reader | 每个方向 739 MiB/s |

这些是短时校准。额外并发带来的队列收益小于增大发布批次，值得继续 profile 批处理和串行控制操作，但尚不能据此证明瓶颈位置，更不能跳过 fsync 或校验。原始与队列工作负载不同，两者比值不是仅衡量库开销的严格对照实验。

另外，native 探针（`cargo build --release --example large_extent_probe`）在一个真实 record 中发布了 **100,000,000,000 字节 payload**，三台远端各读取校验完整数据。每次返回 256 MiB 范围，校验 scratch 最多 1 MiB。发布包含 envelope/segment 哈希与 fsync：277.20 秒、344 MiB/s、进程峰值 RSS 10.9 MiB。reader 耗时 153.75–159.16 秒，单机 599–620 MiB/s；包含 SSH 分发的合计读取约 1,792 MiB/s，峰值 RSS 260.6–260.8 MiB/机。全部通过且私有数据目录已清理。这只是一次发布，**不能给出 100 GB 发布的 p99**。载荷重复使用随机 8 MiB 模式，生成和 checksum metadata 准备不计入带宽时间；没有清缓存。测试覆盖 native storage API，不覆盖 Python tensor 构造或协调者 RPC。

### 四客户端过滤 acquire 延迟（2026-10-23）

`verification/protocol/probes/lease_selection_probe.py` 在同样四个 JuiceFS 客户端上测量修复后的筛选代码，每台使用独立新队列、1,024 个 pending 任务、批量 16，两种过滤各执行 250 次并交替运行：唯一 ID 列表，以及每个 ID 重复四次后打乱的列表。全部 2,000 次 acquire 共返回 32,000 个批内不重复、当前有效的 lease；heartbeat、release、WAL 恢复检查均通过，每台核对了实际 native 哈希。临时目录已清理，WAL 合计 13,268,772 字节。

| 过滤输入 | 合并样本数 | acquire p50 | acquire p99 | 最大值 |
|---|---:|---:|---:|---:|
| 1,024 个唯一 ID | 1,000 | 17.09 ms | 41.62 ms | 71.70 ms |
| 4,096 项／1,024 个不同 ID | 1,000 | 18.45 ms | 44.07 ms | 155.18 ms |

耗时包含本机 Python/native 控制处理、筛选以及 lease WAL 写入／fsync，不包含网络 RPC 传输、heartbeat/release、队列准备和恢复。这是元数据控制操作延迟，不是 payload 带宽，也不是单独测量去重 CPU 成本；更大的输入也改变序列化和排序开销。合并 p99 只是本次负载的观察值，不提供硬截止时间、存储收敛界限或调度公平性保证。证据和输入哈希见 [`20261023-selection.json`](../verification/measurements/20261023-selection.json)。

### 批量优化前的历史 tombstone 开销（2026-10-23）

归档的优化前 `verification/gc/probes/gc_history_probe.py` 在每台机器的独立池中依次累计 0、32、128 个已删除 pack，每档每台采样 250 次后续 GC。每个 pack 只有一条 1 KiB record；创建、释放和首次删除不计时。重复调用每次新增回收文件数／字节数均为零。另一次不计时的 callback 检查确认：每个历史 tombstone 仍触发一次目录同步。

| 历史 tombstone 数 | 合并样本数 | GC p50 | GC p99 | 最大值 |
|---|---:|---:|---:|---:|
| 0 | 1,000 | 10.77 ms | 29.85 ms | 43.48 ms |
| 32 | 1,000 | 23.46 ms | 47.52 ms | 124.06 ms |
| 128 | 1,000 | 64.57 ms | 103.48 ms | 119.08 ms |

计时包含 catalog 锁／刷新、stat 和重复父目录同步，不是单个 syscall 成本。档位按数量递增运行，缓存和后端负载未受控。结果说明历史扫描／同步值得优化，但仅因名称不存在就跳过屏障，会破坏中断删除后的持久性；本轮未做这项优化。这不是延迟上界、TB 规模历史或载荷带宽结果。四台均核对同一 native 哈希，私有数据与临时环境已经清理。完整样本和输入哈希见 [`20261023-gc-progress.json`](../verification/measurements/20261023-gc-progress.json)。

### 批量目录同步对照（2026-10-23）

在同样四台机器重新对照逐 pack 同步版和批量目录同步版，每版各用独立池和环境，核对 native 哈希。每池只有一个 writer 目录，按 0、32、128 个历史 tombstone，每台每版每档采样 250 次重复 GC。两台先跑逐 pack 版，两台先跑批量版。相同探针用 `--expected-barriers per-pack`／`per-directory` 验证对应同步次数；该选项是探针断言，不是 GC 运行时策略开关。

| 单目录 tombstone 数 | 逐 pack 版 p50／p99 | 批量版 p50／p99 | 每次同步次数：原来 → 现在 |
|---|---:|---:|---:|
| 0 | 11.15／28.40 ms | 11.10／27.27 ms | 0 → 0 |
| 32 | 21.99／46.77 ms | 19.69／42.53 ms | 32 → 1 |
| 128 | 57.46／97.54 ms | 48.42／84.45 ms | 128 → 1 |

每格合并 1,000 个样本。128 个 tombstone 时，合并 p50／p99 下降 15.7%／13.4%，但两台机器的 p99 变差，合并最大值也从 127.09 ms 升至 261.71 ms；没有证据证明尾延迟普遍改善。同一目录布局下，同步次数减少是确定的，延迟仍受缓存和后端负载影响。档位按数量递增，两个版本并非在同一主机同时执行。创建、首次删除、探针 callback 不计时；包含 catalog 刷新、历史 stat／扫描和 Python/native 调用，未分离单个 syscall 成本。批量同步仍扫描每个 tombstone，未解决无限历史增长，也未建立延迟 SLA。

6,000 次计时调用全部通过，准备阶段累计创建／删除 1,024 个小 pack；私有数据和临时环境已清理。两个二进制、逐机样本及哈希见 [`20261023-gc-batch.json`](../verification/measurements/20261023-gc-batch.json)。

### 持久回收凭据对照（2026-10-23）

下一轮对照上一版批量二进制和记录持久回收凭据的新版。四台各自使用独立池／环境，核对 native 哈希；在 0、128、1,024 个历史 tombstone 下，每台每版每档采样 250 次重复 GC。两台先旧版、两台先新版。每池只有一个 writer 目录，每个 pack 一条 1 KiB record，档位按数量累积。

| 已完成 tombstone 数 | 批量版 p50／p99 | Receipt 版 p50／p99 | 每次重复调用的 payload 父目录屏障 |
|---|---:|---:|---:|
| 0 | 11.17 / 24.78 ms | 11.19 / 24.71 ms | 0 → 0 |
| 128 | 55.32 / 111.90 ms | 11.30 / 27.80 ms | 1 → 0 |
| 1,024 | 51.75 / 379.55 ms | 11.07 / 20.81 ms | 1 → 0 |

每格合并 1,000 个样本。另一次不计时的 callback 检查确认：新版已完成项的 payload stat、目录屏障、receipt append 均为零。旧二进制没有 stat callback，因此没有把旧版 stat 数当作测量报告。两个版本仍加锁、刷新并同步 catalog。计时不包括创建、首次删除、探针 callback；这是热控制调用，不是 payload 带宽或冷 replay。

首次非空回收新增一次 receipt append／sync。每版每档只有四次首次回收样本，不足以给出 p99 或分离额外开销：128 个新增 pack 时，旧／新版范围为 589–763／557–739 ms；随后新增 896 个时，为 3.81–4.46／3.68–4.15 s。对应调用的 catalog 增长从 10,377 到 20,756 字节、从 71,817 到 143,636 字节；这是 GC 事件增量，不是完整 catalog 大小。空重复调用不新增 receipt。永久 tombstone 和日志仍无界增长；移除已完成索引项并未消除冷 replay 和集合容量成本。

6,000 次计时调用全部通过，准备阶段创建／删除 8,192 个小 pack。另用三个 pack 在四台依次检查新版写入 → 旧版读取／回收／写入 → 新版恢复／回收 → 新版重开，前三阶段保留的 live 引用一直可读。该结果只覆盖这两个二进制，不代表所有历史版本兼容。私有数据和临时环境已清理。逐机样本、首次调用成本、交错结果及哈希见 [`20261023-gc-receipt.json`](../verification/measurements/20261023-gc-receipt.json)。缓存／后端负载未受控，实测尾延迟改善不能作为上界或任意历史规模保证。本轮控制路径修改没有重新测量 100 GB／TB 带宽。

### 校验原生流式长度后的 100 GB 重测（2026-10-23）

修复原生 payload 长度／callback 契约后，沿用 `large_extent_probe` 源码、链接新核心；四台核对相同可执行文件哈希。64 MiB 预跑通过后，实际发布一条 100,000,000,000 字节 record，三台远端并发完整读取。这是独立于此前 bounds 实验的新一轮数据。

| 操作 | 本次观察 |
|---|---|
| 100 GB 原生发布，包含 envelope 哈希、framing、fsync | 271.37 秒；351.43 MiB/s；峰值 RSS 10.75 MiB |
| 三台远端各自完整校验读取 | 149.31–157.97 秒；603.70–638.71 MiB/s；峰值 RSS 260.63–260.83 MiB |
| 包含 SSH 分发的三 reader 合计 | 返回并校验 300 GB payload，约 1,806.29 MiB/s |
| 三台合并的完整 256 MiB 范围请求 | 1,116 个样本；p50 393.86 ms；p99 537.96 ms；最大 592.58 ms |

分位数排除每台最后一次较短请求。这是三个并发原生 Reader session 内的顺序请求，样本相关、已认证 index 可以复用，没有清缓存。Payload 重复使用随机 8 MiB 模式；模式生成与 chunk metadata 准备不计入写入耗时。含 framing／manifest 的文件长度为 100,000,051,365 字节，这不是设备或对象存储的实际流量计数。只有一次大 publication，不能给出其 p99，也不能与早期运行做受控开销归因，或推出 Python／RPC 带宽、TB 实际 I/O 结论。读取 p99 不代表陈旧视图收敛界限或完整重试耗时上界。两个私有数据目录均已清理。

准确源码、二进制、逐范围样本和检查结果见 [`20261023-stream-length.json`](../verification/measurements/20261023-stream-length.json)。

### 依赖深度与控制延迟（2026-10-23）

四客户端分别在深度 128、2,048、16,384 下创建独立池。每池将一条 1 KiB 叶子 publication 放在一个 sealed pack，将合法的小型 inline manifest 链放在另一个 pack。每台每档采样 50 次校验，核对当前 wheel 的 native 哈希，并将进程栈限制为 512 KiB。每次调用新建 native Reader，同次调用内可复用已认证 index。

| Manifest 链深度 | 合并样本数 | 校验 p50 | p99 | 最大值 |
|---|---:|---:|---:|---:|
| 128 | 200 | 49.01 ms | 96.57 ms | 170.27 ms |
| 2,048 | 200 | 810.28 ms | 1,178.05 ms | 1,262.77 ms |
| 16,384 | 200 | 6,273.88 ms | 8,010.08 ms | 8,222.98 ms |

600 次调用全部通过。随后每池 retain 链根、释放 staging，检查 GC 保留两个可达 pack 且叶子仍可读；释放最后 owner 后再回收两个 pack。每档只有四次首次链根 retain 观察，范围分别为 87.75–100.92 ms、533.34–1,107.95 ms、5,605.34–7,147.86 ms，不能称为 retain p99。Store 已有 raw-reference／叶子闭包缓存，之前的校验也预热了文件系统；请求的链根此前尚未 retain。

池相互独立，档位递增，四机并发，后端负载／缓存状态未受控。校验计时包含本机 Python/native 调用、图解析、逐节点文件 I/O 和 metadata 认证；不包含建池、GC，也不测叶子 payload 带宽。消除递归栈增长并未让深图校验变便宜，不能推出常量堆内存或延迟上界。单次读的可见性重试等待也不是包含数千次独立读取的整图遍历上界。私有数据池和远端环境已清理。准确输入、旧二进制崩溃、样本及检查结果见 [`20261023-dependency-traversal.json`](../verification/measurements/20261023-dependency-traversal.json)。

### Checkpoint 回退的 payload 扫描（2026-10-23）

`reactivation_probe` 在四机核对相同原生二进制哈希。每台每档独立建池，先持久保留引用大记录的 checkpoint、释放 publication staging；再重复调用真实 `Catalog::retain_reactivated` 50 次，每次之间释放恢复 owner。800 次计时包括 catalog 锁／refresh、依赖 metadata 校验、完整 payload SHA-256 和 owner append／sync；释放与 GC 不计入，未经过 Python／RPC 或完整 Coordinator 回退 API。

| Payload | 合并样本数 | p50 | p99 | 最大值 |
|---|---:|---:|---:|---:|
| 2 KiB | 200 | 37.36 ms | 82.76 ms | 91.41 ms |
| 1 MiB | 200 | 40.29 ms | 135.69 ms | 149.14 ms |
| 16 MiB | 200 | 51.64 ms | 100.16 ms | 140.31 ms |
| 64 MiB | 200 | 89.07 ms | 192.77 ms | 227.30 ms |

各池首次调用包含冷 catalog 实例，随后复用 catalog／闭包缓存；每次扫描都新建 Reader，文件系统缓存未清除。四机并发但池独立，同池锁争用不在该对照中。档位递增，后端负载未受控，分位数不必随大小单调增长。测量进程最高 RSS 为 3.65 MiB；这是观察值，只有 payload scratch 的 1 MiB 上限属于算法契约。

之后由第一台实际发布一条 **100,000,000,000 字节** `bytes.v1` 记录，由第二台执行一次完整恢复扫描：

| 操作 | 本次观察 |
|---|---|
| 原生发布，含 envelope 哈希、framing、fsync 与 staging | 284.45 秒；335.27 MiB/s；峰值 RSS 10.63 MiB |
| 远端完整 reactivation，含校验和 owner 持久化 | 107.45 秒；887.52 MiB/s；峰值 RSS 3.68 MiB |

Payload 重复随机 8 MiB 模式，模式生成不计入发布计时；checkpoint 准备也在计时之外。该恢复读取完整 100 GB payload，不分配返回 payload。只有一次大记录观察，不提供 p99；校验字节数是逻辑数据量，不是设备／对象存储流量计数。它与此前 tensor 范围读取是不同负载，不能据此作受控性能对比，也不证明 TB 实际 I/O 或冷缓存带宽。

每池最后释放 checkpoint 后，GC 回收 checkpoint pack、保留恢复 owner 引用的 payload pack；释放恢复 owner 后再回收 payload pack。17 个池的 34 个 pack 均按此生命周期回收，私有目录和远端二进制已清理。

**本轮二进制的完整校验仍在 catalog 锁内运行**。这次内存修复消除了整条 payload 分配，未消除随 payload 增长的 I/O 与持锁时间；大回退可能阻塞同池其他 catalog 操作。上述耗时不能作为锁等待、恢复或可见性的硬上界。准确样本、二进制、崩溃回归与形式化检查见 [`20261023-reactivation.json`](../verification/measurements/20261023-reactivation.json)。

### 锁外恢复与同池 catalog 竞争（2026-10-23）

四机核对相同原生探针及 wheel native 哈希。第一台发布数据，第二台执行恢复，第一、三、四台作为三个并发 peer。每个 peer 持久保留一个 1 KiB publication，反复执行一次 retain 加一次 release，计时包含这两个 catalog 事务。三个实验各用独立池，每池的五个 pack 最终均通过 GC 回收。

首先使用同一个二进制、同样的 2 KiB payload，在验证开始时人工暂停 2 秒，对比低层 `Catalog::retain_reactivated` 持锁路径和 `Config::retain_reactivated` 锁外路径。这是诊断性暂停，不是测得的文件系统延迟：

| 路径 | 三个 peer 首个操作对的耗时范围 | 观察到扫描结束标记之前完成的操作对 | 恢复总耗时（含暂停） |
|---|---:|---:|---:|
| 持锁兼容路径 | 2247.32–2294.13 ms | 0 | 2.12 秒 |
| 锁外路径 | 348.88–392.80 ms | 18 | 8.68 秒 |

该观察支持“校验期间其他客户端能继续操作”，不支持“整体恢复一定更快”。锁外那次总耗时反而更长；当前探针没有分开记录初始 catalog 访问、扫描和最终重新获取锁的耗时，不能把差值全部归因于某个阶段。对照样本很少，不作为稳定 p99。标记文件只用于协调测量，存在客户端可见性延迟，不是同步时钟或形式化进展证据；锁是否释放另由确定性的跨进程回归检查。

随后在**没有人工暂停**的情况下发布单条 100,000,000,000 字节记录，并在三个 peer 持续操作同池 catalog 时完整校验、重新检查所有权并提交恢复 owner：

| 项目 | 观察值 |
|---|---|
| 100 GB 原生发布 | 271.63 秒；351.09 MiB/s；峰值 RSS 10.77 MiB |
| 100 GB 恢复（含前后 catalog 事务） | 106.79 秒；893.00 MiB/s；峰值 RSS 3.64 MiB |
| Peer retain／release 操作对 | 1361 个样本；p50 211.67 ms；p99 386.68 ms；最大 433.56 ms |

Peer 循环在操作对之间等待 10 ms，并在观察到扫描结束标记后停止；每台的首次 catalog 实例与后续热缓存调用一起统计。负载／缓存未受控，没有清缓存，也不测 Python／RPC 的完整恢复 API。Payload 重复随机 8 MiB 模式，模式生成和 checkpoint 准备不计入发布耗时；带宽使用逻辑 payload 字节，不是设备流量。只有一次 100 GB 恢复，不能给出其 p99、冷缓存保证或 TB 实际 I/O 结论。

完整 payload 扫描已移出 Coordinator 的 catalog 临界区；metadata 闭包遍历、刷新、重新检查及 owner append 仍持锁，并多一次 catalog 访问。保护来源在扫描期间消失时可以明确失败。测量不能建立锁公平性或延迟上界。三池十五个 pack 的保护／回收检查通过，私有目录和远端环境已清理。源码、精确样本、二进制和检查结果见 [`20261023-unlocked-reactivation.json`](../verification/measurements/20261023-unlocked-reactivation.json)。

### 恢复耗时的阶段对照（2026-10-23）

为排查上面的 8.68 秒样本，原生探针新增本机单调时钟事件，分别记录候选检查、完整校验、最终 catalog 访问、append、sync 与返回；marker 写入和注入暂停也单独计时。库代码和模型未变。四机核对同一新探针二进制及原有 wheel/native 哈希，使用 16 个新建池、每池 2 KiB payload；有竞争的轮次交替运行持锁和锁外路径，每次仍注入 2 秒暂停。

| 场景 | 每条路径的样本数 | 持锁路径总耗时 | 锁外路径总耗时 |
|---|---:|---:|---:|
| 无并发 peer | 2 | 2.087–2.091 秒 | 2.107–2.137 秒 |
| 同池三个 retain/release peer | 6 | 2.073–2.140 秒 | 2.115–2.181 秒 |

六次有竞争的锁外调用中，校验为 2.55–8.24 ms，最终 catalog 阶段为 25.70–87.98 ms，owner append 后的 sync 为 14.00–25.37 ms。最终 catalog 阶段混合了重新获取锁、refresh、重新检查和准备 append，**不是纯锁等待**；持锁路径在这一阶段已经拥有事务，二者不是相同的子操作。锁外试验中 peer 完成了 239 对 retain/release，其中 224 对在各自观察到结束 marker 之前完成；持锁对照为 18 对、0 对。marker 仍只是客户端观察。

这次**未复现** 8.68 秒，不能给旧样本补做归因，也没有获得稳定 p99、锁等待上界或新的大文件带宽结果。没有据此改变数值策略或环境前提。16 池共 68 个 pack 通过保护／回收检查，私有数据和四机环境均已清理并独立复核。原始事件、样本、精确二进制及本轮验证范围见 [`20261023-reactivation-timing.json`](../verification/measurements/20261023-reactivation-timing.json)。

### Catalog 重放失败后的同进程恢复（2026-10-23）

这轮测量主动注入故障：先让另一个 catalog 客户端写完 owner frame、在 sync 前报错；原客户端重放它时，对其 `storage.log` 的下一次真实 fsync 注入一次 EIO。随后显式保留或丢掉这个未确认尾部，再用原对象重试。已确认前缀保持不变。Linux `LD_PRELOAD` shim 只作用于探针指定的私有日志及一次 arm，不链接进库；这不是 JuiceFS 服务断电测试，也不是观察到部署真实丢数据。

四台机器各使用隔离旧／新版 wheel，并校验实际 native 哈希。每次试验新建一个池，发布并封存 2 KiB 数据，最后验证所有权和回收。旧版在四台机器的丢尾部试验中均持续误报 `UnsafeRecovery`，新建对象才能完成；尾部幸存对照正常。新版每台机器各做 25 次丢失和 25 次幸存，共 200 次，全部能由原对象重试完成；幸存的未确认 owner 会在 sync 后保留，显式 release 之前不能 GC。

| 新版尾部结果 | 样本数 | 重试 p50 | 重试 p99 | 最大值 |
|---|---:|---:|---:|---:|
| 丢失 | 100 | 29.20 ms | 81.20 ms | 162.54 ms |
| 幸存 | 100 | 26.92 ms | 46.87 ms | 52.09 ms |

计时只包含同一对象重试 `release_publications`，包括 refresh／必要的全量重放／sync 和 release；排除故障注入、尾部重写及后续 GC。样本来自四机各自的独立池，缓存和背景负载未控制；不能当作长历史恢复上界、常态故障率、带宽结果或 p99 SLA。旧／新版共 208 个试验池及 pack 均完成检查与回收，私有共享目录和远端环境已清理。源码、精确二进制、样本和验证记录见 [`20261023-catalog-replay.json`](../verification/measurements/20261023-catalog-replay.json)。

### 空刷新失败与 catalog 历史规模（2026-10-23）

上一轮的缓存修复在每次 refresh 报错后都重建，包括没有新 frame、缓存完全未变的空刷新。四机对照使用上一轮已修复尾部丢失的二进制作为基线；优化版只在缓存可能含未确认修改时要求重建。每台机器为每个规模创建独立池，原生发布并保护一个 2 KiB pack，再在没有其他客户端操作时离线追加合法的交替 retain／release frame，最后统一 fsync。两个版本读取相同历史，按主机和规模交替先后顺序；这不是逐事件提交吞吐测试。

| 附加历史 frame | Catalog 大小 | 基线空刷新失败后重试 p99 | 优化版重试 p99 |
|---|---:|---:|---:|
| 0 | 1,170 B | 30.47 ms | 22.93 ms |
| 4,096 | 0.73 MiB | 37.07 ms | 19.33 ms |
| 65,536 | 11.66 MiB | 122.18 ms | 17.40 ms |
| 262,144 | 46.63 MiB | 449.53 ms | 17.68 ms |

每格合并四机各 25 次、共 100 个样本；只计时注入一次空刷新 fsync EIO 后的重试，排除注入本身。另有新建 catalog 对象和正常热调用两组，每版／规模也各 100 次，总计 2,400 个计时样本。最大规模下，新建 catalog 的 p50 为基线 411.16 ms、优化版 415.51 ms，正常热调用 p50 均约 11 ms。**优化没有加速必要的完整重放**，只去掉了空刷新失败带来的冗余工作。“新建 catalog”不表示文件系统缓存冷，实验未驱逐缓存，也未控制背景负载。

独立的本机 strace 对照使用 4,096 条附加历史加 5 条原生基础 frame。整个跟踪会话的 catalog `read` 从 36,915 次降到 24,612 次，恰好少 `3 × 4,101 = 12,303` 次；实际成功 fsync 均为 4 次。会话包含首次加载、热对象初始化、校验和检查及一次失败重试，因此总数不是单次重试的 syscall 数。注入的 EIO 由 shim 返回，不进入内核 fsync；跟踪耗时不用于上表。这支持“省去一次完整解析”的归因，没有削减成功同步屏障。

四机另用新二进制通过 40 次未确认尾部丢失／幸存回归，所有权和最终 GC 均正确。16 个历史池、40 个故障回归池及独立 trace 池都已回收并清理。这些数据只有一个存活 pack，不代表大量 owner／tombstone 扫描、巨大单帧、payload 带宽或延迟上界；计数是测量规模，不是 API 配额。证据见 [`20261023-catalog-history.json`](../verification/measurements/20261023-catalog-history.json)。

## GC 陈旧 stat 与删除凭据（2026-10-23）

[JuiceFS Community 缓存文档](https://juicefs.com/docs/community/guide/cache/)将 negative lookup 缓存列为可配置项，默认关闭；这不能确定当前挂载的实际参数，也不能把 stat 与 unlink 的返回语义混为一谈。本轮没有修改挂载参数。

先在四机各轮流创建 100 个 2 KiB 文件：另外三个 reader 预先对该名称 stat 得到 NotFound，writer 完成文件／目录 fsync 并关闭后，再经持久 SSH 管道通知 reader 查询。1,200 次观察均第一次就看到文件；查询耗时 p50 0.255 ms、p99 9.245 ms、最大 21.424 ms。这是收到通知后的单次 stat 观测，不是跨机时钟计算的传播延迟；没有得到负缓存收敛界限，也没有测吞吐。

随后对真实原生发布的 sealed、无 owner pack 注入**一次 stat ENOENT**，unlink 及目录同步仍由真实文件系统执行。旧版四机共 8 例均跳过 unlink、写入 receipt，重复调用和新客户端重放后文件仍存在；探针最后明确手动清理了这些私有孤儿。修复版共 100 例均删除实际文件，重复／重开检查通过。计时包含 tombstone、注入 callback、unlink、父目录同步和 receipt：p50 82.68 ms、p99 163.11 ms、最大 203.34 ms。错误旧版的较少工作不能作为性能基准。

修复将 stat 降为诊断计数，pending 项仍必须尝试 unlink。没有大小观察但成功删除时，`reclaimed_files` 加一，`reclaimed_bytes` 加零；字节计数不是存储容量审计。unlink 自身的 NotFound 仍需可信，这次修复不证明所有缓存配置的 unlink 新鲜度，也不会自动修复历史错误 receipt。所有私有数据和四机临时环境已清理。输入、native 哈希、逐次样本和完整验证见 [`20261023-gc-stat.json`](../verification/measurements/20261023-gc-stat.json)。本轮未重新测量 100 GB／TB 数据吞吐。

## Catalog 有界预读对照（2026-10-23）

对照上一版 stat 修复后的二进制与增加 64 KiB 预读的最终二进制；四台机器各自创建私有池，离线追加 0／4,096／65,536／262,144 条合法 retain/release 历史。每档每版每机 25 次，共 2,400 次 cold／warm／空刷新失败后 retry 计时。两版读取相同日志，按主机／档位交替先后顺序。cold 仅指新 catalog 对象，不清除 OS／JuiceFS 缓存；不是冷存储读或每条提交吞吐。

新对象首次 GC（ms，每格合并 100 个样本）：

| 额外历史 frame | 原版 p50 | 预读 p50 | 原版 p99 | 预读 p99 |
|---:|---:|---:|---:|---:|
| 0 | 17.97 | 19.05 | 68.65 | 41.10 |
| 4,096 | 23.14 | 20.19 | 36.36 | 258.36 |
| 65,536 | 118.12 | 72.31 | 163.47 | 169.50 |
| 262,144 | 417.05 | 232.32 | 485.81 | 396.09 |

262,144 条历史档的 p50 降低约 44.3%，但尾延迟并非普遍改善：4,096 档预读 p99 有 258.36 ms、最大 278.18 ms 的慢样本；65,536 档 p99 也略差。热刷新和 retry 没有新增缓冲分配，测量仍有波动，不能宣称普遍加速或延迟 SLA。完整 replay 仍随历史线性增长；64 KiB 是策略选择，未搜索或证明最优。

独立 syscall 对照在 4,101 个总 frame（约 0.73 MiB）上执行两次完整解析：原版 read 24,612 次，预读版 30 次；两者均为 11 次 seek、4 次实际成功 fsync。总 read 含两次共同的流式 checksum 检查；注入失败在测试 shim 中直接返回，没有执行 kernel fsync。追踪耗时没有混入上表。

针对慢样本另在四机各跟踪最终版 200 次新对象调用。一个 252.32 ms 样本中，236.76 ms 位于 `flock(LOCK_UN)` 内；800 次调用里至少 10 ms 的 syscall 都是 open/flock。这里没有竞争客户端，不能把 flock 耗时直接解释为锁竞争。这个跟踪结果只给该次调用定位，不能证明此前未跟踪的 258–278 ms 样本原因相同，也没有识别文件系统内部／网络原因。

每版日志始终字节不变，保护中的 2 KiB payload 可读，另有 40 次未确认尾部丢失／幸存回归通过。主对照 56 个 pack、syscall 对照 1 个、归因对照 4 个均完成 GC；私有数据和四机临时环境已清理。新增回归后最终构建的 native 哈希改变，因此重新进行了完整对照；首轮结果单独保留，未与最终样本合并。所有原始样本、构建身份、模型与证明范围见 [`20261023-catalog-buffer.json`](../verification/measurements/20261023-catalog-buffer.json)。这不约束 JSON frame、owner／闭包索引或进程总内存，也没有重测 100 GB／TB payload 带宽。

## GC 自动发现与交接故障（2026-10-23）

四机各创建独立原生 2 KiB publication。源客户端显式启用 GC，seal 并建立 source owner；目标客户端用默认自动发现，分新对象与已使用 catalog 两种状态。对其 `storage.log` 预置一次 stat NotFound 或 EIO。旧版 32 次均正常返回 retain，却没有建立 destination owner；释放 source 后实际文件被 GC 删除。新版改用 fresh open，160 次 stat 故障预置均未被消费，真实 retain、源释放后的 GC 保护和读取检查通过。

另 40 次对新版冷发现的 read-only open 注入 EIO：首次 retain 均报 `StorageUnavailable`，源 owner／payload 仍受保护；再次调用成功之后才释放 source，所有 40 次交接均安全。总计新版 200 次交接通过，最后显式释放 destination 才回收。旧版误报成功没有完成同样的工作，不能拿其更短耗时做性能基准。

新版首次 retain 调用耗时（ms；stat 标签表示预置故障未触发，open-error 行表示**失败返回**的耗时）：

| 场景 | 样本数 | p50 | p99／最大值 |
|---|---:|---:|---:|
| cold-stat-missing | 40 | 48.71 | 276.66 |
| cold-stat-error | 40 | 45.97 | 82.79 |
| warm-stat-missing | 40 | 40.02 | 82.35 |
| warm-stat-error | 40 | 33.95 | 95.54 |
| cold-open-error | 40 | 14.95 | 41.46 |

每格只有 40 个样本，nearest-rank p99 就是最大值；这些值不能作为发现超时、重试等待或环境延迟上界。276.66 ms 慢样本未跟踪，不能用其他轮次的 flock 跟踪替它归因。创建／seal／初始 retain 不计时；调用包含 Python/native 边界、发现、锁／fresh refresh、闭包读取和 owner append／sync。本轮没有重测 payload 带宽或 100 GB／TB I/O。

首次自动发现仍信任 fresh-open NotFound，且池模式需在参与者开始前确定；没有验证活跃 legacy 池的在线迁移。需要 GC 的客户端应显式设置 `online_gc=True`，以避免用首次缺失判断 legacy。Rust `Config::gc_enabled` 现在返回 `Result<bool>`；共享 atomic 标记与真实 I/O 的完整细化仍未证明。

232 个私有 pack 已回收，四机临时环境与数据已清理；没有在生产数据上注入故障。原始样本、两个 native 哈希、旧版失败回归、新版验证和独立环境反例见 [`20261023-catalog-discovery.json`](../verification/measurements/20261023-catalog-discovery.json)。这些注入不证明部署自然出现过错误 stat。

## 目录持久化与发布开销（2026-10-23）

四机各对两个二进制运行 25 个独立重试池和 25 个正常池；两台先旧版，两台先新版。每次原生 publication 的 payload 为 2 KiB。探针准备阶段先持久化池外父目录，准备和 GC 不计入发布耗时。

重试探针对私有 `pool/raw` 的实际 fsync 注入一次 EIO，要求调用方收到 StorageUnavailable；失败后该目录已存在。旧版 100 次重试都返回发布成功，却没有在创建 raw 后同步 pool。新版 100 次均完成实际祖先链。两版读取及后续所有权释放／GC 均通过。实测证明的是屏障缺失／补齐；没有实际崩溃丢失数据，数据丢失是 TLA+ 环境允许的后果。

实测路径从叶子到文件系统根共 13 个目录。新版重试有 14 次成功目录 fsync：完整 13 级链，加发布结束时的叶子屏障。旧版为五次，缺少所需 pool 屏障。更老的祖先已由准备阶段持久化，不能仅凭没有再次同步它们认定旧版错误。根据实测深度增加 Depth=12 的 TLC 配置（根是锚点）；这不是支持深度上限。

正常计时关闭注入和目录日志；shim 仍加载并直接转发 fsync。cold 是新 Store 的首次发布，包括 catalog 初始化；hot 是同一 writer 的下一次发布。每格合并四机共 100 个样本：

| 发布方式 | 旧版 p50 ms | 新版 p50 ms | 旧版 p99 ms | 新版 p99 ms |
|---|---:|---:|---:|---:|
| cold | 218.09 | 219.16 | 325.99 | 331.94 |
| hot | 128.08 | 128.12 | 224.05 | 245.63 |

这次 p50 基本相同，hot p99 在新版更高，不能声称尾延迟普遍改善或将差异全部归因于屏障。缓存和背景负载未控制。带日志的重试计时单独保留：每次目录同步都会往 JuiceFS 写跟踪日志，耗时含明显探针开销，不用于生产性能对照。分位数不是超时参数或环境延迟上界。

400 个实际 pack 均由 GC 回收，私有数据和远端环境已清理。未重跑 100 GB／TB 带宽，也未模拟存储服务掉电。新实现要求所有实际祖先（包括池外目录）都可打开同步，且别名／挂载／命名空间稳定。源码、精确二进制、逐机样本与形式化输入见 [`20261023-directory-durability.json`](../verification/measurements/20261023-directory-durability.json)。

## Publication 重试与 GC（2026-10-23）

[摘要](../verification/measurements/20261023-publication-retry.json)与[公开 API 探针](../verification/storage/probes/publication_retry_probe.py)对照上一版目录持久性修复 wheel 和此次 publication hold 修复。四台机器分别使用 `/shared/straw-test` 下的独立私有池，两台先旧后新、两台先新后旧，源 payload 为 2 KiB。探针在内部依赖 hold 已持久化之后的 `after_data_sync` callback 注入 EIO；没有让 JuiceFS 崩溃，也不认证断电持久性。

对 `publish`、`record_set` 分别用同 ID 删除或替换依赖重试。原 source 释放后，第一次 GC 保留依赖；旧二进制的冲突重试移除保护，随后 **32/32 次 GC 实际删除原 pack**。这些依赖属于尚未成功返回的不确定目标发布，不是观测到 JuiceFS 自发丢数据。

修复版 **160/160 次**保住原依赖，完成正确重试、读取结果和源记录，释放被拒绝重试的额外根，并在最终 publication 释放后回收全部 pack。四种 API／冲突组合各有 40 个样本；表中为毫秒 p50／nearest-rank p99。n=40 时 p99 就是样本最大值，不能充当环境时间上界。

| API／依赖变化 | 冲突重试 p50／p99（ms） | 正确重试 p50／p99（ms） |
|---|---:|---:|
| `publish_remove` | 15.04 / 34.98 | 100.74 / 139.50 |
| `publish_replace` | 40.02 / 85.80 | 92.93 / 142.33 |
| `record_set_remove` | 18.04 / 63.27 | 143.17 / 263.68 |
| `record_set_replace` | 34.30 / 68.44 | 154.30 / 259.93 |

计时包含 catalog 和屏障工作，缓存与背景负载未控制。旧版冲突重试时延另行存档（每组 n=8），但它恢复失败，不能作为成功重试的性能基线。本次小 payload 实验不提供带宽、真实 100 GB／TB 或普遍延迟结论。模型中的 pack 数只是有限搜索配置，不是支持的依赖数量上限；Rust 并集证明不设这样的基数上限。

两个版本合计由 GC 回收 **528 个 pack**；旧版另留下 **48 个私有 pack**（未解决的目标输出，以及替换重试保留的额外根）。所有远端命令返回后才由 supervisor 手动清理这些 fixture，未计入 GC 成功。四机均确认测试数据与环境已删除。逐例事件／时延、两个 wheel 与 native 哈希位于 `/shared/straw-experiments`；完整形式化输入、反例、日志和源码哈希在相邻的 `publication-retry-verification`。

## 独立文件发布完成与 GC（2026-10-23）

[摘要](../verification/measurements/20261023-publication-finalization.json)、[API 探针](../verification/storage/probes/publication_finalize_probe.py)及[原生大记录探针](../verification/storage/probes/standalone_extent_probe.rs)覆盖 `segment_target_bytes=0` 的原始写入、record_set 和 publish。旧 wheel 成功返回却未在 catalog 封存 raw／record_set 文件：48 个正常案例中有 32 个在显式 seal、release、close 和重新打开后仍无法回收；16 个 publish 对照均回收。新版 240 个正常案例、48 个封存不确定案例全部回收。四机各用独立私有池，两台先旧后新、两台先新后旧；每种 API／大小组合旧版 n=8、新版 n=40。

故障案例在 publication seal 或 catalog sync 前／后通过 callback 返回 EIO，再 GC、同身份重试、校验小记录并释放。GC 均未删除受保护文件，重试未产生重复文件。catalog sync 前报错可留下完整 frame，由下一次 refresh sync 确认持久；这些注入不代表部署自然发生了 JuiceFS 故障，也没有验证服务崩溃／断电。

新版时延单位为毫秒，列出 p50／nearest-rank p99；n=40 时 p99 是样本最大值。record_set 的源记录在计时前准备，计时只包括 manifest 发布；另两种 API 写入表中 payload。GC 计时不含显式 release，覆盖完整 collect 调用，包括名称删除和 receipt。

| API | 源 payload | 发布 p50／p99（ms） | GC p99（ms） |
|---|---:|---:|---:|
| `write_records` | 2 KiB | 162.01 / 269.29 | 98.53 |
| `write_records` | 8 MiB | 225.28 / 379.13 | 93.86 |
| `record_set` | 2 KiB | 251.66 / 359.71 | 119.31 |
| `record_set` | 8 MiB | 251.65 / 401.25 | 98.56 |
| `publish` | 2 KiB | 185.87 / 285.71 | 83.18 |
| `publish` | 8 MiB | 260.33 / 352.80 | 105.91 |

旧版时延另有归档，但漏封存和空操作 GC 没完成相同工作，不能直接当作性能提升基线。缓存／背景负载未控制；四机运行与下面的大记录探针重叠。重复字节的小 payload 和短样本不提供冷缓存、对象存储带宽或延迟上界。

独立原生探针流式写入一条 **100,000,000,000 字节 payload**，真实文件长度 **100,000,000,585 字节**；使用重复的 8 MiB 随机 pattern，顺序写入，没有构造 sparse 文件。含哈希、数据同步、staging 和封存的发布耗时 **283.616 秒，逻辑 payload 带宽 336.26 MiB/s**。同机重新打开、扫描完整 extent 校验 checksum 耗时 **111.828 秒，852.80 MiB/s**，未驱逐缓存；进程峰值 RSS **10,848 KiB**。这是单次观测，不能算 p99、冷读结果或物理设备 SLA。返回完整 payload 的读取 API 仍有独立内存需求；本探针使用流式写入和仅 checksum 校验。

文件已在 catalog 封存，staging 仍保护它。显式 release 耗时 53.67 ms；之后 GC 在 **5437.42 ms** 内回收恰好一个文件，报告 100,000,000,585 字节。这个时间衡量 unlink、父目录屏障和持久 receipt，不代表异步后端对象回收已完成。大测试之前还跑了 2 KiB 原生 smoke。没有实际写 TB 文件；单次工作负载仍限制为 100 GB 十进制，完成控制器本身不依赖文件大小。

四机由 GC 回收 400 个文件，另有旧版遗留的 48 个 fixture 文件在远端工作返回后手动删除，单独计数。大文件由 GC 回收；全部共享私有池和远端环境已清理。工件位于 `/shared/straw-experiments`、`publication-finalization-large`、`publication-finalization-verification`。历史应用的 open 项不自动封存，它们可能仍属于不确定或活跃 writer。

## Pending 文件与同步错误（2026-10-23）

[摘要](../verification/measurements/20261023-pending-publication.json)、[API 探针](../verification/storage/probes/pending_publication_probe.py)及[限定路径的 fsync shim](../verification/storage/probes/publication_sync_fault.c)对照上一版独立文件完成修复 wheel 和此次 pending 描述修复。四机使用独立私有 JuiceFS 池，两台先旧后新、两台先新后旧。write_records、record_set、publish 均测试 2 KiB／8 MiB 源记录和三种故障：

- `before`：消费 arm 文件，跳过一次真实 payload fsync，返回 EIO。
- `after`：真实 payload fsync 成功，但 shim 向 straw 返回 EIO。
- `callback`：straw 收到 fsync 成功之后，after_data_sync 再报 EIO。

仅拦截指定私有 payload 路径；fsync 跟踪与 arm 文件位于本地 /tmp，不在被测池。每条跟踪区分是否真正调用和实际结果。这些注入不表示 JuiceFS 服务崩溃、真实断电或自然故障发生率。

旧版 **72/72 次**重写完整 extent，并留下原 partial／catalog-open 项。新版 **360/360 次**复用原 UUID／文件、校验字节、返回可读记录并回收全部已释放文件；重试没有 payload-chunk 写入。未知同步（before／after）对同一文件补做一次屏障；已确认同步后 callback 失败则无需再次同步文件，但仍做 checksum 校验和目录持久化。Packed 对照和损坏拒绝另由本地回归覆盖，不属于这次仅测 standalone 的四机运行。

新版 write_records 重试时延（ms；每行 n=20，p99 为样本最大值）：

| Payload | 故障 | p50 | p99／最大值 | 重试文件同步次数 |
|---|---|---:|---:|---:|
| 2 KiB | `before` | 71.16 | 148.88 | 1 |
| 2 KiB | `after` | 70.40 | 147.36 | 1 |
| 2 KiB | `callback` | 76.96 | 134.48 | 0 |
| 8 MiB | `before` | 94.45 | 147.60 | 1 |
| 8 MiB | `after` | 99.30 | 171.80 | 1 |
| 8 MiB | `callback` | 99.50 | 142.67 | 0 |

完整报告保留全部 API／大小／故障组合（旧版每组 n=4、新版 n=20），包括 publish 和 record_set。record_set 的源数据在计时前准备，被重试 extent 是小 manifest，不是再写源 payload。计时包含输入哈希、完整重试检查、所有权／catalog 工作、屏障和跟踪开销；缓存／背景负载未控制，因此不是仅文件系统时延，也不是延迟上界。

旧案例合计重写 **201,445,104 个逻辑 extent 字节**，并在 72 个原 partial 中留下相同字节数。计数依据完整文件长度和观察到的重试 payload 写入，不是块设备／后端物理流量。新版重试 payload chunk 数为零；节省的是文件重写，输入哈希和完整校验扫描仍保留。本轮没有重跑 100 GB／TB；上一轮 100 GB 证据对应其他二进制和范围，没有把大小或时延写成新的环境上界。

两版合计由 GC 回收 **576 个完成文件**；supervisor 在所有远端命令返回后另外删除 72 个旧版孤立 partial，没有计入 GC。共享私有池和远端环境均确认删除。精确 wheel、native／shim 哈希、跟踪和逐例结果位于 `/shared/straw-experiments`；形式化输入、日志、源码快照与包检查位于 `pending-publication-verification`。历史应用的孤立 partial／open 项不自动修复。

## 命名空间身份与 GC（2026-10-23）

[最终测量记录](../verification/measurements/20261023-path-identity.json) 保存两版 wheel／native 精确哈希。四机旧版 64 个案例中，48 个别名案例在 owner 仍存在时丢失数据，16 个规范路径对照保持可读；最终版 320 个案例均在受保护 GC、重新打开 catalog 后保持可读，释放 owner 后才回收。覆盖 file、directory、writer-directory 别名，2 KiB／8 MiB payload，独立／packed 文件。另有 32 个旧 writer／新 GC 案例在迁移前保留 staging，检查旧别名日志重放。跨客户端交接把发布、retain、GC、read 分配到不同机器：旧版四个别名案例全丢失，修复版八个均保护到 release。总计 440 个数据文件由 GC 回收；私有池和远端环境已清理，并在四机确认不存在。

重放成本对照让两版读取同一份 catalog 字节：一个真实、受 owner 保护的 2 KiB 文件，以及 0／128／4,096 条离线构造的已完成 receipt 历史，对应已不存在路径。这不是历史载荷写入／删除吞吐量测量。每行每版合并 20 次观察；cold 指新的 catalog 对象，并未清除文件系统缓存。下表单位毫秒，数值为 p50／p99：

| 已回收历史路径数 | 旧版 cold | 最终版 cold | 旧版 warm | 最终版 warm |
|---|---:|---:|---:|---:|
| 0 | 30.83／97.72 | 19.13／39.16 | 12.85／29.95 | 17.21／47.12 |
| 128 | 22.32／48.61 | 25.31／114.01 | 14.73／23.79 | 15.42／32.83 |
| 4,096 | 26.23／75.41 | 106.15／137.29 | 13.08／26.79 | 15.05／58.88 |

前缀缓存避免反复解析共有祖先，但独立名称查询仍增加冷重放成本。未控制后台负载、未驱逐缓存；该样本量的 nearest-rank p99 就是最大值。单工作负载旧版四次、最终版二十次，记录另含规范路径的发布／retain／read／GC 时间。旧版别名误删／读取失败执行的工作不同，不能拿来称作性能优势。这些值均不构成硬时延上界。本轮不替代此前不同二进制的 100 GB 带宽测量；2 TiB 稀疏回归也不代表实际 TB I/O。

部署需满足稳定别名／前缀绑定和真实解析（`ENV-NAMESPACE`）；缓存身份不会检测 retarget。Bind mount 别名／inode 等价和外部任意命名空间修改不在当前策略内。依赖别名所有权前，所有 collector 都需要升级。

## Catalog 路径编码与重放查询（2026-10-23）

[测量记录](../verification/measurements/20261023-catalog-encoding.json)对照前一版已修复别名误删的二进制与新增 `path_encoding=1` 的版本；两版都具备别名保护。四机每版各 64 个正常／别名案例、双向共 64 个旧 writer／新 reader 及新 writer／旧 reader 案例、八个跨客户端混合版本交接均通过。总计 224 个数据文件由 GC 回收，私有池及远端环境均清理并逐机确认。

每个历史夹具只有一个真实、受保护的 2 KiB 文件，另加 0／128／4,096 个已完成 receipt 的离线历史项；它不是历史 payload 写入／删除吞吐量测量。两版读取每个夹具相同的日志字节；legacy 事件无标记，resolved 事件包含真实的版本 1 标记。半数主机先测旧版，半数先测新版。下表每组汇总 20 次未插桩观察，单位毫秒，数值为 p50 / p99：

| 编码与历史项数 | 旧版 cold | 新版 cold | 旧版 warm | 新版 warm |
|---|---:|---:|---:|---:|
| legacy-0 | 28.19 / 91.57 | 35.39 / 67.09 | 22.43 / 62.86 | 15.04 / 43.75 |
| legacy-128 | 22.76 / 45.89 | 21.17 / 53.40 | 11.44 / 38.45 | 12.22 / 61.21 |
| legacy-4096 | 92.26 / 184.26 | 83.62 / 145.65 | 21.68 / 79.10 | 18.44 / 47.29 |
| resolved-0 | 25.12 / 59.40 | 23.00 / 103.49 | 15.66 / 61.49 | 15.58 / 63.76 |
| resolved-128 | 25.33 / 141.54 | 24.49 / 71.64 | 14.44 / 37.74 | 16.63 / 66.44 |
| resolved-4096 | 98.93 / 217.10 | 30.13 / 54.11 | 18.61 / 79.49 | 13.54 / 116.82 |

另行插桩的每机一次冷读取中，4,096 条 resolved 历史的 `readlink` API 调用从 4,101 降为 0；legacy 历史两版仍各 4,101。0／128 项的计数分别为 4／133，只有新版 resolved 降为 0；所有热调用计数为 0。计数只覆盖拦截到的 `readlink/readlinkat`，并非所有文件系统调用、内核路径查询或 metadata 服务 RPC。插桩样本不混入上表时延。

Cold 指新 catalog 对象，未清除文件系统缓存，也未控制后台负载；n=20 时 nearest-rank p99 就是最大值。尤其 warm 尾部仍有波动，不能据此宣称所有延迟都改善。旧日志不重写，旧历史解析成本仍存在；新增字段、解析版本和规范路径是协议选择，测量不提供新的时间／重试硬界限。本轮使用 2 KiB／8 MiB 数据和既有稀疏大偏移回归，没有重跑 100 GB 或进行物理 TB 传输。

紧凑 JSON 标记每个事件增加 18 字节。实测 4,096 项 legacy／resolved 夹具分别占 910,802／984,584 字节（加上真实发布历史，共 4,099 个事件）。两版比较时读取的是同一份 resolved 夹具，耗时降低并非来自减少日志字节。

## 未完成写入与清理重试（2026-10-23）

[精确二进制记录](../verification/measurements/20261023-incomplete-publication.json)对照前一版与新的未完成写入清理版本。四机每版各 64 个案例：2 KiB／8 MiB、standalone／packed，分别在 header、首个 payload chunk、record 之后或 footer 之前中断，每个案例连续失败两次。正常 GC 后旧版仍留下 64 个 partial、167,835,504 字节；新版没有未发布残留。Packed 旧版的死尾部在整个 pack 最终回收时消失，但重试 offset 已跳过这些字节；新版在重试前即恢复原前缀，保留旧记录可读。

在 8 MiB 记录首个 4 MiB chunk 后失败，两次共留下 8,389,000 字节（含 framing）；新版为 0。下表只取这个失败阶段，两次失败合并 n=8、重试 n=4，单位毫秒，均为 p50 / p99。失败时间含发布准备和失败后的清理，并非单个 truncate/fsync 延迟。

| payload / 模式 | 旧版失败返回 | 新版失败及清理 | 旧版成功重试 | 新版成功重试 |
|---|---:|---:|---:|---:|
| 2 KiB / standalone | 69.00 / 103.46 | 117.51 / 173.32 | 186.65 / 218.31 | 159.11 / 176.46 |
| 2 KiB / packed | 34.53 / 80.52 | 63.72 / 73.44 | 94.51 / 196.89 | 61.00 / 186.76 |
| 8 MiB / standalone | 110.14 / 198.95 | 156.84 / 279.83 | 208.69 / 295.06 | 158.21 / 300.02 |
| 8 MiB / packed | 109.85 / 117.42 | 122.14 / 272.34 | 162.61 / 231.02 | 133.93 / 220.80 |

清理需要额外屏障，失败返回时间可能增加；不能把旧版漏做工作的快慢当作同等语义性能对照。样本少，nearest-rank p99 就是最大值，未清缓存／控制后台负载，也不是带宽测量。各 host 独立执行本地 writer 重试，使用同一共享文件系统；此轮没有声称完成跨主机 writer 恢复。

另做 32 个真实 syscall 注入案例：truncate、unlink、文件 fsync、父目录 fsync，各在真实调用前或成功后返回 EIO。均保留可重试清理状态、保留旧记录，恢复后完成发布／GC。注入轨迹单独记录，未混入上表，也不表示自然发生了这些错误或真实断电。正常 GC 共回收 240 个文件；旧版遗留的 64 个私有 partial 在记录后由实验驱动手动清理，未算成 GC 成功。全部私有池及四机临时环境已确认移除。

本轮实际 payload 为 2 KiB／8 MiB；另有本地 2 TiB 附近的稀疏偏移回归。没有新增物理 TB 传输或对最终二进制重跑 100 GB 带宽。精确起点、长度保存及屏障顺序是协议／实现义务；观测耗时不进入安全性前提。

## GC 缺失父目录与 100 GB 复核（2026-10-23）

[精确二进制记录](../verification/measurements/20261023-gc-parent-repair.json)比较前一版与父目录修复版。每版四机各 120 个案例，包含真实 2 KiB／8 MiB 待删记录及始终保留到核对后的另一个 owner。已有父目录对照都通过；旧版 80 个缺失一／二层目录的案例连续失败三次，需要人工补目录后才能完成 GC。新版无需人工修复。GC 报错前可以已 unlink 其他文件，因此记录的实际删除数与成功返回的 reclaimed_files 分开。

每个大小／父目录组汇总 n=20，单位毫秒，p50 / p99；缺失层数为收集开始时实际缺失的目录数。**旧版缺失案例的值是报错耗时，新版是成功完成耗时，不能作为等价工作量加速比较。**

| 真实待删 payload | 缺失层数 | 旧版首次尝试 | 新版首次尝试 |
|---|---:|---:|---:|
| 2 KiB | 0 | 60.58 / 125.72 | 63.38 / 101.73 |
| 2 KiB | 1 | 48.72 / 83.65 | 73.20 / 117.09 |
| 2 KiB | 2 | 44.87 / 124.51 | 74.37 / 123.88 |
| 8 MiB | 0 | 56.90 / 117.95 | 69.20 / 197.12 |
| 8 MiB | 1 | 44.25 / 72.18 | 65.36 / 103.55 |
| 8 MiB | 2 | 42.89 / 71.28 | 72.96 / 99.14 |

补目录路径观察到 12 个成功目录屏障（含根），据此增加 Depth=11 的 TLA+ 夹具；正常父目录路径没有增加创建／祖先同步。Nearest-rank p99 在 n=20 时就是最大值；没有控制缓存和后台负载，100 GB 实验也与四机实验尾部重叠。已有父目录尾延迟有波动，不能据此给出性能改善或 SLA。

另有 24 个 syscall 注入案例：目录 open 的虚假 ENOENT／EIO、补目录时 fsync 的调用前／成功后 EIO，以及 mkdir 的调用前／成功后 EIO。成功 mkdir 后的 EIO 被 create_dir_all 的目录存在检查协调为成功，后续仍执行屏障；其余错误保留重试义务。全部样本保持受保护记录可读、完成后新客户端不再重复 payload GC I/O。包括故障调用的已发生副作用，共观察到 GC 调用删除 528 个真实数据文件；旧版仅人工补目录，未人工删除这些数据文件。私有池及远端环境均已清理。

同一批 Rust 核心源码另编译原生探针，在 `node-1` 实际发布一个 **100,000,000,000 字节**记录，文件为 **100,000,000,585 字节**。发布（含准备／checksum／持久化／staging）281.197 秒，即 **339.15 MiB/s**；完整 extent 认证扫描 110.960 秒，即 **859.47 MiB/s**。峰值 RSS 10.69 MiB，释放后 GC 5.301 秒，回收一个文件并确认路径消失、owner 为零，实验池已清理。

这是单次原生核心执行，使用重复的 8 MiB 随机模式、同机读取、不清缓存；不是 Python FFI 吞吐量、p99 或后端物理设备／trash 回收完成保证。探针与 wheel 的二进制哈希分别保存，并检查核心源码一致。没有写入实际 TB 文件；2 TiB 附近仍由稀疏偏移及整数证明覆盖。

## Publication 准备失败对照（2026-10-23）

[精确二进制、原始样本与证明范围](../verification/measurements/20261023-publication-preparation.json)包含四机旧／新 wheel 各 320 例，两台先旧、两台先新；另有 24 例实际 open／seek 前／后 EIO 注入。失败案例各登记报错两次，再重试真实 2 KiB／8 MiB 记录。旧版正常 close／GC 后累计留下 480 个 open reservation，新版为零；664 个成功写入的数据文件均由正常 GC 删除。旧版仅 catalog 残留随私有 fixture 清理，明确不算 GC 退役；没有手工删除 payload 文件。

正常发布时延，毫秒 p50／nearest-rank p99，每行／二进制合并 n=20，p99 即样本最大值：

| 模式 | Payload | 旧版 | 新版 |
|---|---:|---:|---:|
| standalone | 2 KiB | 138.19 / 878.15 | 147.14 / 211.54 |
| standalone | 8 MiB | 269.55 / 1004.50 | 282.05 / 1041.42 |
| packed | 2 KiB | 116.23 / 181.70 | 104.33 / 166.96 |
| packed | 8 MiB | 188.37 / 254.76 | 163.03 / 413.57 |

失败时延包含不同工作：新版删除／同步／封存后才返回原错误，旧版遗失 reservation；后续发布重试的目录／catalog 初态也不同，因此不是等工作量失败路径的速度比较。正常请求尾延迟有升有降，未建立普遍性能改善。缓存／背景流量未受控，且与 100 GB 实验重叠，分位数不是硬上界。

解析后的父目录深度为 11 条边（含根 12 级），对应已有 `directory_gc_repair_measured_depth` TLC 配置；这是实测参数，不是支持深度上限，也不是组合证明。原始报告的 hook 次数含打开缺失父目录前的尝试，不能直接视为实际 fsync 次数。

当前 Rust 核心实际发布 **100,000,000,000 字节 payload**，耗时 284.076 秒（**335.71 MiB/s**）；完整 extent 认证 105.693 秒（**902.30 MiB/s**）。峰值 RSS 10.68 MiB；释放后 GC 名称删除／receipt 为 5.575 秒。文件为 100,000,000,585 字节，私有池已清理。仅一次原生核心样本，重复随机 8 MiB 模式、同机读取、不清缓存；不能推出 Python／FFI 或冷缓存物理带宽、p99、TB 实际 I/O 或后端对象回收完成。

## Publication 交接失败与 100 GB 重试（2026-10-23）

[二进制、原始样本及验证记录](../verification/measurements/20261023-publication-handoff.json)包含四机旧／新版各 400 例：write_records、publish、record_set，standalone／packed，2 KiB／8 MiB 输入，在 staging 和后两种 API 的末尾 hold release 丢失响应。注入发生在真实 catalog sync 成功之后，是 callback 模拟报错，不是真实 syscall EIO／断电。record_set 只写引用这些 payload 的小 manifest。

旧版允许另一请求覆盖原缓存，重试时再写一份；新版提前拒绝其他请求，原请求重试没有 payload 写入 callback。释放和 GC 后，旧版留下 400 个 owner、120 个 open reservation，以及 640 个文件／2,350,605,360 字节（包括依赖）；新版全部为零。旧版正常 GC 删除 960 个文件，新版 1,400 个。旧版残留另由实验 fixture 删除，不算 GC 成功；私有池和四机临时环境已确认清理。最初汇总断言漏算依赖文件，800 个案例本身均已通过；修正逐场景计数后重新汇总，未重跑或筛选样本，原失败报告保留。

write_records 原请求重试时延，毫秒 p50／p99，每组 n=20（四机各 5 次）；nearest-rank p99 即样本最大值。**旧版重写，新版复用，工作量不同**：

| 模式 | Payload | 旧版 | 新版 |
|---|---:|---:|---:|
| standalone | 2 KiB | 109.25 / 118.34 | 38.04 / 47.15 |
| standalone | 8 MiB | 166.77 / 239.81 | 44.57 / 67.25 |
| packed | 2 KiB | 63.33 / 80.44 | 11.09 / 20.73 |
| packed | 8 MiB | 117.46 / 142.06 | 18.65 / 26.73 |

缓存和背景负载未受控，实验与 100 GB 任务重叠；不提供延迟上界，其他 API／阶段样本保留在 JSON。模型无需从这些分位数获得重试截止时间。

另在 node-1 用相同 Rust 核心实际写一个 **100,000,000,000 字节**记录（文件 100,000,000,582 字节），staging 生效后报错。首次尝试耗时 281.320 秒，按 payload 折算 339.00 MiB/s；**该尝试以报错结束，不能叫成功发布吞吐量**。另一 ID 在 0.001 毫秒被拒绝，未遍历 payload；原请求重试 69.253 秒，重新哈希输入一次，payload 写入 callback 为零，文件身份／长度／mtime 不变。包含诊断 GC 和拒绝请求的完整流程 350.585 秒（272.02 MiB/s），不是单个 API 的耗时。

完整 extent 认证读取 105.522 秒（903.77 MiB/s），峰值 RSS 18.71 MiB；释放后 GC 5.552 秒，删除一个文件、owner／open 为零，实验池已清理。原生单次样本、重复 8 MiB 随机模式、同机读取、不清缓存；不代表 Python FFI／冷缓存物理带宽、p99、TB 实际 I/O 或后端对象回收完成。

## Staging 消费后 GC 与重试（2026-10-23）

[精确二进制与测量记录](../verification/measurements/20261023-staging-consumption.json)包含四机旧／新各 336 例：320 个 publish／record_set 场景，另有 16 个 write_records 控制。每次在真实 catalog sync 成功后模拟释放响应丢失，再由 peer GC，使用 fresh Config 重放后重试。始终另有 reader 保留一个无关 publication；重试和 GC 不得破坏它。旧版所有 320 个 manifest 场景重试均因文件已删除而失败，raw 控制通过；新版 336 例全部成功，重试无 manifest 读取／catalog append hook。

载荷为真实 2 KiB／8 MiB，每图 1 或 12 层；publish 和 record_set 的可达子图实测分别为 2／24 个节点、1／23 条直接边，对应 TLC 的 Depth=1／12（深度 12 搜索 98,296 个状态）。直接 raw 根（包括 record_set 内部 raw manifest 的消费）不再新增图缓存节点；独立 reader 对照的图另行计数。搜索深度不是 API 上限，样本分位数也不是环境硬界限。

以下为 **12 层、每层 8 MiB** 的重试毫秒 p50／p99，及第一次释放增加的 catalog 字节数中位数。每组 n=20（四机各 5 次），nearest-rank p99 为最大值；**旧版返回错误，新版成功，不能视为等价工作量提速**：

| API | 模式 | 旧版重试 | 新版重试 | Catalog 字节：旧／新 |
|---|---|---:|---:|---:|
| publish | standalone | 186.22 / 238.73 | 20.45 / 57.59 | 1103 / 4659 |
| publish | packed | 192.08 / 265.08 | 22.45 / 56.38 | 1103 / 4659 |
| record_set | standalone | 177.94 / 216.67 | 23.88 / 61.20 | 2051 / 5607 |
| record_set | packed | 181.80 / 237.62 | 25.15 / 77.68 | 2051 / 5607 |

直接边随节点／边线性记录，不保存每个节点的传递闭包；这不意味着整个 catalog 的内存或历史大小有界。以上字节差是序列化 log 增长，不是 heap 测量。全部组的 setup、首次报错、重试和 metadata 统计保留在 JSON。缓存及背景流量未受控，最终运行与另行归档的中间版本实验重叠；旧版没有新增读 hook，旧版 hook 数为零不能解释为零 I/O。

旧／新正常 GC 分别删除 2112／2112 个数据文件；没有手动删除遗留 payload，私有池与四机临时环境已清理。单图最大真实 payload 为 96 MiB，本轮没有对当前新二进制重跑 100 GB 或写实际 TB 文件。完整回归仍含稀疏 TB 偏移检查；这些结果不提供带宽 SLA、冷缓存保证或物理回收完成证明。

另用最终 wheel 在 512 KiB 主线程栈下执行 2,048 层 inline manifest 链。读取 2049 个 manifest 后，释放 frame 增加 469,470 字节；首次报错耗时 0.672 秒。Peer GC 删除两个文件后，fresh replay／重试 21.11 毫秒完成，不读 manifest、不追加 catalog。峰值 RSS 47.64 MiB；这是单次含 Python 的进程观测，不是内存上界。叶 payload 仅 1 KiB，其余是真实 metadata；私有池已清理。

## 准备阶段依赖 owner 清理（2026-10-23）

[精确二进制记录](../verification/measurements/20261023-provisional-holds.json)覆盖四机旧／新各 480 例：publish／record_set、standalone／packed、2 KiB／8 MiB 记录，以及输入校验、hold 同步后响应丢失、payload callback 报错三个切点。每组每机五次，合并 n=20，最近秩 p99 等于样本最大值。不同机器交替版本顺序；与本地回归和验证工作并发，未驱逐缓存。

两版都先释放源 staging 并运行 GC，确认失败准备的依赖仍可完整读取，之后再 close。旧版 close 并 GC 后，仍留下 480 个内部 owner／480 个数据文件；修复版全部清理。独立 reader 始终可读，释放它后才完成最终 GC。普通 GC 分别删除 800／1280 个旧／新数据文件；旧版遗留的 3,021,279,300 字节随后只在新建的私有夹具目录内删除，未冒充正常 GC。GC 文件数仅计 GC 返回值，不包含存活 writer 自动回滚的未发布文件。新版无人工 payload 清理。远程环境和所有私有池已清理。

下面是 8 MiB 记录在写入切点报错后的 `close()` 耗时，单位毫秒。新版 close 包含同步释放内部 owner 的额外工作，旧版直接遗留 owner，因此不是等工作量吞吐对比。

| API | 模式 | 旧版 p50 / p99 | 新版 p50 / p99 |
|---|---|---|---|
| publish | standalone | 0.00 / 0.00 | 33.34 / 80.24 |
| publish | packed | 34.63 / 77.85 | 72.03 / 135.47 |
| record_set | standalone | 0.00 / 0.01 | 44.89 / 96.58 |
| record_set | packed | 31.76 / 72.23 | 69.84 / 159.79 |

测量用于检查错误响应和回收边界，不把 p99 写成时限／公平性假设。完整记录也包含校验／hold 切点、close 和 GC 耗时。没有本版 100 GB 或实际 TB 传输、冷缓存带宽或断电持久性结论；此前大文件证据属于各自的精确二进制。

## 当前 Rust 核心的 100 GB 四机工作流（2026-10-23）

[完整记录与精确二进制](../verification/measurements/20261023-shared-record-workflow.json)使用与上一轮 693 项 Python 回归相同的生产 Rust 源码，另行编译原生探针；不是 Python／FFI 吞吐测量。先以 33,556,480 字节预跑，随后在 node-1 成功发布单个 **100,000,000,000 字节** tensor record，源文件 100,000,050,713 字节。发布耗时 **281.063 秒，339.31 MiB/s**，包括输入哈希、framing、写入和同步；随机 8 MiB 重复图案及 checksum 元数据准备不计入。Writer 峰值 RSS 11.04 MiB，为本次原生进程观测。

目标 record_set 在 footer 前报错，但已建立的准备 hold 保留源。释放源 staging、运行 GC 后，四机读取互不重叠的分片；共 373 个分片，认证／返回字节都恰为 100 GB，无重叠或遗漏。总读耗时 **41.071 秒，2322.01 MiB/s**，包含 supervisor 发起 SSH 的时间；没有把各机速率直接相加。

| 机器 | 已校验／返回字节 | 原生读取秒数 | MiB/s | 峰值 RSS MiB |
|---|---:|---:|---:|---:|
| node-1 | 25,106,507,776 | 40.567 | 590.22 | 260.86 |
| node-2 | 24,964,497,408 | 39.982 | 595.46 | 260.93 |
| node-3 | 24,964,497,408 | 40.145 | 593.05 | 260.87 |
| node-4 | 24,964,497,408 | 40.091 | 593.85 | 260.89 |

每次请求最多返回 256 MiB，另有有界认证 scratch；RSS 包含这份返回 buffer，不应与 8 MiB scratch 设置混为一谈。372 个完整 256 MiB 分片的耗时样本 p50／p95／p99 为 **417.86／457.87／525.69 ms**，排除了最后一个短分片。这些样本同属一个文件、并发机器和同一次运行，彼此相关；不是整文件操作的独立 p99，更不是可见性延迟上界。未驱逐缓存，单次写入无法提供发布 p99 或带宽 SLA。

全部 reader 返回后，原交接重试 339.73 ms；另一错误准备的显式 close 清理 61.48 ms。消费引用在 catalog sync 后丢失响应，peer GC 正常删除源和 1,152 字节 manifest，共 100,000,051,865 字节，耗时 1.678 秒。新客户端在 manifest 已消失后重试释放，20.00 ms 完成，manifest 读取和 catalog append hook 均为零；最终 owner／open pack 都为零。私有数据与四机程序目录均已清理；这不证明 JuiceFS 后端物理空间已立即回收。

首次观察到 writer 等待 reader 时，Linux 累计 user／system CPU 为 207.70／38.87 秒。它还包含图案准备与控制操作，监测从发布后半段开始，不是精确的 API CPU 计时；不能仅据此判定 CPU 或文件系统瓶颈。原始计数单独归档。本轮未执行实际 TB 传输，也未在 100 GB 运行中注入 reader 错误或物理断电；故障恢复与内存／带宽的普遍界限仍未证明。

## 四机 writer callback 访问（2026-10-24）

[精确二进制与完整记录](../verification/measurements/20261024-writer-access.json)对比上一版 wheel 和非阻塞 writer 准入。每机覆盖 metrics／seal／close、callback 重入／另一依赖线程、2 KiB／8 MiB、standalone／packed；旧版每组合一次，共 96 例，新版每组合五次，共 480 例。外层写入在 footer 前暂停，此时真实 payload 和 index 已写入；既有已提交前缀必须保留。

旧版每个子进程都到达访问调用，在两秒窗口内仍存活但没有响应，然后由其 supervisor 显式终止并等待退出。有限观察本身不是无限死锁证明；独立 TLA+ 等待环提供模型层的活性反例。旧版由实验清理了 144 个私有数据文件、402,853,536 字节，不计为 GC 成功。

新版 480 例都返回 ResourceLimitExceeded，随后完成并校验原发布、再写一条记录、释放 owner 并关闭。正常 GC 删除 960 个文件，最终 owner／open／retained 都为零，无新版数据文件需要人工删除。共享私有目录和远程测试环境全部清理。

| 访问接口 | 调用方式 | 样本数 | p50 ms | p99 ms |
|---|---|---:|---:|---:|
| metrics | callback | 80 | 0.011 | 0.027 |
| metrics | thread | 80 | 0.014 | 0.027 |
| seal | callback | 80 | 0.012 | 0.030 |
| seal | thread | 80 | 0.014 | 0.031 |
| close | callback | 80 | 0.011 | 0.104 |
| close | thread | 80 | 0.013 | 0.052 |

计时对象是 Python 可见的 busy 异常响应，不是文件读写。每组混合两种大小、两种布局、四机与五次重复；n=80 的 nearest-rank p99 就是最大样本。重复、调度和后台负载并非独立／受控。两秒 watchdog 和这些微秒量级响应都没有写成环境的数值时限。本轮 binding 修改不产生新的 100 GB／TB、payload 带宽或断电结论；此前原生 100 GB 实测保留各自二进制身份。

## 四机队列提交后清理（2026-10-24）

[精确二进制与完整记录](../verification/measurements/20261024-commit-cleanup.json)覆盖 2 KiB／8 MiB、standalone／64 MiB pack target、释放前／部分 catalog frame 后／持久回执后报错，以及原实例重试／fresh recovery。四台机器各跑旧版每组合一次、新版三次，共旧版 96、新版 288 例。

旧版释放前与部分 frame 错误的 64 例留下 owner 和文件；回执已持久化的另 32 例正常 GC。旧版人工清理 64 个私有文件、268,579,360 字节，另有正常 GC 的 32 个文件、134,289,680 字节，两者分开计数。新版 288 例最终 owner／open 为零，正常 GC 删除 288 个文件、1,208,607,120 字节，无人工数据清理；每例先验证独立 reader 在队列 prune 后仍可读取，随后才释放该 owner。所有共享实验目录和远程环境已删除。

| 错误切点 | 重试方式 | 样本数 | p50 ms | p99 ms |
|---|---|---:|---:|---:|
| after_storage_catalog_part | live | 48 | 44.055 | 106.006 |
| after_storage_catalog_part | reopen | 48 | 92.596 | 142.292 |
| after_storage_catalog_sync | live | 48 | 15.459 | 64.744 |
| after_storage_catalog_sync | reopen | 48 | 62.907 | 120.300 |
| before_staging_consume | live | 48 | 36.381 | 119.436 |
| before_staging_consume | reopen | 48 | 85.228 | 344.026 |

每组 n=48 混合四机、两种大小、两种布局和三次重复，nearest-rank p99 等于最大样本。reopen 计时还包含 close／打开／epoch 提交，不能当成纯释放延迟。缓存与后台负载没有受控，样本并非独立。数据只说明这次错误恢复工作流，不是可见性上界，也不产生新的 100 GB／TB 吞吐量结论。探针复制到远端后仅做 Black 排版；归档保存实际执行版本、最终版本哈希及 AST 等价核对。

## 实际队列中的 100 GB 与历史恢复（2026-10-24）

[精确程序、源码和完整记录](../verification/measurements/20261024-queue-record-workflow.json)使用当前原生核心，先持久写入一个 record 和一个 manifest，再将同一引用交给四个任务。在接收已写入 WAL、释放 staging 前注入错误，fresh recovery 补做清理。随后生成真实历史、给四个输入 reader 发 lease、取消任务并再次恢复。此时 GC 仍保留 reader 输入；四台机器分别读取不重叠分片，supervisor 校验成功退出、分片身份与字节覆盖后逐个确认。前三次确认后的 GC 都删除零个文件，第四次才删除数据与 manifest。数据回收后继续恢复历史，manifest read／catalog append 计数都为零。

实际 payload **100,000,000,000 字节**，数据文件 100,000,050,717 字节。发布耗时 271.268 s，351.561 MiB/s；独立 manifest 创建 0.247 s。首次待清理恢复 0.091 s；128 条历史建立 14.341 s，取消后恢复 0.095 s。writer 就绪时峰值 RSS 11,444 KiB。

四 reader 共认证并返回完整 100 GB，373 个分片，聚合耗时 40.182 s、2373.393 MiB/s，包含 SSH 启动与 supervisor 处理。最终 GC 删除两个文件、100,000,051,877 字节，owner／open 为零。后续第一次恢复 0.082 s；没有人工数据删除，全部共享私有目录与远程程序已清理。

另外使用 2 KiB 数据建立不同数量的真实已接收／取消事务。下表 WAL／catalog 大小取 reader 确认前；恢复样本取数据 GC 后。每组依次在四台机器各新建 20 个 coordinator，前一个已退出才开始下一个。每次恢复新增一个 epoch，因此不是对完全静止日志的重复计时。

| 历史交接数 | 建立历史 s | WAL 字节 | Catalog 字节 | 恢复 p50 ms | 恢复 p99 ms | 峰值 RSS KiB |
|---|---:|---:|---:|---:|---:|---:|
| 0 | 0.000 | 7,166 | 3,850 | 54.261 | 99.166 | 3,484 |
| 128 | 15.499 | 184,920 | 35,338 | 63.585 | 114.786 | 6,380 |
| 1024 | 104.910 | 1,433,168 | 255,754 | 95.187 | 158.703 | 26,420 |

n=80 的 nearest-rank p99 就是最大样本。新实例不代表驱逐了存储缓存，四机／重复样本与后台负载也未受控。历史建立耗时测的是逐笔 submit/cancel，不是批量接收的容量；0／128／1024 是实验与模型配置，不是支持规模上限。元数据仍随历史增长，没有 compaction 或内存上界证明。

这是原生单条 100 GB 与队列输入读取流程；8 MiB 随机 pattern 重复，发布计时排除 pattern／chunk checksum metadata 的准备，包含实际数据 hashing／framing／sync。读返回缓冲区最大 256 MiB，全部请求字节经过认证。测量期间同机也运行形式化检查，没有 CPU 隔离；保留了进程与 cgroup 观察，但未据此归因性能瓶颈。没有 Python／FFI、物理 TB 传输或硬带宽 SLA 的新结论；此前稀疏 TB 偏移证据保持独立。

## 位置可区分的 100 GB 范围读取（2026-10-24）

[完整记录与精确二进制](../verification/measurements/20261024-positioned-range.json)使用全局 word 位置相关的数据，取代重复 8 MiB pattern。每个 checksum chunk 都有不同的摘要；四机 reader 在生产认证之后逐字节核对结果。独立 Python 实现核对 32 MiB 小负载的全部 chunk 摘要，以及每个远端请求的首尾字节。单次实际写入仍为 **100,000,000,000 字节**，没有扩大到物理 TB；此前稀疏 TB 偏移证据保持独立。

生成数据并准备 chunk checksum metadata 耗时 113.308 s，单列在发布计时之外。发布、关闭耗时 363.514 s，262.349 MiB/s，包含重新生成流、两遍内容处理、framing 和持久化。数据源以最多 1 MiB 的 callback 片段送入 writer；这是实验负载选择。writer 峰值 RSS 5,044 KiB，数据文件 100,000,050,725 字节。

四机完整读取 373 个 chunk，合计 100 GB，另外检查 96 个边界区间，其中 25 个为空区间。样本涵盖字边界、1 MiB buffer 边界、256 MiB chunk 边界、2 GiB／4 GiB、文件中部和尾部。非空小切片长 1–4099 字节；空切片的 payload checked 量为零，仍可能读取和认证 metadata。

| 指标 | 本次结果 |
|---|---:|
| 总返回字节（含小切片） | 100,000,094,695 |
| 总 payload 认证字节 | 120,953,550,848 |
| 四机完成时间，包含 SSH、额外切片及逐字节 oracle | 62.131 s |
| 100 GB / 上述完整工作流时间 | 1534.941 MiB/s |
| 完整 256 MiB 请求，原生调用 p99（n=372） | 575.895 ms |
| 非空小切片，原生调用 p99（n=71） | 552.673 ms |
| 四 reader 峰值 RSS KiB | 268376 / 268348 / 268204 / 268268 |

小切片仍需认证整个被触及的 chunk：一个 1 字节请求可以认证 256 MiB，跨 chunk 的 17 字节请求可以认证 512 MiB。这里的 checked 量是逻辑 payload 认证量，不是后端物理 I/O 计数。低 scratch 内存不等于低读取放大，优化需要保持完整认证，不能直接跳过未返回字节。

所有 reader 成功退出且覆盖／oracle 核对通过后，正常 GC 删除一个文件、100,000,050,725 字节，owner／open／retained 为零；没有人工数据删除。32 MiB 四机边界负载也通过并正常回收。实际生产代码未变，本轮未复现新的范围读取缺陷。

没有驱逐缓存、隔离后台负载或提供性能 SLA；writer 主机同时运行 TLC／Verus。上述 p99 是混合位置的样本分位数，小切片 n<100 时 nearest-rank p99 是最大值。新工作流包含位置数据生成、额外认证读取和独立内容核对，不能与先前重复 pattern 的纯读取耗时直接归因为存储速度变化。原生调用延迟单独计时，不含随后 oracle 的时间；整个工作流时间包含 oracle。未新增 Python／FFI、真实 TB 吞吐或完整 I/O 适配证明。

## 固定 4 MiB checksum 的 100 GB 验证（2026-10-24）

[完整记录](../verification/measurements/20261024-tensor-granularity.json)将新 tensor 的 checksum 粒度固定为 4 MiB，去掉原来的 512 个摘要预算；100 GB 使用 23,842 个摘要。Reader 继续按旧记录的编码粒度读取。实际 Rust 区间函数有 Verus 证明；Python producer 与完整 I/O 细化仍未证明。格式、元数据增长和跨策略同 ID 重试限制见 [FORMAT](FORMAT_zh.md#类型化张量)。

本轮使用实际 100,000,000,000 字节位置相关 payload，四机各检查返回的全部字节。完整读取仍为 373 个最多 256 MiB 的请求，另有与上一轮相同的 96 个小区间（25 个空区间）。新旧轮次的请求位置、长度完全一致；seed 和二进制不同，缓存及后台负载未受控，不能将延迟差异全部归因于粒度修改。TLC／Verus／集成测试在 writer 主机同时执行。

| 指标 | 上轮 256 MiB checksum | 本轮 4 MiB checksum |
|---|---:|---:|
| 小区间返回字节 | 94,695 | 94,695 |
| 小区间 payload 认证字节 | 20,953,550,848 | 330,158,080 |
| 非空小区间原生调用 p99，n=71 | 552.673 ms | 25.391 ms |
| 完整 256 MiB 原生调用 p99，n=372 | 575.895 ms | 533.180 ms |
| 100 GB / 完整四机工作流时间 | 1534.941 MiB/s | 1756.167 MiB/s |
| 数据文件字节数 | 100,000,050,725 | 100,003,195,567 |

同样的小区间，其逻辑 payload 认证量减少为原来的 1/63.465。这项字节计数可由区间和 chunk 大小确定；不是后端物理 I/O 计数。单字节请求最多认证 4 MiB，任意 N 字节非空请求最多认证 `N + 2 × (4 MiB - 1)` 字节；单次空请求为零 payload。metadata、重试及物理对象预取不在界限内。

元数据准备／生成耗时 111.553 s；发布与关闭耗时 354.223 s，269.230 MiB/s。writer 峰值 RSS 18,064 KiB，四个 reader 为 280796 / 280804 / 280724 / 280792 KiB。四机完成时间 54.304 s，包括 SSH、额外切片和逐字节 oracle；原生调用时延不包含随后 oracle。n=71 的 nearest-rank p99 是样本最大值，不是环境延迟上界或性能 SLA。

最终二进制还完成 32 MiB + 37 字节、8193 个摘要的四机负载，独立 Python 重算其全部摘要及所有请求的首尾字节。两个安装 wheel 的 Python producer 各实际发布 **2 GiB + 17 字节**，旧版产生 257 个 8 MiB 摘要，新版产生 513 个 4 MiB 摘要；切片、同策略重试和 GC 均通过。该 Python 负载使用重复字节，只检查真实 API 门槛，不作为物理带宽证据。100 GB 是原生流式源，没有分配 100 GB 的 Python tensor。

所有测量数据均在 reader 退出后通过正常 GC 回收，再删除各自私有元数据及远端探针；本轮 100 GB GC 删除一个文件、100,003,195,567 字节，owner／open／retained 为零。完整新验证为 761 个 Python、38 个原生测试，282 个 TLC 案例、116 项 Verus 检查和 96 个预期语义拒绝；模型和二进制证据分别归档。没有新增物理 TB、完整恢复细化或“全系统无 bug”的结论。

## Typed 引用提前拒绝的四机测量（2026-10-24）

[完整记录](../verification/measurements/20261024-typed-preflight.json)复现并修复两个边界：错误 shape 的大 tensor 在字段比较前分配／读取 payload；空切片直接成功。64 MiB 的错误引用在 reader 仅有额外 16 MiB 地址空间时，旧版先报分配 `ValueError`，新版直接报 typed 不匹配 `CorruptData`。新顺序保留 checksum 校验，并明确空切片仍可能读取 metadata。

四台机器分别运行旧、新安装 wheel，每个实例创建私有 4 KiB、4 MiB、64 MiB 记录，再由独立子进程读取。这是同一部署上四个客户端的对照，并非四机共享同一条记录的远端可见性试验。每个尺寸／版本／主机执行 50 次错误完整读取、100 次错误小切片、100 次错误空切片、100 次合法小切片，共 8,400 次调用。重复字节只用于 API／顺序检查，不作为物理带宽证据。

| 64 MiB 记录上的指标 | 旧版 | 新版 |
|---|---:|---:|
| 错误完整读取的进程 rchar 增量 p99，n=200 | 67,112,143 字节 | 3,277 字节 |
| 错误完整读取时延 p99，n=200 | 199.476 ms | 4.995 ms |
| 合法小切片时延 p99，n=400 | 17.483 ms | 35.519 ms |

旧版在三个尺寸上共接受 1,200 次错误空切片，新版全部拒绝。各新 reader 在拒绝错误大引用时只进行 metadata 工作。rchar 包含 metadata 和读取 `/proc/self/io` 的观测开销，不等于精确 payload 认证量或物理后端 I/O；worker RSS 是高水位观测，新版没有超过导入后的高水位不等于零分配。全部数据由正常 GC 删除，共 24 个文件、570,510,800 字节，owner／open／retained 均为零，然后移除私有目录和远端程序。

第一组试验与形式化／集成任务重叠，合法小切片的尾延迟变差，不能忽略这个结果。全量检查进程结束后，又在本机以**同一条 64 MiB 不可变记录、两个常驻 reader、串行 ABBA 批次**交错执行旧新版，每版 400 次合法切片。旧／新 p50 为 3.804／3.838 ms，p95 为 9.574／11.775 ms，p99 为 15.891／21.608 ms。这个补充控制减少了文件、进程启动和执行顺序差异，仍没有系统级负载隔离或冷缓存保证，不足以宣称所有合法请求绝无性能退化；没有把任一 p99 当成环境上界。

本轮未重跑 100 GB／物理 TB；上一轮 100 GB 吞吐保留原二进制身份，不套用到本轮。新增 Rust layout 匹配循环由 Verus 证明；Python shape 乘积、JSON／FFI、调用位置及完整 I/O／恢复细化仍是明确缺口。

## 合法小切片的尾延迟归因（2026-10-24）

[测量记录](../verification/measurements/20261024-read-latency.json)继续检查上述合法切片的尾延迟差异。旧、新 wheel 仍分别是固定 4 MiB 粒度版和 typed 提前校验版，本轮没有修改生产代码。每组共用一条不可变记录，两个常驻子进程按 ABBA 顺序交错读取最前面的 32 字节；每版每种模式 800 次。三个独立实验分别使用 JuiceFS 的 64 MiB／4 KiB 记录，以及本机 overlay 的 64 MiB 记录，共 14,400 次读取。

| 原始 API 路径，仅调用边界计时 | 旧版 p50 / p95 / p99，ms | 新版 p50 / p95 / p99，ms |
|---|---:|---:|
| JuiceFS，64 MiB 记录 | 4.337 / 12.211 / 20.748 | 4.225 / 11.502 / 18.113 |
| JuiceFS，4 KiB 记录 | 0.535 / 1.129 / 5.774 | 0.541 / 1.597 / 6.648 |
| 本机 overlay，64 MiB 记录 | 3.415 / 3.450 / 3.907 | 3.417 / 3.443 / 3.566 |

另外两种诊断模式分别包装 Python 阶段边界，以及在此基础上加入仅选择私有目录的 libc interposer。诊断计时不能与原始路径混为一组 p99。每次均核对返回内容，保存实际 native／Python 文件和 shim 哈希、墙钟／线程 CPU 时间及原始事件；没有注入延迟或错误。此前两组仅阶段计时的 6,400 次调用也另存原始脚本和哈希，不混入上表。

所有 libc 样本中，新旧版均有两次 `open64`、两次 close。64 MiB 记录每次认证 4,194,304 字节 payload，九次 read 合计返回 4,197,460 字节（含 metadata）；4 KiB 记录认证 4,096 字节，六次 read 合计返回 5,222 字节。代码对应 `Config::frame` 先调用 `inspect_once` 打开并检查索引，再由 `frame_with_index` 打开同一引用的文件、检查 envelope 并读取 payload。这个计数不是后端物理 I/O 或 RPC 计数。

JuiceFS 的 64 MiB 诊断样本中，旧／新各自最慢八次请求的总时间有 86.3%／78.1% 位于两次 `open64` 内；4 KiB 对应为 96.6%／95.7%。最慢的旧版请求共 70.694 ms，其中一次 open 为 65.655 ms，整次线程 CPU 为 3.554 ms。本机 overlay 上，两次 open 的合计 p99 为 0.011／0.010 ms。阶段样本里的 Python 编码 p50 从约 20.3 µs 增至 25.1 µs，不能解释这里几十毫秒的尾部。

结论是把这些慢样本定位到了打开文件的调用边界，**还没有定位到 JuiceFS 内部、网络或服务端**。墙钟减线程 CPU 包含调度和其他等待；interposer 只覆盖指定路径／描述符及列出的 libc 函数，不覆盖任意 syscall 或其他线程。新旧 p99 的方向在实验间变化，不能据此宣称 typed 校验导致固定尾延迟，也不能保证所有负载无退化。

可进一步审查同一次尝试中能否复用已检查的描述符，减少第二次打开；任何实现都仍需保留 fresh-open 重试、索引／envelope 绑定和 payload 认证。本轮没有实施该优化，也没有新增正确性 bug 的结论。`VisibleBy` 继续表示尝试次数，3.85 秒继续只表示等待策略总和，均不改为这些 p99。48 个相关读取 TLC 案例与 118 项 Verus 正向验证重跑；没有将未重跑的完整集成套件或 Verus 负例记为新执行。

三个新实验都在 reader 退出后正常 GC，删除三个文件、134,230,390 字节，owner／open／retained 为零，随后移除私有 metadata。重复数据、同一位置和未驱逐缓存限定了结论；本轮形式化任务在测量结束后执行，仍没有系统级负载隔离。没有新四机／100 GB／物理 TB 吞吐结果，也没有新的时间上界或带宽 SLA。

## 复用索引检查描述符：四机对照与 100 GB（2026-10-24）

[完整记录](../verification/measurements/20261024-single-open.json)将普通读取与 cold session 的索引检查描述符直接交给 frame 读取，避免第二次成功打开；warm session 继续打开新文件，session 仅缓存已认证索引。失败后关闭本次数据文件并在下一次尝试重新打开。新增实际 frame 定位算术的 Verus 证明，完整描述符／I/O 适配仍未整体证明。

四台机器各自创建 4 KiB／64 MiB 的私有记录，同一记录由旧版 typed-preflight wheel 和新版常驻进程按 ABBA 批次交错读取前 32 字节。每种尺寸、版本、计时模式、主机各 800 次，总计 38,400 次。下表只含原始 API 路径的调用边界计时，每个汇总组 n=3,200；Python 阶段包装和 libc 插桩另存，不混入该分位数。

| 记录尺寸 | 旧版 p50 / p95 / p99，ms | 新版 p50 / p95 / p99，ms |
|---|---:|---:|
| 4 KiB | 0.521 / 1.086 / 5.439 | 0.361 / 0.656 / 4.494 |
| 64 MiB | 3.775 / 8.930 / 17.015 | 3.602 / 7.982 / 14.977 |

单机原始路径 p99（每组 n=800）保留如下，不能用汇总结果掩盖差异：

| 主机 | 4 KiB 旧／新 p99，ms | 64 MiB 旧／新 p99，ms |
|---|---:|---:|
| client-1 | 4.327 / 2.910 | 21.175 / 14.191 |
| client-2 | 5.652 / 4.703 | 14.695 / 16.477 |
| client-3 | 4.812 / 5.007 | 17.434 / 13.751 |
| client-4 | 6.361 / 4.483 | 16.661 / 14.847 |

所有 libc 样本中新版 open／close 从 2／2 减到 1／1，statx 从 5 减到 3；4 KiB 记录的六次 read 仍合计 5,222 字节，64 MiB 记录的九次 read 仍合计 4,197,460 字节。Payload 认证量分别为 4,096 和 4,194,304 字节。该观察来自没有触发名称／可见性重试的样本：`open_committed` 对 NotFound 另有最多五次 File::open，不能把本表推广成任意故障下只有一次 syscall。

各单机 p50 均下降，但两个单机组合的 p99 上升。64 MiB 的 Python 阶段包装模式汇总 p99 也从 20.795 升到 22.538 ms；libc 模式则从 24.913 降到 21.685 ms。插桩改变计时，且没有驱逐缓存或隔离整个集群的背景负载，不能据此承诺所有尾延迟都改善。这些实验在本轮形式化、集成及 100 GB 任务结束后执行；这是四机各自读自己的记录，不是跨客户端发布可见性试验。

同一生产源码的原生探针另发布 **100,000,000,000 字节**位置相关数据，使用 23,842 个 4 MiB checksum；四机逐字节核对 373 个最多 256 MiB 的完整区间和 96 个边界区间（25 个为空）。累计返回 100,000,094,695 字节，认证 100,330,158,080 字节。数据准备耗时 112.335 s；发布与关闭为 352.091 s，270.860 MiB/s。四机完成耗时 55.239 s，包含 SSH、额外区间和 oracle，折合 1726.467 MiB/s。Writer RSS 高水位为 18,080 KiB，四个 reader 为 280736 / 280776 / 280720 / 280808 KiB。

完整 256 MiB 请求的原生 p99 为 546.046 ms（n=372）；非空小区间 p99 为 66.614 ms（n=71，因此为样本最大值），高于之前固定粒度轮次的 25.391 ms。该最慢区间在本机读取 `[0, 4099)`，认证 4 MiB；这次没有对大文件请求做 syscall 阶段计时，原因仍未归因。大文件使用 session，复用改动只减少每个 reader 初次读取的第二次打开；其他调用已经是单次打开。不同 seed、缓存、背景负载和二进制的跨轮次结果不能证明 bulk 加速或固定退化。Writer 的部分阶段与验证工作重叠，写入吞吐也不是裸文件系统上限。

另以同一二进制完成 32 MiB + 37 字节、8,193 个摘要的四机小负载；独立 Python 复算全部摘要，并检查远端每次返回的前后缀，原生 oracle 检查全部返回字节。正常 GC 回收大文件 100,003,195,567 字节、小负载 34,653,048 字节，以及延迟实验八个文件共 268,471,976 字节，owner／open／retained 均为零。随后移除私有 metadata 和远端程序，没有人工删除数据文件。没有新增物理 TB 吞吐结果；稀疏大偏移仍由完整回归单独覆盖。

全量检查通过：782 个 Python、40 个原生测试，302 个 TLC 案例，以及 119 项 Verus 验证和 105 个预期语义拒绝。单次成功取得描述符的数量是算法策略，p99 和带宽仍是部署测量，不成为环境时间上界。

## 名称查询与数据重试的组合（2026-10-24）

[完整记录](../verification/measurements/20261024-read-lookup.json)核对 `open_committed` 的名称重试与外层数据可见性重试。生产 Rust／Python 和 wheel 均与描述符复用轮次一致；本轮新增模型和探针，没有发现需要修改读取策略的新反例。

四台机器各自运行 84 个实际 straw API 注入场景，使用私有 4 KiB／64 MiB 记录，覆盖普通读取及冷／热 session。精确路径的 libc shim 注入一次／四次／五次 ENOENT、EIO、权限／配额／空间错误，以及与 payload 校验失败的交错。Payload 故障只翻转返回缓冲区的一个字节，不改磁盘；每个场景取消注入后重新读取并逐字节对照原内容。336 例全部符合预期，失败描述符均在下一次打开前关闭，GC 正常回收八个文件、268,456,360 字节。

最重的组合是每次成功打开前四次 ENOENT，再经历五次可重试校验失败，第六次成功／继续失败。两种结果均观察到 30 次 libc open 和六次取得／关闭描述符；没有 EINTR 的这些样本与模型的 30 次 Rust File::open 上限相符。显式等待为 4,750 ms。64 MiB 恢复场景的实际 API 耗时为 5,255.787–5,373.256 ms（n=12），另包含读取、认证、分配和调度，不能将等待和当成 API 超时。

随后单独补充四机 120 个 EINTR 场景。40 次 EINTR 后成功时，全部观察到 41 次 libc open；40 次 EINTR 后再四次 ENOENT并成功时为 45 次。这符合 [Rust 1.90 的 open 实现](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/fs/unix.rs#L1185)和其内部 [`cvt_r` 重试](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/sys/pal/unix/mod.rs#L310)。所以 **30 次只限制 straw 调用 File::open 的次数，不限制 libc／内核调用或 RPC 次数**。这些是返回 EINTR 的受控注入，没有发出真实信号，也不证明无限中断时终止。补充场景也正常 GC 八个文件；两组本机开发检查另有 84／30 例、四个正常 GC 文件。各阶段探针源码及二进制分别保留，未把后续版本冒充早期运行输入。

自然观测与上述注入分开：四台 writer 轮流发布新原始文件，另外三个客户端在创建前先 open 得到 NotFound；writer 写完并完成文件、所在目录 fsync 后，经 SSH 管道通知 peers 打开、完整读取并核对 SHA-256。每台 writer、每种尺寸各 50 个名称，共 400 个文件、1,200 次远端读取。全部第一次打开成功。

| 文件尺寸，每组 n=600 | 首次 open p50 / p95 / p99 / max，ms | 完整读与核对 p50 / p95 / p99 / max，ms |
|---|---:|---:|
| 2 KiB | 0.248 / 0.804 / 4.528 / 7.403 | 0.935 / 3.999 / 7.716 / 9.988 |
| 4 MiB | 0.271 / 3.543 / 6.773 / 71.509 | 12.416 / 18.521 / 26.998 / 86.566 |

Open 计时从 reader 发起该调用开始；完整读取计时含 open／read／hash。另存的通知到回执时间含 SSH、调度和 coordinator 收取回执的等待。没有使用跨主机时钟相减，没有隔离集群背景负载或驱逐缓存；部分形式化检查与测量重叠。这些原始文件不是 straw publication，不能称为端到端 straw 性能或传播延迟。所有 reader 退出后移除 400 个私有原始文件、839,270,400 字节，清理方式与正常 straw GC 分开记录。

没有在本轮重新运行 100 GB 或物理 TB 工作负载；此前同一生产源码的 100 GB 证据保留原运行身份。有限自然样本不产生 `NameBy`／`VisibleBy` 的必然界限，也不能证明没有名称延迟或完整系统没有 bug。

## 离线清理绕过共享保护的修复（2026-10-24）

[完整记录](../verification/measurements/20261024-offline-guard.json)复现了一个真实删除边界：`inspect_run` 用 `Path.exists()` 判断 `storage.log`／`queues` 是否存在。若目标标记的 stat 在初次报告和清理前重检时各虚假返回一次 NotFound，根队列日志会把它不知道的共享 pack 列为孤儿；即使所有参与者已停止，旧 `cleanup_orphans` 也会删除 checkpoint owner 或另一队列仍引用的数据。这里是受控故障注入，不是自然观察到 JuiceFS 返回了陈旧 stat。

四机各运行旧、新安装 wheel，每版十六个场景：catalog／queues 两种布局，2 KiB／8 MiB payload，以及 stat NotFound、stat EIO、open EIO、open EACCES。Catalog 布局从初始化就启用 GC，没有在线迁移；独立 checkpoint 持有 publication，根队列不引用它。Queues 布局是无 catalog 的根队列加另一命名空间队列，后者已在 WAL 接收该输入。所有队列和 writer 都在 inspection 前关闭。

| 四机结果 | 旧版 | 修复版 |
|---|---:|---:|
| 场景数 | 64 | 64 |
| 实际误删受保护 pack | 16 | 0 |
| 误删字节 | 67,144,696 | 0 |
| Catalog owner 或其他队列 WAL 仍有效 | 全部 | 全部 |

旧版十六次误删都来自 stat 假缺失；八次仍保留 checkpoint owner，另外八次仍保留另一队列已接收的 WAL。两者的数据读取随后失败。Python 3.12 上的 stat EIO 本来就会抛出错误，这些场景没有误删；不能把缺陷描述成“所有 stat 错误都会被吞掉”。旧 guard 在正常 stat 看见标记时就拒绝，所以针对 open 的故障在旧版没有触发。

新版通过只读 fresh open 检查两个标记，不读取 catalog 内容。存在即拒绝，除 FileNotFoundError 外的错误也以 `UnsafeRecovery` 拒绝；清理入口继续重新检查。所有 stat 注入在新版都未被消费；open EIO／EACCES 则实际触发并被拒绝。新版每个场景分别调用 inspection 和携带未验证报告的 cleanup，两次都拒绝，数据逐字节核对通过。两版 wheel 的 native 二进制 SHA 相同，仅 Python inspector 生产代码变化。

完成保护检查后，catalog 夹具显式释放 checkpoint，再正常 GC：旧版回收 24 个文件、100,717,044 字节，误删过的八个 catalog 文件不能算作 GC 回收；新版正常回收 32 个文件、134,289,392 字节。无 catalog 的 namespaces 夹具使用私有测试目录清理，旧／新各清理仍存在的 24／32 个数据文件、100,717,044／134,289,392 字节，单列为人工 fixture teardown。另有本机旧／新各十六例和最初的 Python 假 stat 复现；它们不混入四机表格。所有私有目录、注入 arm／trace 和远端环境均已移除。

这是离线管理工具的安全修复，不提供新带宽或 p99 保证。没有在本轮重跑 100 GB／物理 TB；Rust 读取路径保持上一轮二进制。`ENV-OFFLINE` 仍要求 fresh-open 缺失可信以及最终检查期间真正停机／命名空间稳定；TLA+ 的环境负例明确暴露 fresh-open 真实性的依赖；实际停机仍由调用方保证。修复不能恢复以前已经误删的数据，完整 Python／I/O 细化仍未证明。

## 离线别名清理对照（2026-10-25）

[记录](../verification/measurements/20261025-offline-alias.json)在 `the configured host list` 的四台机器上，对比上一轮 fresh-open guard wheel 与别名修复版。每个场景使用 `/shared/straw-test` 下私有、已停止、无 catalog 的单队列；别名均在任务接受之前建立，没有系统调用故障注入或外部重绑定。每台每版 28 个场景：七种引用形态、2 KiB／8 MiB payload，以及有／无另一条规范路径根。

| 四机汇总 | 旧版 | 修复版 |
|---|---:|---:|
| 场景数 | 112 | 112 |
| 清理后被接受引用无法读取 | 88 | 0 |
| 被接受的真实文件误删 | 56 | 0 |
| 真实文件仍在，但必经别名被删 | 32 | 0 |
| 误删字节 | 235,006,436 | 0 |
| 独立孤儿 pack 正常清理 | 112 | 112 |

形态包含独立 manifest 文件别名、经过 partial 文件的 pack 链、partial／sealed 目录别名、经过目录后再 `..` 的路径、显式别名记录和传递依赖。所有场景原始 journal 都保持不变。修复后 payload 逐字节匹配，同时仍删除真正的孤儿 pack 和悬空／孤儿 partial 别名。另一个不在 raw 枚举中的未引用目标必须保留，用来检查 unlink 的是候选原路径而非解析目标。两版 native 二进制相同，本轮只改 Python 管理路径。

报告中 `retained_paths` 改为规范物理身份，候选列表仍保留原始名字。检查器保护完整引用解析轨迹，而非只保护最终父目录；partial 候选也做相同检查。未知／越界候选身份中止检查，被保留引用的轨迹解析失败则禁止删除。所有私有池和远端环境均已清理；私有夹具拆除、正常离线孤儿清理与旧版有意复现的误删分别记录，不称为 catalog GC。每版另外 28 个本地场景，以及最初两组各四个复现，均与四机统计分开。

本轮 Python 改动不重复 100 GB 或实际 TB 工作负载。此前 native 吞吐结论仍仅适用于原测量条件，没有新增带宽或 p99 保证。参与者停止、真实解析、不可变数据和外部命名空间稳定仍为假设；新契约明确不能预先假设 straw 会保留本轮正在验证的别名。

## 验证工作中的运行性能对照（2026-10-25）

[测量记录](../verification/measurements/20261025-runtime-performance.json)对比 v0.1.3 commit 与 catalog-event 验证阶段的 release wheel，包含期间全部实现修复和重构，不能单独归因为证明注解。普通构建不包含 proof 函数、spec 和 ghost 历史；实际增加的写前准入、描述符／长度检查及 catalog 操作构造仍会执行，可能影响性能。

两版使用相同 release 工具链，在同一主机的独立安装环境运行。每个文件系统／负载组合各四次旧、新版试验，顺序为 B C C B C B B C。第一组共 16 轮，每轮包含 1 GiB 数据、500 次同步 WAL 追加和五次 2,000 帧 catalog 重放。补充的 48 轮将进程固定在同一个 CPU，分开测试 2 GiB 本地数据、启用在线 GC 的 1 GiB JuiceFS 发布，以及旧路径编码／规范化编码各 20,000 帧 catalog 的五次重放。同一种编码的 catalog 夹具在两版间逐字节一致。

| 测量项 | v0.1.3 中位数 | 当前版中位数 | 变化 |
|---|---:|---:|---:|
| JuiceFS 发布，关闭 GC，MiB/s | 214.98 | 217.22 | +1.04% |
| JuiceFS 校验读取，关闭 GC，MiB/s | 371.58 | 366.76 | −1.30% |
| JuiceFS 发布，开启 GC，MiB/s | 108.93 | 109.21 | +0.26% |
| 固定 CPU 的本地发布，关闭 GC，MiB/s | 219.53 | 215.56 | −1.81% |
| 固定 CPU 的本地校验读取，关闭 GC，MiB/s | 504.47 | 503.32 | −0.23% |
| JuiceFS 同步 WAL，每帧平均 ms | 15.455 | 15.676 | +1.43% |
| JuiceFS 旧编码 catalog，20,000 帧，ms | 53.567 | 58.100 | +8.46% |
| JuiceFS 规范化 catalog，20,000 帧，ms | 57.899 | 60.949 | +5.27% |
| 本地旧编码 catalog，20,000 帧，ms | 40.278 | 34.909 | −13.33% |
| 本地规范化 catalog，20,000 帧，ms | 43.154 | 37.257 | −13.67% |

表格取各轮中位数；重放先取每轮五次新 store 调用的中位数。吞吐越高越好，时延越高越差。记录保留全部样本和范围：开启 GC 的旧版发布为 31.89–111.33 MiB/s，当前版为 103.03–111.30 MiB/s。旧版异常慢的一轮没有删除，其 CPU 时间与其他轮接近。固定 CPU 的本地写入 CPU 时间增加 1.98%，JuiceFS 规范化 catalog 的 CPU 时间增加 7.55%；这些数据还不能把每项差异定位到某个实现改动或外部等待。

数据负载重复使用固定 seed 的 32 MiB 随机 payload，逐字节核对返回数据；持久发布计时不包含最后 writer close。读取／重放使用暖缓存，没有全局驱逐缓存或隔离集群后台负载。Catalog 计时含 replay、根／候选快照和恢复屏障，没有物理 GC 删除。这些是应用逻辑字节速率，不是裸设备／对象存储流量、队列 RPC 吞吐或四客户端 100 GB 重测。每版四次试验不能建立稳定尾延迟，也不能证明所有负载都无回退。

本次对照明确保留共享文件系统上 catalog 变慢的结果；形式化检查通过不等于性能验收通过。测量期间没有改动生产源码，确切的历史探针、wheel、源码、原始报告和哈希单独归档。复现探针现在保留原始延迟样本并按 nearest-rank 计算分位数；旧探针分位数下标的偏移已在记录中注明，上表不使用这些分位数字段。

## catalog 操作构造性能对照（2026-10-25）

[操作构造阶段](../verification/measurements/20261025-catalog-decode.json)对照本次 release wheel 与上一 catalog-event wheel，基线不是 v0.1.3。同机固定 CPU、交错两版、新建私有 fixture，共 64 轮。每行每版四轮，每轮三个热缓存 replay／snapshot 调用；表中取每轮中位数的中位数。两版 fixture 哈希一致，多路径 owner 覆盖批量集合构造成本。

| 文件系统 | 编码 | Frames | 每 owner 路径数 | 前版 ms | 本版 ms | 耗时变化 |
|---|---|---|---|---|---|---|
| local | legacy | 20,000 | 1 | 35.442 | 35.154 | -0.81% |
| local | legacy | 2,000 | 64 | 59.064 | 58.589 | -0.80% |
| local | normalized | 20,000 | 1 | 36.721 | 36.802 | +0.22% |
| local | normalized | 2,000 | 64 | 50.031 | 49.904 | -0.25% |
| shared | legacy | 20,000 | 1 | 51.739 | 50.727 | -1.95% |
| shared | legacy | 2,000 | 64 | 75.996 | 75.563 | -0.57% |
| shared | normalized | 20,000 | 1 | 51.824 | 53.779 | +3.77% |
| shared | normalized | 2,000 | 64 | 70.380 | 67.625 | -3.92% |

共享盘 normalized 单路径组耗时增加 3.77%；其余组变化为 −3.92% 至 +0.22%。这些小样本热缓存结果不证明零开销，也不建立带宽或延迟保证；没有重跑数据带宽或 100 GB 工作负载。保留全部正式样本并清理各自私有目录。一次前置安装检查发现 pip 沿用了同版本全局包，探针在新版本计时前拒绝该二进制；保留该失败日志及未配对的基线样本，单独排除在正式交错对照外。

## 提交分派器性能对照（2026-10-25）

[提交／恢复阶段](../verification/measurements/20261025-commit-recovery.json)对照本次 release 与上一 catalog-decode 构建。`runtime_performance.py --phase commits` 开启在线 GC，使用一个共享不可变输入，测量任务接受／取消及最终物理回收。同一 CPU 交错执行 16 轮，每文件系统每版四轮；本地每轮 128 个任务，共享盘每轮 64 个。私有目录均在校验后清理。

| 文件系统 | 操作 | 单位 | 前版 | 本版 | 耗时变化 |
|---|---|---|---|---|---|
| local | accept | ms/task | 0.270 | 0.255 | -5.74% |
| local | retire | ms/task | 0.061 | 0.070 | +15.31% |
| local | collect | ms/collection | 0.311 | 0.305 | -2.05% |
| shared | accept | ms/task | 61.957 | 63.409 | +2.34% |
| shared | retire | ms/task | 30.750 | 29.980 | -2.51% |
| shared | collect | ms/collection | 112.816 | 99.808 | -11.53% |

最初本地退休小样本增加约 9 µs／任务（+15.31%）。因此另行进行每版六轮、每轮 2,048 任务的交错复核，保留原样本：本地接受 wall／CPU 变化为 −0.48%／−0.36%，退休为 +0.66%／+1.76%，较大负载没有复现最初幅度。共享盘接受耗时增加 2.34%。这些同机热缓存结果不能证明零开销或延迟 SLA，也未重跑 100 GB 数据负载。

## catalog 恢复调用链对照（2026-10-25）

对照本次恢复组合与前一提交恢复阶段的 release wheel，单机固定一个 CPU，交替执行两个版本；每个文件系统／编码／版本各 4 轮，共 32 轮。每轮使用独立的 4,096-frame catalog，每个 owner 含 4 条路径，执行 7 次恢复及根／候选快照；下表汇总各轮耗时中位数。测量不驱逐缓存，不代表 100 GB 数据带宽或 p99 上界。

| 文件系统 | 路径编码 | 前版 → 本版（ms／恢复及快照） | 耗时变化 | CPU 时间变化 |
|---|---|---|---|---|
| 本地 | legacy | 11.712 → 11.759 | +0.41% | +0.33% |
| 本地 | normalized | 12.548 → 12.400 | -1.18% | -1.18% |
| 共享文件系统 | legacy | 28.944 → 29.045 | +0.35% | -0.60% |
| 共享文件系统 | normalized | 28.741 → 28.933 | +0.67% | -0.39% |

全部轮次、原始样本及二进制哈希均保留在[阶段记录](../verification/measurements/20261025-catalog-recovery.json)。这组结果未显示明显回退，但不能证明没有运行时成本；证明和性能验收仍分别审查。

## GC 批量执行对照（2026-10-25）

[批量删除阶段](../verification/measurements/20261025-gc-batch.json)对比前一 catalog recovery wheel 与经过验证的批量执行体。单机固定 CPU，交错运行 16 轮，每版／文件系统四轮；每轮在新负载上执行三次回收。每份负载有三个 writer 目录，本地每个 writer 有 64 个 pack，共享文件系统每个 writer 有八个。发布、seal 和 staging 释放不计时；collection 计时包含 refresh、候选选择、持久 mark、unlink、父目录屏障和回执。

| 文件系统 | 每次回收 pack 数 | 前 → 后（ms/collection） | wall 变化 | CPU 变化 |
|---|---:|---:|---:|---:|
| 本地 | 192 | 9.649 → 9.827 | +1.84% | +0.71% |
| 共享 | 24 | 156.157 → 167.855 | +7.49% | +1.31% |

表中是四轮各自中位数的中位数。共享文件系统各轮中位数前版为 151.833–209.022 ms，后版为 161.688–187.643 ms。所有样本及 native 哈希均保留，不隐藏本次测得的变慢，也不凭这些样本归因或宣称没有回退。改写没有增加文件系统调用，统计改为 checked arithmetic，但源码审查不能证明运行成本完全相等。缓存及背景负载未受控；本次测的是回收延迟，不是新的 100 GB 带宽或 p99 上界。
