# 文件系统部署

[English](FILESYSTEM.md)

straw 使用**已挂载的 Linux/POSIX 文件系统**，依赖真实文件、目录、追加写、文件/目录 fsync 和 advisory file lock。它不会发起对象存储 API 请求，也不替代文件系统客户端。单机可用本地文件系统，多机需要访问同一份共享池内容。

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

## 文件数与空间规划

- 长期使用的 writer 把多次 publication 及其内嵌 manifest 追加到 pack，默认轮换目标为 **1 GiB**。不会为每个 sample 创建临时文件或旁路 manifest 文件；`publish_many` 也能减少事务开销。
- 每次 writer 启动都有独立目录和当前 pack；writer 越多，未填满的 pack 越多。每个 sample 都创建 writer 或 seal 会失去打包效果，应在进程/worker 范围内复用。
- 下一次 publication 将超过目标大小时先轮换。单次超大 publication 保持连续，因此可能超过目标。除目标大小外，也要配置 record/buffer 限制；目标大小不是配额。
- 每个队列有 `run.json` 和一个 `control/journal.log`。命名空间队列共享数据 pack 与池级 `storage.log`。用户 owner 名称是日志条目，不是文件。可选 trace 每个进程增加一个追加文件。
- GC 只删除完全失去引用的封存 pack，一条存活记录就可能保留整个 pack。必要时将保留周期差异很大的数据写入不同 writer 流；目前没有存活记录搬迁/整理。
- 崩溃的 writer 可能留下 open pack/staging 所有权；空 writer 目录和 WAL/任务历史也会随运行时间增长。超时无法证明它们已经死亡。

根据形状/dtype 和保留的活跃版本估算张量字节数，再加上 framing、未填满 pack、checkpoint、reader、排队结果和 WAL。Pack 数量近似为数据量除以目标大小，再加各 writer 的未满 pack；publication 大小和显式 seal 都会变化，因此这不是严格数学上界。队列逻辑预算可能重复计算共享数据，也不限制物理占用。

应用应设置准入/保留预算与文件系统配额。Benchmark 有显式 `--max-files`、`--max-gib` 防护。每个有界任务使用新池，删除池前停止全部参与者。引用仍存活时，不要通过直接删除 pack 文件来强行满足配额。
