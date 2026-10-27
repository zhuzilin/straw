# 测量实际负载

[English](BENCHMARKS.md)

安装 wheel 后即可使用 `python -m straw.benchmark`，也可调用 `straw-benchmark`。它测量打包写入、持久化队列接收和带校验的读取，无需模型或 GPU。

## 单机

```sh
python -m straw.benchmark run --local \
  --root /tmp/new-straw-bench --report benchmark-results/local.json \
  --record-bytes 1048576 262144 --gib 0.0625 \
  --writers 2 --readers 2 --segment-mib 8 --online-gc \
  --max-files 64 --max-gib 0.25 --timeout 120
```

每个任务发布两条记录，分别为 1 MiB 和 256 KiB。一个本地 worker 进程运行指定数量的读写线程，协调器位于父进程。必须使用新 root，报告必须放在 root 之外。载荷在本地生成，读取会校验存储 checksum；总读取字节数必须符合 fanout。启用 `--online-gc` 后，运行期间并发执行 GC，并在所有 benchmark reader 完成后检查实际回收。

## 多机

每台机器安装同一个 wheel，需要相同的可写挂载路径、launcher 到各机的 SSH 连接，以及到 launcher 协调器的网络连接。在列出的第一台机器上启动：

```sh
python -m straw.benchmark run --hosts HOST_A HOST_B HOST_C HOST_D \
  --root /shared/new-straw-bench --report benchmark-results/four-host.json \
  --record-bytes 12582912 4194304 4194304 --gib 8 \
  --writers 2 --readers 2 --segment-mib 1024 --online-gc \
  --max-files 128 --max-gib 12 --timeout 600
```

第一台机器也参与读写。默认 fanout 为 1，按环形分配，使每个远端 worker 读取另一台机器写入的数据。`--fanout 4` 让四台机器各读取全部 publication。Fanout 必须为 1 或机器数；它会改变总读取量，比较性能时应保持一致。

Launcher 在**每台机器启动一个 worker 进程**。`--writers`、`--readers` 指定进程内线程数，不是进程数；每个 writer 线程创建独立 store/pack 写入流。协调器位于 launcher 进程。本地模式在 launcher 所在机器上运行一个 worker 进程。straw 本身不创建进程池，应用自行选择执行拓扑。

`--python /path/to/python` 指定远端已安装包的解释器。用户名和端口由 SSH 配置控制。SSH 别名与可访问地址不同时，用 `--advertise-host ADDRESS` 指定协调器地址。默认监听 `0.0.0.0`，应使用私有任务网络。只有源码开发时才需要 `--source-dir /shared/checkout/src`；wheel 模式不需要共享源码树。

未指定 `--record-bytes` 或 `--profile` 时，默认使用显式 AI 形状模型：四个 8K-token sample，每个含 int32 路由 `[tokens,48,8]`、int32 候选 ID `[tokens,128]` 与 float32 分数 `[tokens,128]`。`--tokens`、`--samples` 可调整规模。这是合成数据尺寸，不是在运行模型。

## 分析并重放应用轨迹

```sh
STRAW_TRACE_DIR=/shared/my-trace python your_application.py
python -m straw.benchmark profile --trace /shared/my-trace \
  --output benchmark-results/profile.json
python -m straw.benchmark run --hosts HOST_A HOST_B HOST_C HOST_D \
  --root /shared/new-straw-replay --report benchmark-results/replay.json \
  --profile benchmark-results/profile.json --gib 8 \
  --segment-mib 1024 --online-gc --max-files 128 --max-gib 12
```

所有 worker 必须能从相同路径读取 profile。Trace 每个进程写一个追加文件，记录大小、时间、codec 和张量形状元数据，不记录张量值或应用记录元数据。Trace 放在载荷 root 之外，保留策略由应用负责。

重放用固定随机种子采样观察到的 publication 大小分布。默认每个任务保持一个 publication，不保留原始请求到达时间、队列/优化器语义或全部按行切片读取。有限任务数可能使实际 GiB 向上或向下取整，以报告为准。`--publications-per-task N` 会显式将 N 个观察到的 publication 合成一次任务/写入，改变事务粒度；不能将它与未合并负载标为相同工作负载的公平比较。

## 阅读报告

| 字段 | 含义 |
|---|---|
| `written_bytes`、`read_bytes` | Worker 实际完成的应用载荷字节数 |
| `write_mib_s`、`read_mib_s` | 载荷量除以最慢 worker 的运行时间 |
| Worker 延迟分位数 | Publication、接收 RPC、读取完成的延迟 |
| `data_files`、`file_count`、`physical_bytes` | 最终确认回收前的文件数量与逻辑文件长度 |
| `gc`、`gc_seconds` | 最后一次 GC 的结果和延迟 |
| `concurrent_gc_passes` | Worker 活跃期间执行的 GC |
| Worker CPU/RSS | 进程 CPU 时间与峰值常驻内存 |

新路径和跨客户端读取不代表冷后端存储，测试不会全局清除缓存。应用吞吐不等于对象存储/设备带宽、网络流量或文件系统内部请求数。比较时同时考虑硬件、文件系统/缓存设置、形状、publication 大小、reader fanout、持久化约定与保留的工作量。

Trace 写入统计带 framing 的 extent 字节，同时单独报告 payload 字节；读取统计返回给调用者的字节。`checked_read_bytes` 统计经过认证校验的数据块，不含 index/journal 流量。`checked_read_amplification` 为校验块字节数除以返回字节数，缺少 range-read 计数时记为未知。`read_to_extent_write_ratio` 单独描述读写比例；早期实验 profile 曾将这个比例称为 `read_amplification`，但其 publication 大小列表仍可重放。完成速率峰值将字节计入操作完成所在的对齐 1/10 秒窗口，因此要求各机时钟对齐。

