# 更新日志

[English](CHANGELOG.md)

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
