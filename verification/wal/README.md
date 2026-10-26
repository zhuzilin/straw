# WAL 与恢复 / WAL and recovery

[总入口 / Index](../README.md)

队列及 catalog 的追加、持久化前缀、replay、预读和崩溃恢复。 / Queue and catalog append, durable prefixes, replay, read-ahead and crash recovery.

模型与配置在 `tla/`，部署实验与配套注入程序在 `probes/`。如有 `checks/` 或 `verus/`，分别放可执行检查和数学证明。生产 Rust 的证明合约仍随实现在仓库 `rust/` 中。

Models/configurations live in `tla/`, deployment experiments and their shims in `probes/`. Optional `checks/` and `verus/` contain executable checks and mathematical proofs. Production Rust proof contracts remain with their implementations in the repository `rust/` directory.

从仓库根目录运行 / Run from the repository root:

```sh
python verification/runners/check_tla.py --area wal
python verification/runners/environment_manifest.py --model CatalogReadBuffer
```

不加 `--area` 运行完整模型集合。完整 Rust 证明共用 `verification/runners/check_verus.py`。 / Omit `--area` for the complete model suite. All Rust proofs use the shared Verus runner.

| Model | Configuration |
|---|---|
| [CatalogReadBuffer](tla/CatalogReadBuffer.tla) | [CatalogReadBuffer.cfg](tla/CatalogReadBuffer.cfg) |
| [CatalogReplay](tla/CatalogReplay.tla) | [CatalogReplay.cfg](tla/CatalogReplay.cfg) |
| [CatalogTransaction](tla/CatalogTransaction.tla) | [CatalogTransaction.cfg](tla/CatalogTransaction.cfg) |
| [WalRecovery](tla/WalRecovery.tla) | [WalRecovery.cfg](tla/WalRecovery.cfg) |

证明边界和待补项见 [FORMAL_VERIFICATION](../../docs/FORMAL_VERIFICATION_zh.md)。 / See [formal verification](../../docs/FORMAL_VERIFICATION.md) for proof boundaries and outstanding work.

实际 Rust 合约覆盖 [framing.rs](../../rust/framing.rs) 的字节布局、[wal_recovery.rs](../../rust/wal_recovery.rs) 的共享字节执行函数、[catalog_state.rs](../../rust/catalog_state.rs) 的实际索引与顺序 apply，以及 [replay.rs](../../rust/replay.rs) 的恢复 guard。[catalog_json.rs](../../rust/catalog_json.rs) 已验证真实 JSON 字段解码；具体 Device/Target、路径到业务操作、完整事务与上层协议的细化继续单列，见[调用链对应表](../../docs/VERIFICATION_BOUNDARIES_zh.md)。

Production contracts cover framing, the shared byte replay driver, actual catalog indexes/ordered apply and recovery guards. The actual catalog JSON wire decoder is also checked under audited library primitives. Concrete Device/Target, paths/business updates and complete transaction/protocol refinement remain separate obligations; see the [caller correspondence](../../docs/VERIFICATION_BOUNDARIES.md).

`CatalogTransaction` 另检查永久无效事件在 WAL 效果前被拒绝、事务仍可用；实际 `Indexes::validate` 与 `apply` 证明相同准入条件。解码／语义结果到模型 `ValidEvent` 的绑定仍是调用者义务。

`CatalogTransaction` also checks side-effect-free rejection of invalid events. Actual index preflight and apply prove identical acceptance; binding decoded/semantic validity to the modeled `ValidEvent` remains a caller obligation.

[catalog_log.rs](../../rust/catalog_log.rs) 统一实际索引、游标、序号、恢复状态和待完成追加。其 Verus 不变量覆盖任意有限有类型 frame 历史、部分失败前缀和 rebuild；可擦除 ghost 历史不产生运行期开销。原始 payload 已绑定有类型历史，Apply::recover 连接实际重建／同步／确认；业务解释、具体 Source 和外层锁／open 仍是独立义务。

