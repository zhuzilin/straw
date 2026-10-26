# 验证工具

[English](README.md)

以下命令从仓库根目录运行。顶层按要审查的功能组织；每个功能内放对应模型、证明模块、自动检查、探针及配套故障注入程序。生成产物放在 `target/` 或外部实验归档中。

核心实现的完整细化仍未完成。[调用链对应表](../docs/VERIFICATION_BOUNDARIES_zh.md)区分实际函数体证明、调用者义务和外部可信保证。

0.1.4 冻结新增形式化验证，剩余证明义务延期。下列已集成检查继续执行；本次发布以回归和故障注入测试覆盖未闭合路径，测试通过不等于完成证明。见[本次发布范围矩阵](../docs/FORMAL_VERIFICATION_zh.md#本次发布的验证范围)。

| 位置 | 审查范围 |
|---|---|
| [`protocol/`](protocol/README.md) | 所有权、分布式提交／交接、lease、重新激活及队列工作流 |
| [`wal/`](wal/README.md) | 队列／catalog 持久化、replay、预读及崩溃实验 |
| [`gc/`](gc/README.md) | 发现、删除、目录屏障、回执及条件进展 |
| [`storage/`](storage/README.md) | 发布、读取、可见性、typed layout、路径及带宽 |
| [`offline/`](offline/README.md) | 停机检查与孤儿清理，包括别名与控制元数据根 |
| [`common/`](common/README.md) | 跨功能共用的环境模块与统一 Verus 入口 |
| `runners/` | 固定版本的 TLC／Verus 运行器及环境契约清单检查 |
| `measurements/` | 跨功能的历史脱敏证据摘要 |
| `environment_contracts.json` | 稳定契约编号、组件层和声明的消费者 |

```sh
python verification/runners/environment_manifest.py --check
python verification/runners/environment_manifest.py --list-layers
python verification/runners/environment_manifest.py --model OfflineAliases
python verification/runners/check_tla.py --jar /path/to/tla2tools.jar
python verification/runners/check_tla.py --area wal --area gc --jar /path/to/tla2tools.jar
python verification/runners/check_verus.py --verus /path/to/verus
```

Verus 运行器核对固定的工具版本／哈希；TLC 只记录 jar 哈希，不做校验。环境准备、证据层级和未证明边界见[形式化验证](../docs/FORMAL_VERIFICATION_zh.md)，契约审查见[环境模型](../docs/ENVIRONMENT_MODELS_zh.md)，部署条件与探针命令见[文件系统测量](../docs/FILESYSTEM_zh.md)。Cargo example 名称不变，例如 `cargo run --release --example positioned_extent_probe`。

按功能的布局取代了此前仅按工具划分的目录，不保留重复兼容副本。TLA 模块名在全库内保持唯一，运行器为每个案例装配声明的环境依赖。CI、测试、打包清单和当前文档使用功能路径；历史摘要保留原运行输入的路径与哈希。

Verus 日志把正常代码标为 `positive: proof required`，必须零错误通过；`negative: expected rejection` 表示故意破坏代码或环境契约的副本，此处 PASS 说明 Verus 按预期以语义证明错误拒绝了该副本。负例意外验证成功、编译／工具失败、超时或资源耗尽都会使检查失败。

历史测量 JSON 是公开脱敏摘要：内部主机标识和私有工件位置已删除。保留的哈希标识当时运行输入，公开 JSON 本身不是原始证明工件。当前检查可用上面的运行器复现；历史源码快照可能不同。旧探针与负例即使不在默认测试集合中，仍用于复现记录过的故障。判断源码是否无用时，需要检查测试、调用者、配套故障 shim、环境证据及测量用途。Python 缓存和生成日志可以丢弃，不进入源码包。

Verus 运行器还会使用 Rust 1.99.1 编译真实 Cargo.lock 依赖。serde_json 独立可信白名单只含二十一个基础方法规范、四个不透明类型、透明的实际 Value 枚举及仅用于 Value 的编码／观察定律和四条独立固定哈希的 canonical-content 库定律和一个 Map key 唯一性定律，实际 catalog JSON 字段解码相对于这些契约验证，不假设整个 straw 解码器或业务结果。审查入口为 `ENV-JSON-LIBRARY` 和 `verification/common/verus/serde_json.rs`。

实际字段到操作构造还使用独立审计的四个标准 HashMap/HashSet 消费迭代原语和两个不透明类型（`ENV-COLLECTION-ITERATOR`）。原生命名空间与批量集合适配保留明确对应义务（`ENV-CATALOG-RESOLVER`、`ENV-SET-BUILDER`），没有假设完整 decoder 正确。
