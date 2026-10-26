# Documentation

[中文版](README_zh.md)

Start with the [project README](../README.md), then follow the task you need:

| Need | Guide |
|---|---|
| Understand why straw exists and where it fits | [Design](DESIGN.md) |
| Write/read records, tensors and task results; manage their lifetimes | [Usage](USAGE.md) |
| Qualify a shared filesystem and plan storage | [Filesystem](FILESYSTEM.md) |
| Understand WAL, ownership, recovery and GC | [Protocol](PROTOCOL_AND_VERIFICATION.md) |
| Run model exploration, crash campaigns and regression tests | [Verification](VERIFICATION.md) |
| Review actual Rust callers, proved bodies and open refinement obligations | [Implementation/proof correspondence](VERIFICATION_BOUNDARIES.md) |
| Measure local/shared-filesystem performance and replay a workload | [Benchmarks](BENCHMARKS.md) |
| Build and install binary distributions | [Building](BUILDING.md) |
| Integrate a scheduler or training system | [Applications](APPLICATIONS.md) |
| Implement a compatible reader or change the format | [Format](FORMAT.md) |
| See what each release includes | [Changelog](../CHANGELOG.md) |

For concurrent-read and ownership specifications and counterexample replay, see
[TLA+ / Verus formal verification](FORMAL_VERIFICATION.md).

Review filesystem responses, faults, peers and progress premises separately in
[Environment models](ENVIRONMENT_MODELS.md).

Find proof runners, models and experiment sources in the [verification directory guide](../verification/README.md).