[wal_append.rs](../../rust/wal_append.rs) 是两个日志实际使用的字节追加循环；Verus 在 `ENV-APPEND-IO` 下证明写入的新 frame 满足恢复器的判定，并由 `LogState::append_to` 连接到 catalog 确认。原生 Appender 仍需证明其 fd／原语结果对应，见[追加证明边界](../../docs/FORMAL_VERIFICATION_zh.md#实际-wal-追加与-catalog-确认)。

`wal_recovery` 的实际格式错误分支有精确合约；合法帧不能被错误拒绝，成功恢复合法有限日志必须得到唯一、有序且完整的 payload 历史。Device 错误／业务 apply 错误与格式错误分开，完整损坏 frame 不作为残尾截断。见[恢复完备性证明](../../docs/FORMAL_VERIFICATION_zh.md)与[阶段证据](../measurements/20261025-replay-completeness.json)。

Actual replay format-error branches have exact contracts. Valid frames cannot be rejected as malformed, and successful recovery of a valid finite log reconstructs its unique, complete ordered payload history. Device and business-application errors remain separate; concrete I/O and payload-to-operation correspondence are independent obligations.

[catalog_target.rs](../../rust/catalog_target.rs) 实际实现 catalog 的原始 Target 历史协议：LogState 的 ghost 历史绑定精确字节，成功时由同一有类型 apply 推进。decoder 语义、Journal Target 与完整物理调用链仍单列。 / The actual catalog Target projects exact bytes from LogState history and uses the proved typed apply; decoder meaning, Journal Target and complete physical call composition remain separate obligations.

[catalog_event.rs](../../rust/catalog_event.rs) 验证实际 handoff／staging 名称、receipt release-only 条件及图名称／兼容检查。 / Actual semantic predicates prove exact identifier syntax, release-only receipts and staging-graph admission; full path/operation conversion remains separate.

[catalog_decode.rs](../../rust/catalog_decode.rs) 将实际字段解码和语义准入连接到完整有序操作，路径及批量集合适配单列契约。 / Actual field decoding and semantic admission compose into ordered operations; native namespace/set conformance, raw parsing/run binding and full transaction refinement remain open.

`catalog_target::Apply::recover` 进一步连接真实 rebuild／字节 replay／同步／确认，成功时历史、cursor、sequence 和持久边界一致；错误不推进确认下限。 / The actual catalog recovery composition proves matching history/cursor/sequence/durable boundaries on success and preserves the old confirmation floor on errors. Native I/O, cache meaning and complete business/lock composition remain open; see the [record](../measurements/20261025-catalog-recovery.json).

[catalog_append](../../rust/catalog_append.rs) verifies the actual input-field/run binding, serialization and append/history composition under a narrow external Value encoding relation. `tests/test_json_roundtrip.py` and the native journal regression cover numeric preservation through recovery. Native adapters remain open. [catalog_equivalence](verus/catalog_equivalence.rs) and [catalog_json_equivalence](verus/catalog_json_equivalence.rs) prove identical completed indexes/cursor/sequence for the same raw prefix under explicit Value observation and String content-key premises. / 实际追加组合已经连接输入字段、run、序列化与历史；相同原始前缀的完整业务状态等价已经在显式库／String key 前提下证明，原生适配仍未闭合。

实际 queue 事件校验／编码／持久帧组合见 [journal_append](../../rust/journal_append.rs)，帧解码及有限历史观察对应见 [journal_equivalence](verus/journal_equivalence.rs)。 / Actual event-to-durable-frame binding and successful replay observations are proved; native File/run identity, physical prefix selection and complete business replay remain open.

实际 [wal_source](../../rust/wal_source.rs) 与 [wal_sink](../../rust/wal_sink.rs) 已将缓冲／句柄／失败隔离及追加／同步流程连接到 Device／Writer；File／Buffer／Barrier／Hooks 基础契约和原生快照／锁绑定仍单列。零字节读取在 EOF 后成功的规格已修正。 / Actual descriptor dispatch now proves the higher interfaces under explicit file/buffer/barrier/hook contracts; native primitive and outer-context correspondence remain open. [确切证据 / evidence](../measurements/20261025-file-adapters.json).

`journal_equivalence::physical_prefix_business` composes the actual `State::recover` and `State::apply_business` postconditions with `queue_history::frames_fold`: the successful business result follows the actual decoded byte-prefix values, excluding the identity header. It does not presume independent parsing has identical numeric representation. Native run/File/lock/cache correspondence remains separate. [Evidence](../measurements/20261026-queue-history.json).

[journal_io](../../rust/journal_io.rs) connects the production Journal recovery, append and root-retirement calls to the original borrowed File and permission, including constructor failures and all returning error paths. Successful calls bind the same physical cursor/durable prefix to the recovered or appended state. Initial open/run/lock authority and callback primitives remain separate; see the [fixed chain evidence](../measurements/20261026-journal-io.json). / 实际 Journal 三条调用链已将原始文件、借用归还与成功持久边界连接；初始打开／run／锁授权及 callback 原语仍单列。
