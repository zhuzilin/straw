# 本版本验收

- [x] 移除 Verus 与局部证明设施，保留协议模型和产品回归。
- [x] 补齐跨日志、WAL 调用者、reactivation、认证读取、writer 退休及条件进展场景；修复并回归 owner 根匹配问题。
- [x] 文档合并为[设计与 spec](docs/DESIGN_zh.md)，包含环境前提、三条协议主线及模型／实现对应。
- [x] 应用集成并入[使用指南](docs/USAGE_zh.md#integration)，检查／构建／发布并入[贡献指南](CONTRIBUTING_zh.md)。
- [x] 删除过程测量与重复文档，只保留公开基准；同步[审查 skill](.agents/skills/straw-protocol-review/SKILL.md)。

当前无待办。验收结果随对应 PR／CI 产物保存；有限模型与执行回归不构成完整实现等价证明。后续协议改动按[检查流程](CONTRIBUTING_zh.md#checks)重新核对当前输入。
