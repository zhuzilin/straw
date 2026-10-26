# 文件系统部署

[English](FILESYSTEM.md)

straw 使用已挂载的 Linux/POSIX 文件系统。多个客户端需要同一存储池、稳定命名和一致的持久化／锁语义；不直接调用对象存储 API。

## 部署身份与持久化

[环境契约](DESIGN_zh.md#environment)定义协议前提。下面说明实际部署如何满足并检查这些条件。


Catalog 使用一个固定的 `storage.log` inode。客户端仍存在时不要替换或重命名它。事务持锁后会重新打开 I/O 描述符，以读取新内容；持有锁本身不意味着所有文件系统缓存均已失效。

远端 unlink 后允许暂时存在过期的文件属性缓存：删除已经 tombstone 的文件时，ENOENT 是幂等结果。其他错误仍必须报错。

所有协调器应使用相同、规范化的绝对池挂载路径；队列 owner 身份目前包含其控制根路径。相对记录引用可以通过其他挂载根读取，但不能用这种方式迁移运行中协调器的所有权命名空间。变更本地根时，应针对读取 store 重建 `TensorRef` 描述符。

为防御可能暴露陈旧或不一致视图的部署，reader 容忍长度、header、索引和数据分别延迟可见，并对相关校验失败做有限次 reopen/revalidation。这超出了 JuiceFS 的默认契约，不表示正确部署一定会产生这些延迟。文件系统必须在该重试预算内提供完整正确视图，才能保证成功；没有界限的最终可见性只能保证持续错误不会被当作有效数据返回。WAL/catalog 仍要求持锁后重新打开能读到完整已提交前缀；不要把 extent 的重试策略推及日志截断恢复。参见 [TLA+ 假设](DESIGN_zh.md#review)。

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

信任某个部署之前，在隔离目录上运行[多客户端故障/GC 检查](#qualification)。SIGKILL 进程后，内核和存储服务仍在运行；它不能证明掉电、元数据服务故障切换、客户端断连或设备写入重排下的保证。这些故障需要文件系统/厂商单独认证。

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

<a id="qualification"></a>

## 多个文件系统客户端

准备共享源码和挂载，四台机器具有相同可用的包/解释器并配置 SSH：

```sh
python verification/wal/checks/crash_campaign.py \
  --hosts HOST_A HOST_B HOST_C HOST_D --parent /shared/test-tmp \
  --output /tmp/straw-four-client-crashes.json
```

在第一台机器上启动 launcher。测试程序把子进程崩溃/交接用例分配到四个客户端，在释放源之前检查恢复后的所有权。它使用共享验证脚本；运行已安装 wheel benchmark 的应用使用者不需要该源码树。

[多机 benchmark](BENCHMARKS_zh.md)补充并发打包 I/O、持久化接收、独立 reader 和在线回收。限制存储预算并使用新 root。检查返回字节数、退出状态和清理记录，不能只看吞吐或 launcher 的成功提示。

## 专项 GC 检查

`verification/gc/probes/gc_scale.py` 使用微小记录测量所有权管理成本。在每个客户端各启动一份，使用不同 queue 名称和同一个新 root：

```sh
python verification/gc/probes/gc_scale.py --root /shared/new-gc-scale \
  --queue HOST_A --count 1024 --participants 4
```

在 HOST_B/C/D 上同时运行对应命令。每个进程向自己的命名空间队列提交 1,024 个根，加入四客户端 barrier，再重新打开协调器、计时 GC、抽样读存活记录并验证无引用 pack 的删除。设为 `--count 16384` 可得到共 65,536 个根。计时排除协调器/catalog 重放，不测量张量带宽或冷存储。调用方在**四个进程全部停止后**负责清理。继续被中断的测试时，先确认旧 worker 已停止，再在同一队列上使用 `--resume`。

`verification/gc/probes/gc_cross_client.py` 专门验证远端删除后仍缓存正向文件属性的情形。在一个客户端准备唯一 root：

```sh
python verification/gc/probes/gc_cross_client.py --root /shared/new-gc-cache
```

随后四个客户端各使用不同 `--worker` 名称运行：

```sh
python verification/gc/probes/gc_cross_client.py --root /shared/new-gc-cache --worker HOST_A
```

每个客户端先预热 stat 缓存，写一个 `ready-<worker>` 标记。四个标记全部存在后，监督程序创建 `/shared/new-gc-cache/go`。Worker 并发执行 GC，必须全部成功退出，且 `reclaimed_files` 总和为一；之后才能删除私有 root。Barrier 的 60 秒超时限制失败准备阶段的等待。该测试只有一个 pack 和固定数量标记，不会每个 sample 创建一个文件。

## 故障范围与报告

保留失败构建／源码身份、操作 ID 与小复现。进程退出不会关闭内核和存储服务，因此单独验证真实锁、缓存设置、服务中断、设备／VM 掉电及 ENOSPC/EIO。后台失败不能当成操作回滚，也不能因 RPC 回复丢失就释放未确认的所有权。

公开性能结果与测量方法集中在 [benchmark](BENCHMARKS_zh.md#published-results)，原始部署日志保存在受控的本地或 CI 产物中。不要删除仍有 reader 或不确定进程使用的数据。
