# Verification directory

[中文版](README_zh.md)

Humans read the [three protocol flows](../docs/DESIGN.md); agents use the [two review maps](../docs/DESIGN.md#review) to compare document → TLA+ → implementation.

The [review skill](../.agents/skills/straw-protocol-review/SKILL.md) supplies the workflow; [contributor checks](../CONTRIBUTING.md#checks) list acceptance commands. Directories group implementation areas without defining another protocol.

| Directory | Purpose |
|---|---|
| [protocol](protocol/README.md) | Cross-log handoff, ownership, requests and recovery |
| [wal](wal/README.md) | Partial writes, recovery and caller transactions |
| [gc](gc/README.md) | Discovery, deletion barriers, receipts and writer retirement |
| [storage](storage/README.md) | Publication, authenticated reads, visibility and namespaces |
| [offline](offline/README.md) | Stopped-client root and alias audits |
| [common](common/README.md) | Shared environments and performance probes |

`runners/check_tla.py` runs positives and semantic negatives that must fail the named property. `environment_contracts.json` records environmental premises, implementation obligations and model consumers. Tool errors or timeouts never count as semantic rejection.

`measurements/` retains only the three public benchmarks cited in [benchmarks](../docs/BENCHMARKS.md#published-results). Per-run acceptance reports and raw logs belong in CI artifacts or ignored `target/`. Historical process records are removed from the current tree; committed versions remain in Git history. See the [retention policy](../CONTRIBUTING.md#evidence).
