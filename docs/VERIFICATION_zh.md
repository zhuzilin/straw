# 验证协议与部署

[English](VERIFICATION.md)

[协议文档](PROTOCOL_AND_VERIFICATION_zh.md)说明不变量、持久化顺序、假设和参考工作。以下检查覆盖不同边界；单元测试全绿不能证明文件系统掉电安全。

## 快速本地检查

在已安装 Rust 1.89+、PyTorch 和开发包的源码目录运行：

```sh
cargo fmt --check
cargo clippy --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/ownership_model.py --output /tmp/straw-ownership.json
python verification/crash_campaign.py --output /tmp/straw-crashes.json
```

| 检查 | 提供的证据 |
|---|---|
| Rust 测试 | Native 打包、中断尾部、原子接收和 fencing |
| Python 回归测试 | 通过公开绑定覆盖 native 核心：损坏/截断、有界容量、重试/receipt、lease、continuation、张量、所有权和 GC |
| 独立所有权模型 | 穷举两个队列、reader、checkpoint 与崩溃的有界交错；五种刻意不安全的协议（包括 checkpoint 回退）必须产生反例 |
| Native 故障注入 | 新 WAL 帧的每个字节前缀、每字节一次 XOR 修改；所有权/WAL 交接边界的进程突然退出 |
| 示例与 wheel 检查 | 不依赖源码导入或运行时 Rust 工具链的安装和公开 API 用法 |

有限模型探索的是协议抽象，不是 Rust 实现、任意依赖图或无界历史。它不是 TLA+/TLAPS refinement 或活性证明。故障注入覆盖前缀持久化和进程死亡；内核仍在运行。自有子进程退出后，私有临时目录会被清理，报告路径应选在该目录外。

## 多个文件系统客户端

准备共享源码和挂载，四台机器具有相同可用的包/解释器并配置 SSH：

```sh
python verification/crash_campaign.py \
  --hosts HOST_A HOST_B HOST_C HOST_D --parent /shared/test-tmp \
  --output /tmp/straw-four-client-crashes.json
```

在第一台机器上启动 launcher。测试程序把子进程崩溃/交接用例分配到四个客户端，在释放源之前检查恢复后的所有权。它使用共享验证脚本；运行已安装 wheel benchmark 的应用使用者不需要该源码树。

[多机 benchmark](BENCHMARKS_zh.md)补充并发打包 I/O、持久化接收、独立 reader 和在线回收。限制存储预算并使用新 root。检查返回字节数、退出状态和清理记录，不能只看吞吐或 launcher 的成功提示。

## 专项 GC 检查

`verification/gc_scale.py` 使用微小记录测量所有权管理成本。在每个客户端各启动一份，使用不同 queue 名称和同一个新 root：

```sh
python verification/gc_scale.py --root /shared/new-gc-scale \
  --queue HOST_A --count 1024 --participants 4
```

在 HOST_B/C/D 上同时运行对应命令。每个进程向自己的命名空间队列提交 1,024 个根，加入四客户端 barrier，再重新打开协调器、计时 GC、抽样读存活记录并验证无引用 pack 的删除。设为 `--count 16384` 可得到共 65,536 个根。计时排除协调器/catalog 重放，不测量张量带宽或冷存储。调用方在**四个进程全部停止后**负责清理。继续被中断的测试时，先确认旧 worker 已停止，再在同一队列上使用 `--resume`。

`verification/gc_cross_client.py` 专门验证远端删除后仍缓存正向文件属性的情形。在一个客户端准备唯一 root：

```sh
python verification/gc_cross_client.py --root /shared/new-gc-cache
```

随后四个客户端各使用不同 `--worker` 名称运行：

```sh
python verification/gc_cross_client.py --root /shared/new-gc-cache --worker HOST_A
```

每个客户端先预热 stat 缓存，写一个 `ready-<worker>` 标记。四个标记全部存在后，监督程序创建 `/shared/new-gc-cache/go`。Worker 并发执行 GC，必须全部成功退出，且 `reclaimed_files` 总和为一；之后才能删除私有 root。Barrier 的 60 秒超时限制失败准备阶段的等待。该测试只有一个 pack 和固定数量标记，不会每个 sample 创建一个文件。

## 如何理解失败

保存失败时的源码/wheel 哈希、配置、操作 ID 和最小复现。限制载荷规模，不删除仍被 reader 或状态不确定进程使用的存储。RPC 响应丢失仍可能意味着操作已提交，应通过稳定身份确定结果。完整帧的 checksum 错误必须报错，不能当成不完整尾部截掉。

作出更广泛的生产保证前，需要验证实际客户端/挂载缓存设置、分布式锁和同步语义、存储服务中断、设备/VM 掉电、ENOSPC/EIO，以及长时间保留/恢复。应用模型训练和优化器联合 checkpoint 还需要独立的端到端测试。
