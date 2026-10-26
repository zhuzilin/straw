# 验证目录

[English](README.md)

人先读[三条协议主线](../docs/DESIGN_zh.md)，agent 再按[两层审查映射](../docs/DESIGN_zh.md#review)核对文档 → TLA+ → 实现。

[审查 skill](../.agents/skills/straw-protocol-review/SKILL.md)提供具体流程；[贡献指南](../CONTRIBUTING_zh.md#checks)列出完整验收命令。目录按实现领域组织，不增加另一份协议正文。

| 目录 | 用途 |
|---|---|
| [protocol](protocol/README.md) | 跨日志、所有权、请求和恢复 |
| [wal](wal/README.md) | 日志碎片写入、恢复、调用者事务 |
| [gc](gc/README.md) | 发现、删除屏障、回执和 writer 退休 |
| [storage](storage/README.md) | 发布、认证读取、可见性和 namespace |
| [offline](offline/README.md) | 停止客户端后的根与别名审计 |
| [common](common/README.md) | 共享环境与性能实验 |

`runners/check_tla.py` 执行正例和必须因指定性质失败的语义负例。`environment_contracts.json` 登记环境前提、实现义务与模型消费者。工具错误或超时不算负例成功。

`measurements/` 只保留[性能文档](../docs/BENCHMARKS_zh.md#published-results)引用的三份公开基准。逐轮验收报告与原始日志保存在 CI artifacts 或忽略的 `target/`；历史过程记录从当前树删除，已提交版本仍可通过 Git 查询。收录标准见[贡献指南](../CONTRIBUTING_zh.md#evidence)。
