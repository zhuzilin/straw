# Contributing

[中文版](CONTRIBUTING_zh.md)

Use Rust 1.89+ and CPython 3.10+. Install the PyTorch build appropriate to your
environment, then install the development package:

```sh
python -m pip install -e '.[test,build,dev]'
python -m black --check src tests examples tools verification
python -m isort --check-only src tests examples tools verification
cargo fmt --check
cargo clippy --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/runners/check_tla.py  # Java 17+
python verification/protocol/checks/ownership_model.py
python verification/wal/checks/crash_campaign.py --output /tmp/straw-crashes.json
```

See [building](docs/BUILDING.md) for binary wheels and
[verification](docs/VERIFICATION.md) for the scope of each check.
Keep machine addresses, credentials, training scripts, payloads and raw
experiment logs outside the project. Use a private temporary root for every
filesystem test, with bounded data and cleanup after owned processes stop.

Protocol changes must explain durable ordering, retry behavior, compatibility,
reader ownership and failure handling. Add a reproducer for a real failure
boundary, not just an assertion matching the implementation. Update the format
and independent model where their abstractions change. Unknown versions must
fail explicitly. Framing constants are part of the storage format; changing
them requires an explicit format version.

Custom application codecs and scheduling belong outside the Rust core. Prefer
clear references and explicit lifetime signals to application-specific imports.
Performance changes need comparable payload sizes, publication granularity,
reader fanout, filesystem/cache conditions and actual measured byte counts.

The project uses the [MIT license](LICENSE).

User and contributor guides have matching English `.md` and Chinese `_zh.md`
versions. Update both when behavior or examples change, preserve reciprocal
language links, and keep API names, commands and protocol constants identical.

[TLA+ / Verus specifications and instructions](docs/FORMAL_VERIFICATION.md) describe model assumptions,
code correspondence and required negative controls.

When changing an environment assumption, update its stable ID in
[`environment_contracts.json`](verification/environment_contracts.json), the
[environment review](docs/ENVIRONMENT_MODELS.md), its consuming models and
sensitivity controls. Keep algorithm policy, exploration bounds and measured
percentiles separate from required environment guarantees.