## 存储与清理

`--max-files`、`--max-gib` 限制有限测试的规模。Benchmark 在临时 root 外保存 JSON 报告、不含载荷的 `.logs.tar.gz` 以及 `.cleanup.json`。成功结束时先确认自有 worker 已停止，再删除 root；SSH 断开并不能证明远端进程已退出。失败的测试（包括容量用尽）会保留 root 供检查，不自动重置队列、提高上限或重新启动。无法检查或停止远端机器时同样保留 root。确认 worker 死亡前不要删除。Root 预算不覆盖外部报告/trace 目录。

## 元数据与小范围读取

对每个已安装 wheel 运行同一探针，测量 writer 历史和交替 extent 读取的开销：

```sh
python verification/storage/probes/metadata_scaling.py \
  --count 1000 --output /tmp/straw-metadata.json
```

`--parent` 选择文件系统，`--online-gc` 纳入 catalog 工作。探针使用私有临时 root，关闭 store 后清理，并记录原生二进制哈希。结果包括最近写入中位数、registry 大小、Linux 进程逻辑读取量、同步 hook，以及在四个 extent 之间交替进行的 400 次认证读取。Journal／file sync hook 可能指向同一次屏障，不应重复相加。逻辑读取可能命中缓存，不是设备／网络流量。对照应保持负载、挂载、解释器和 GC 模式一致；这些小型测试不代表生产吞吐或尾延迟上界。

<a id="published-results"></a>

## 精选公开基准

以下记录来自 2026-10-26 的 0.1.4 候选构建，早于后续验证精简与 owner 修复；不是当前源码的重新测量。只保留支撑下述公开结论的数据、样本与哈希。挂载／硬件条件影响结果，旧构建数字不能当作当前性能承诺。

### 0.1.4 候选版性能对照

[发布对照记录](../verification/measurements/20261026-release-comparison.json)在同一台机器、同一 JuiceFS 挂载上，按旧版／新版／新版／旧版顺序比较 0.1.3 与 0.1.4 候选版。每轮使用新目录并开启在线 GC，发布及认证读取 1 GiB（32 × 32 MiB），追加 500 个 WAL frame，重放 2,000-frame catalog，并执行任务接受、退休和回收。WAL 与 catalog 恢复各运行五次。

| 指标 | 0.1.3 | 0.1.4 候选版 | 变化 |
|---|---:|---:|---:|
| 发布吞吐 | 134.93 MiB/s | 107.00 MiB/s | −20.7% |
| 认证读取吞吐 | 314.18 MiB/s | 322.91 MiB/s | +2.8% |
| 500 次同步 WAL 追加 | 8.395 s | 8.094 s | 耗时 −3.6% |
| 接受 64 个任务 | 2.843 s | 3.832 s | 耗时 +34.8% |
| 回收 24 个 pack | 153.84 ms | 161.28 ms | 耗时 +4.8% |

表中数值是每个构建两轮进程运行的中位数；GC 先取每轮三次回收的中位数。全部阶段成功。未清空缓存，紧随写入的读取和重复恢复可能命中客户端／服务端缓存；不能据此推断多机峰值带宽或延迟上界。两轮新版均出现发布及任务接受回退。此次比较的是包含持久化与 writer fencing 改动的完整构建，未做 profile，尚不能归因。记录保留全部样本及确切 wheel、原生库和探针哈希；下文历史测量仍只代表各自的构建和负载。

### 0.1.4 候选版：四节点读取单条 100 GB 记录

独立的 [100 GB 测量](../verification/measurements/20261026-release-100gb.json)由一个节点发布一条真实记录，payload 为 100,000,000,000 字节，内容随字节位置变化。四个 reader 分工覆盖全部字节，并逐字节核对独立计算的预期值。它与上面的 32 MiB publication 对照属于不同负载，不是版本间比较。

| 阶段 | 实测结果 |
|---|---|
| 发布前 checksum 准备 | 113.501 s，排除在 publication 计时之外 |
| 单节点 publication | 358.855 s；265.755 MiB/s |
| 四节点读取及逐字节 oracle 的总 wall time | 55.275 s；合计 1,725.333 MiB/s |
| 各节点 native 完整分片读取吞吐 | 593.8–625.8 MiB/s |

publication 本身仍包含位置相关 payload 的重新生成、认证和同步。读取总 wall time 包含调度、边界切片和逐字节 oracle；各节点 native 计时不含随后执行的 oracle。缓存未清空，这些逻辑吞吐不是冷读或后端存储带宽上限。单次 publication 不能提供 publish p99。

释放所有权后，正常 GC 报告回收一个文件、100,003,195,567 字节（包括 framing 和元数据）。这表示文件系统命名空间中的删除已持久化，不代表已经测得对象存储后端物理释放空间的时刻。报告保留确切探针源码与原生二进制哈希。

### 四节点可见性采样

[可见性探针](../verification/measurements/20261026-release-visibility.json)使用一个 writer 和三个远程 reader，每种模式发布 1,000 次、每次 64 KiB。共 6,000 次读取全部成功，没有失败重试。从 fsync 返回到收到校验成功回复的实测 p99，保持 writer 打开时为 9.515 ms，关闭 writer 后为 9.035 ms。该指标包含通知、fresh open、读取、hash 校验和回复，不是纯文件系统可见延迟，也不是必然延迟上界。两种模式使用同一挂载且未清空缓存；所有探针进程正常退出，临时数据已清理。
