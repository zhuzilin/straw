# 文档

[English](README.md)

从[项目 README](../README_zh.md)开始，再按需求选择：

| 需求 | 指南 |
|---|---|
| 了解为什么需要 straw、适用于哪里 | [设计原理](DESIGN_zh.md) |
| 读写记录、张量与任务结果，管理生命周期 | [使用方法](USAGE_zh.md) |
| 验证共享文件系统、规划存储 | [文件系统](FILESYSTEM_zh.md) |
| 理解 WAL、所有权、恢复和 GC | [协议](PROTOCOL_AND_VERIFICATION_zh.md) |
| 运行模型探索、故障注入和回归测试 | [验证](VERIFICATION_zh.md) |
| 审查实际 Rust 调用者、已证明函数体和未完成细化 | [实现与证明对应表](VERIFICATION_BOUNDARIES_zh.md) |
| 测量本地/共享文件系统性能，重放应用负载 | [Benchmark](BENCHMARKS_zh.md) |
| 构建、安装二进制发行包 | [构建](BUILDING_zh.md) |
| 集成调度器或训练系统 | [应用集成](APPLICATIONS_zh.md) |
| 实现兼容 reader 或修改格式 | [格式](FORMAT_zh.md) |
| 查看各版本包含的内容 | [更新日志](../CHANGELOG_zh.md) |

检查并发读取与所有权规范、重放反例：见 [TLA+ / Verus 形式化验证](FORMAL_VERIFICATION_zh.md)。

单独审查文件系统响应、故障、其他客户端和进展前提：见[环境模型](ENVIRONMENT_MODELS_zh.md)。

证明运行器、模型和实验源码的位置见[验证目录说明](../verification/README_zh.md)。
