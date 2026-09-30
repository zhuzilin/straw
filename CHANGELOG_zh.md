# 更新日志

[English](CHANGELOG.md)

## 0.1.2

- 大记录的 payload 通过有界 native 写入缓冲区分块写入。记录和批量发布不再需要适配 writer 的临时缓冲区；原有 packed record 格式和张量读取方式不变。
- 移除记录数、依赖数、结果大小、队列任务、控制消息及元数据的固定逻辑配额。旧的限制参数继续保留，以兼容 API 和队列身份；`max_buffer_bytes` 仍约束临时 payload 复制。
- 共享文件系统上，已发布 extent 的文件长度可能先于文件头内容可见。reader 遇到这种短暂情况会延迟并重新打开文件；持续失败、非零坏文件头和 checksum 错误仍会报错。

已有存储数据和调用方保持兼容。上述写入和读取行为需要 0.1.2；packed record 和 WAL 格式不变。

## 0.1.1

- 持久化任务调度：按 `TaskSpec.priority` 从高到低、`scheduling_key` 从低到高、再按 FIFO 顺序领取任务。两个字段默认均为 0，接受有符号 64 位整数。Rust pending 索引无需读取载荷即可选择任务，WAL 恢复时重建调度顺序。
- 新增 `Coordinator.yield_tasks()`：原子保存一批 continuation 输入及可选调度字段，结束这些任务的 lease 并归还到 pending 队列，不消耗失败重试次数。归还的任务进入相同调度键的 FIFO 队尾；相同请求可幂等重试，任一 lease 失效则拒绝整批操作。
- 新增 `Coordinator.pending_tasks()`：按领取顺序返回 pending 任务的 spec，支持按任务 ID 前缀及最低优先级过滤。应用负责协调暂停后的快照，并持有所引用的输入。
- 补充优先级/FIFO 恢复、失效 lease、非法调度键，以及 WAL 崩溃窗口下批量操作全有或全无恢复的回归测试。

已有调用方可以不指定新增调度字段。Packed record 与 WAL 的 framing 不变；新增 API 和调度行为需要 0.1.1。仅有 pending 任务快照尚不构成应用联合 checkpoint，也不提供任意 checkpoint 回退。

## 0.1.0

straw 首次发布：面向 AI 应用、基于文件系统的持久化队列与共享张量存储，以 `straw-queue` 包分发，采用 MIT 许可证。

- Rust 实现存储、WAL、队列及所有权协议，提供 Python API。多台机器通过共享文件系统并行读写，首个共享存储目标为 JuiceFS。
- 不可变记录与 manifest 打包存储，支持批量发布和按大小轮换文件，避免每个 sample 或张量单独创建文件。
- 支持 bytes、自定义 codec、NumPy/PyTorch 张量、带校验的惰性按行读取，以及复用已认证索引的 native 读取会话。
- 跨队列共享张量引用、张量级写时复制、显式 reader/checkpoint 所有权，以及可选在线 GC，回收无引用的封存 pack。
- 持久化任务 lease、continuation 进度、完成 receipt 和消费者状态，支持 WAL 恢复与已保留 checkpoint 的恢复。
- 本地与多机 I/O benchmark、应用负载采集与重放、独立协议模型，以及进程崩溃与恢复检查。
- CPython 3.10–3.13 的 Linux x86_64 wheel、自动化产物测试与 PyPI Trusted Publishing、独立使用示例和配套中英文指南。

每个队列需要一个由外部机制隔离保护的协调器。尚不提供自动故障切换、存活 pack 整理、日志压缩或通用分布式应用 checkpoint 事务。其他网络文件系统需要单独验证部署语义。详见[文件系统要求](docs/FILESYSTEM_zh.md)与[协议保证](docs/PROTOCOL_AND_VERIFICATION_zh.md)。
