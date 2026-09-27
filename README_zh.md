# straw

[English](README.md)

**面向 AI 应用、基于文件系统的持久化队列与共享张量存储。**

straw **以共享文件系统作为存储层**：多台机器上的 worker 通过同一共享挂载并行读写打包后的数据，彼此传递小型引用。存储与队列协议由 Rust 实现，Python API 支持 bytes、NumPy 数组和 PyTorch 张量。

**当前实现和多机验证以 [JuiceFS](https://juicefs.com/) 共享挂载为主**，通过 Linux/POSIX 文件系统接口访问。本地文件系统用于开发和单机测试。未来计划扩展到包括 NFS 在内的更多网络文件系统，逐一验证其跨客户端锁、数据可见性和持久化语义。

straw 目前服务于 **[slime](https://github.com/THUDM/slime)** 的 rollout 与训练流水线，包括共享底层张量的 rollout → rollout 和 rollout → train 队列。它是独立的 AI 应用库，也适用于数据预处理和推理流水线。详见[设计原理](docs/DESIGN_zh.md)和[应用集成](docs/APPLICATIONS_zh.md)。

## 多机并行读写

多个 producer 可以并行追加各自的 pack，其他机器上的 reader 通过引用读取已提交的不可变数据区间。协议保证先持久化载荷、再发布引用，用日志记录任务归属、已接收结果和消费者进度，并通过跨客户端 catalog 锁协调共享所有权与 GC。Reader pin 和显式读取完成信号保护数据，直到所有消费者使用完毕。详见[并发规则](docs/USAGE_zh.md#进程与线程并发)和[协议](docs/PROTOCOL_AND_VERIFICATION_zh.md)。

## 避免大量小文件的性能瓶颈

每个 sample 或张量创建一个文件，会给共享文件系统带来大量元数据操作，尤其容易影响 NFS 性能。straw 将多条记录及其 manifest 合并到大的追加式 pack 文件中，支持批量发布，队列元数据使用固定的追加日志。**不会为每个 sample 或每个张量单独创建文件。** 文件数随 pack 数量和 writer 生命周期增长，而非随 sample 数量逐条增长；通过[打包与 GC](#通过打包控制文件数)管理其生命周期。

**状态：**早期版本，支持 Linux/POSIX。每个队列只能有一个由外部机制隔离保护的协调器；尚无自动选主、日志压缩或存活 pack 整理。回收之前，应用必须提供真实的读取完成信号。详见[文件系统要求](docs/FILESYSTEM_zh.md)与[协议保证](docs/PROTOCOL_AND_VERIFICATION_zh.md)。

## 安装

从 PyPI 安装：

```sh
pip install straw-queue
```

pip 会选择包含 Rust 扩展的兼容 wheel，**安装 wheel 无需 Rust 或 maturin**。NumPy 和 PyTorch 是运行时依赖，可以复用已有的兼容 PyTorch。发行包名为 `straw-queue`，导入名为 `straw`。[支持的平台、离线安装及源码构建](docs/BUILDING_zh.md)。

## 写入、读取、保留、回收

以下完整示例使用私有临时目录。实际任务应在共享挂载上创建新目录，所有客户端使用相同 run ID。

```python
from tempfile import TemporaryDirectory
from straw import Record, SharedFilesystemStore

with TemporaryDirectory() as root:
    with SharedFilesystemStore(root, "example", online_gc=True) as store:
        ref = store.publish([Record("message", b"hello")], submission_id="write-1")
        store.retain("application:checkpoint-1", [ref])
        store.release_publications([ref])  # The checkpoint now owns this data.
        assert next(store.read(ref)).payload == b"hello"

        store.seal()  # Stop appending to this pack before it can be collected.
        assert store.collect_garbage()["reclaimed_files"] == 0
        store.release("application:checkpoint-1")
        assert store.collect_garbage()["reclaimed_files"] == 1
```

返回的引用是地址，不代表数据会被永久保留。刚发布的数据由 publication staging（发布暂存所有权）保护；使用期间必须有队列、checkpoint、显式 owner 或 reader pin 保护它。GC 只删除**已经封存且没有任何所有者的 pack**；关闭 writer 不会释放应用所有权。详见[数据与生命周期 API](docs/USAGE_zh.md)。

## 可以存什么？

| 数据 | 写入 | 读取 |
|---|---|---|
| 原始字节、编码后的文本、JSON、图片或应用自定义格式 | `Record` + `store.publish` | `store.read` / `read_record`，再由应用解码 |
| NumPy 数组和 PyTorch 张量 | `publish_tensors` | `TensorRef.load()` 或连续行切片 |
| 组成一个结果的多条记录或多个张量 | `publish` / `publish_many` | 一个 `RecordSetRef` 引用有序记录集 |
| 多个队列共享的不可变数据 | 依赖关系与 `TensorRef.share` | 每个队列保留同一批底层 pack |
| 修改后的张量 | `TensorRef.updated` | 返回新张量，原张量保持不变 |

straw 不会对存储的载荷执行 unpickle。Codec 名称显式指定并带版本，自定义对象由应用负责序列化。张量辅助接口会将设备数据复制为连续 CPU 存储，支持带校验的按行读取；它不提供 GPU IPC 或内存页级写时复制。详见[类型与示例](docs/USAGE_zh.md#张量)。

应用可以使用多进程、多机或多线程。straw 提供同步 Rust 接口，本身不会创建 I/O 进程池。详见[并发与 writer 归属](docs/USAGE_zh.md#进程与线程并发)。

## 通过打包控制文件数

每个 writer 进程复用 store。默认目标大小为 1 GiB 的 pack 容纳多个 publication，manifest 也写在 pack 内。队列元数据使用固定的追加日志，GC 使用一个共享 catalog。**不会为每个 sample 或每个张量单独创建文件。**

按字节数轮换 pack，用 `publish_many` 合并小写入，释放已经消费的数据，并封存空闲 writer。只要 pack 中还有一条存活记录，整个 pack 都会保留。反复重启 writer、每个 sample 都 `close()`/`seal()`、永久保留所有 checkpoint，仍会使存储增长；目标 pack 大小不是全局配额。详见[文件数与保留策略](docs/FILESYSTEM_zh.md#文件数与空间规划)。

## 运行小示例

安装 wheel 后，可以直接运行源码树中的示例：

```sh
python examples/records.py
python examples/tensors.py
python examples/work_queue.py
python examples/multiprocess.py --root /tmp/new-straw-recovery-run
```

不依赖 SSH 或 GPU 的有界本地 benchmark：

```sh
python -m straw.benchmark run --local \
  --root /tmp/new-straw-benchmark --report benchmark-results/local.json \
  --gib 0.0625 --record-bytes 1048576 262144 \
  --segment-mib 8 --online-gc --max-files 64 --max-gib 0.25
```

Benchmark 会校验读取结果，测量持久化发布与接收延迟，报告字节数和文件数，并在 worker 停止后删除临时载荷目录。详见[多机 benchmark 与轨迹重放](docs/BENCHMARKS_zh.md)。

## 后续阅读

- [设计原理与架构](docs/DESIGN_zh.md)
- [记录、张量、任务、恢复与 GC](docs/USAGE_zh.md)
- [文件系统部署与文件数规划](docs/FILESYSTEM_zh.md)
- [协议、参考工作与正确性边界](docs/PROTOCOL_AND_VERIFICATION_zh.md)
- [可执行验证](docs/VERIFICATION_zh.md)与[二进制格式](docs/FORMAT_zh.md)
- [构建 wheel 与发布检查](docs/BUILDING_zh.md)
- [更新日志](CHANGELOG_zh.md)
- [应用集成](docs/APPLICATIONS_zh.md)与[贡献指南](CONTRIBUTING_zh.md)

## GitHub 构建与测试

- [全套测试](.github/workflows/tests.yml)：Rust 测试、格式检查和 Clippy，以及 CPython 3.10–3.13 下针对已安装 wheel 的全部 Python 测试、所有权模型、故障注入、示例和有界 I/O/GC benchmark。
- [Wheel 构建](.github/workflows/wheels.yml)：构建并验证 CPython 3.10–3.13 的 Linux x86_64 wheel（manylinux/glibc 2.28+），同时生成源码包并检查能否重新构建。从工作流运行页面的 **Artifacts** 下载产物。推送版本 tag 时，通过 Trusted Publishing 将已验证产物发布到 PyPI。

两条工作流都可以通过 **Actions → 选择工作流 → Run workflow** 手动触发。全套测试还会在 push 和 pull request 时运行；wheel 构建会在 pull request 和 `v*` tag 时运行。托管 CI 使用本地文件系统和 CPU PyTorch；多客户端 JuiceFS 部署验证与 GPU 训练使用[独立检查](docs/VERIFICATION_zh.md)。详见[构建与发布说明](docs/BUILDING_zh.md)。

## 许可证

[MIT](LICENSE)。
