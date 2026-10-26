# Verification tools

[中文版](README_zh.md)

Run commands from the repository root. Top-level directories follow the
components being reviewed. Each feature keeps its models, proof modules, checks
and paired experiment drivers/shims together. Generated outputs belong in
`target/` or an external experiment archive.

| Location | Review scope |
|---|---|
| [`protocol/`](protocol/README.md) | Ownership, distributed commit/handoff, leases, reactivation and queue workflows |
| [`wal/`](wal/README.md) | Queue/catalog persistence, replay, read buffers and crash campaigns |
| [`gc/`](gc/README.md) | Discovery, deletion, directory barriers, receipts and conditional progress |
| [`storage/`](storage/README.md) | Publication, reads, visibility, typed layouts, paths and bandwidth |
| [`offline/`](offline/README.md) | Stopped-client inspection and orphan cleanup, including alias/control roots |
| [`common/`](common/README.md) | Shared environment modules and the combined Verus entry point |
| `runners/` | Pinned TLC/Verus runners and environment-contract inventory checks |
| `measurements/` | Historical, redacted summaries spanning multiple features |
| `environment_contracts.json` | Stable contract IDs, component layers and declared consumers |

```sh
python verification/runners/environment_manifest.py --check
python verification/runners/environment_manifest.py --list-layers
python verification/runners/environment_manifest.py --model OfflineAliases
python verification/runners/check_tla.py --jar /path/to/tla2tools.jar
python verification/runners/check_tla.py --area wal --area gc --jar /path/to/tla2tools.jar
python verification/runners/check_verus.py --verus /path/to/verus
```

These runners check pinned tool versions/hashes. See
[formal verification](../docs/FORMAL_VERIFICATION.md) for setup, evidence levels
and unproved boundaries, and [environment models](../docs/ENVIRONMENT_MODELS.md)
for contract review. Probe commands and deployment conditions are documented in
[filesystem measurements](../docs/FILESYSTEM.md). Cargo example names remain
unchanged, for example `cargo run --release --example positioned_extent_probe`.

Complete core implementation refinement remains open.
The [call-chain correspondence](../docs/VERIFICATION_BOUNDARIES.md) identifies
which production bodies have contracts and which caller obligations remain.

For 0.1.4, new formal-verification work is frozen and the remaining proof
obligations are deferred. Continue running the integrated checks above;
regression and fault-injection tests cover the remaining paths for this release
without completing their proofs. See the [release scope matrix](../docs/FORMAL_VERIFICATION.md#verification-scope-for-this-release).

The feature layout supersedes the earlier directories grouped only by tool.
There are no duplicate compatibility copies. TLA module names remain globally
unique; the runner assembles the declared environment dependencies for each
case. CI, tests, package manifests and current documentation use the feature
paths. Historical summaries preserve original input paths and hashes.

Dated measurement JSON is a public, redacted summary: internal host identifiers
and private artifact locations are removed. Embedded hashes identify original
execution inputs; the public JSON is not the original proof artifact. Reproduce
current checks with the runners above; historical source snapshots may differ. Old probes and negative controls remain useful
for reproducing recorded failures even when they are not in the default test
suite. Remove a source only after checking its tests, callers, paired fault shim,
environment evidence and measurement purpose. Python caches and generated logs
are disposable and excluded from the source distribution.

The Verus runner also builds the real Cargo.lock dependency graph with Rust 1.99.1. Its separately audited serde_json specification allowlist contains twenty-one primitive method specifications, four opaque types, the transparent actual Value enum and Value-only encoding/observation laws, four hash-pinned canonical-content laws and one Map key-uniqueness law; actual catalog JSON wire decoding is verified against them. No whole straw decoder or business result is assumed. See `ENV-JSON-LIBRARY` and `verification/common/verus/serde_json.rs`.

The actual field-to-operation constructor also uses four separately audited standard HashMap/HashSet consuming-iterator primitives and two opaque types; queue replay additionally uses the standard mem::replace contract (`ENV-COLLECTION-ITERATOR`). Native namespace and bulk-set adapters retain explicit conformance obligations (`ENV-CATALOG-RESOLVER`, `ENV-SET-BUILDER`); this is not an assumed complete decoder.
