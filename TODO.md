# straw 剩余形式化验证

0.1.4 冻结新增 verification，以下端到端证明延期。继续运行已有 TLA+、Verus、回归、故障注入和恢复检查；测试通过不等于完成证明，也不代表这些缺口已有生产故障。

- [ ] **分布式协议与跨日志组合**：连接实际 retain、queue acceptance、状态应用、staging 代际回执和恢复；覆盖请求重试、旧 attempt／epoch、checkpoint 回退及所需数据保护，建立完整调用链的不变量。
- [ ] **完整 WAL 原生调用链**：在已验证 framing、File 适配和 Journal 所有者之外，闭合全部公开调用者的锁／独占资格、callback、派生缓存、部分失败隔离与重新恢复对应；保持同一日志身份和已确认前缀。
- [ ] **retain／reactivation、缓存与 GC 组合**：绑定真实认证依赖图、pool／run／namespace 缓存上下文、既有 owner 的根身份，以及锁外认证后的锁内重查；将完整根保护连接到已验证的 mark／删除／持久回执链。
- [ ] **完整 extent 认证读取**：组合真实索引、frame、envelope、payload、摘要和读缓存，覆盖文件长度与各区域独立可见、重开重试和持续损坏；环境契约不能替代 straw 的调用者义务。
- [ ] **崩溃 writer 的所有权闭环**：组合 gate／fence、在途 I/O、持久操作登记、结果认证、恢复 staging、sealing 与 GC，覆盖 recover／abandon 和报错但已生效的重试。
- [ ] **真实重试与条件活性**：将实际执行历史连接到公平调度、锁／I/O 最终返回和调用者重试前提；区分永久无效输入与可恢复错误，并从 sealed 且永久无主的对象证明最终发现、删除和回执完成。

文件系统、标准库／密码学原语、外部 fencing 和应用释放 owner 的假设继续单列；不通过扩大环境保证来关闭上述义务。
