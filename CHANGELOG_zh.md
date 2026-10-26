# 更新日志

[English](CHANGELOG.md)

## 0.1.4

### 恢复与续读

- 在线 GC 保留 producer continuation，以及独立的 reader、consumer 和 checkpoint 根。采样进程失败后可以从已持久化的 continuation 恢复。训练应用仍须选择并加载 checkpoint，恢复相匹配的模型／optimizer 状态与数据游标，并保留重放需要的数据；straw 不会自动加载模型 checkpoint，也不能恢复已回收的数据。
- 重试已接受的 consumer state、batch plan、batch ready 和 checkpoint registration 时，先查询持久回执，再决定是否读取旧引用。即使旧数据已合法回收，相同请求仍可返回原结果，不会重新建立所有权。
- 持久化 staging 清理意图、依赖关系和对应代际的完成回执。丢失回复后，恢复可以完成已接受的交接，不会释放同一引用后来重新建立的 staging。
- WAL 写入结果不确定、状态部分应用或捕获 Rust callback panic 后，保持隔离直至恢复。追加前拒绝畸形 journal／catalog 事件，重建时保护已确认前缀，并拒绝池／run／日志身份变化。补齐计数溢出检查，发放 lease 前对重复 task ID 去重。

### 存储与读取

- header、footer、index、envelope、payload 和 tensor checksum chunk 可在同一有限预算内重试各自的可见性延迟。返回的数据始终经过认证；持续损坏会在预算耗尽后报错。
- 同次读取尝试复用认证索引的文件描述符，catalog replay 使用至多 64 KiB 预读；失败后重新打开文件。依赖遍历改用去重工作队列，支持深层依赖图。
- 读取 payload 前检查 tensor dtype、shape 和长度。新 tensor 使用 4 MiB checksum chunk，旧数据沿用存储的 chunk 大小。checkpoint 重新激活在 catalog 锁外认证全部可达 payload，payload scratch 至多 1 MiB，随后在锁内重新检查所有权和 tombstone。
- JSON 有限浮点数在序列化／恢复中精确往返。拒绝遍历长度不符或吞掉 callback 错误的原生 payload provider。

### GC 与发布安全

- unlink 后重试目录持久化屏障，包括文件已不存在或父目录缺失的情况。共享父目录的屏障合并执行，持久删除完成后才记录回收回执。永久保留 tombstone；冷缓存 retain 遇到已回收的根或依赖时，在读取缺失 payload 前返回 `InvalidReference`。
- 保护规范路径及别名、其他队列、活跃 reader、checkpoint、producer continuation 和未完成 publication。公共 owner API 拒绝内部 `queue:`、`publication:` 名字。离线清理保留控制文件，在 Python 3.13 及更早版本中均拒绝符号链接环，共享池发现结果不确定时拒绝继续删除；无引用的悬空链接仍可清理。
- 发布准备、同步、staging、sealing 失败时保留依赖保护和重试身份。补齐缺失的祖先目录屏障，writer 复用前回滚未发布尾部，并封存已完成的 standalone reservation 以便 GC。GC hook 报错且已影响 WAL 时，后续写入必须先重新恢复。

### Writer 退休

- 新增显式 `retire_writer(..., policy="recover" | "abandon")`，通过持久 fence、操作登记和可复用恢复回执处理旧 writer。recover 先保护完整结果再释放准备所有权；abandon 保留已有 queue、reader 和 staging 根。仍持有 gate 的 writer 返回 `WriterBusy`，已被 fence 的 writer 无法恢复写入。退休依赖已列明的锁、fence 可见性和在途 I/O 契约，不能仅凭超时判断进程死亡。
- writer 正执行 callback 时，metrics、seal、close 立即返回 busy，不再等待。无法确认来源的历史 preparation owner 继续受保护，并报告为独立恢复事项。

### 兼容与迁移

- packed record 和 WAL 字节 framing 不变，旧 catalog 记录仍可重放。新增 queue staging handoff 事件需要新版 coordinator：一旦写入这些事件，不要降级 queue writer。依赖规范路径／别名保护前，应升级全部 GC 参与者；旧 collector 仍可能删除通过别名持有的文件。混用版本不具备统一的清理／重试保证；历史泄漏和已回收的旧 manifest 不会自动修复。见[格式兼容说明](docs/FORMAT_zh.md#队列-staging-交接凭据)。
- Rust 调用者需改用 coordinator 只读 accessor（`state()`、`journal()`、`store()`、`limits()`、`epoch()`），通过独立 `Store` 发布，并将显式 `&mut Catalog` callback 改为 `&mut LockedCatalog<'_>`。`Config::gc_enabled` 返回 `Result<bool>`。现有 Python 调用签名保留，另增加退休 API。
- 旧版本已舍入的浮点数无法还原；旧请求 hash 和使用不同 tensor chunk 策略生成的重试可能在升级后冲突。保留重试身份，并检查[存储格式规则](docs/FORMAT_zh.md)。catalog 依赖关系和回执暂未实现压缩，元数据会持续增长。

### 已知性能回退

- 相同 JuiceFS 工作负载、启用在线 GC 时，相比 0.1.3，写吞吐中位数下降 20.7%，接受 64 个任务的耗时增加 34.8%；读取和同步 WAL 追加未见同量级下降。此次简短对照每个构建只有两轮，缓存未清空；尚未做 profile，不能归因于某项安全协议或验证相关重构。见[完整样本与构建哈希](verification/measurements/20261026-release-comparison.json)。

### CI 与验证边界

- 扫描源码分发包与 wheel 内容（包括原生库），检查本机路径和可识别的凭据。编译时 remap Rust 路径，避免发布二进制携带本机源码、用户目录和工具链位置；CI 上传产物前检查，发现问题即拒绝，报告不输出命中的秘密内容。
- release 构建和发布以前置检查为条件，包括 TLA+／Verus、语义负例、environment contract 审计、Rust／Python 测试、格式和 Clippy。TLC 只记录 jar 哈希、不做校验；Verus 日志明确区分证明通过与预期语义拒绝。验证目录按协议、WAL、GC、存储与离线恢复组织，环境假设可独立审查。
- 本次发布冻结新增形式化验证。已集成证明覆盖具体生产函数和模型；完整 retain／reactivation、跨日志恢复、认证与活性组合延期，以回归和故障注入测试覆盖。见[本次发布范围与边界](docs/FORMAL_VERIFICATION_zh.md#本次发布的验证范围)。测试通过不等于这些证明已完成，历史性能测量也不能代表当前构建的吞吐或延迟。

## 0.1.3

- 已发布 extent 的文件头在任意位置出现暂时零值时都会触发重试，支持部分字节以及不连续位置尚不可见的情况。此前只有整个文件头为零时才会触发这类重试。重试上限和 checksum 校验保持不变；其他错误的文件头字节仍会立即报错。

已有调用方和存储数据保持兼容，packed record 和 WAL 格式不变。

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
