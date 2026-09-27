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
