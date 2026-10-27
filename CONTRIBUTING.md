# Contributing

[中文版](CONTRIBUTING_zh.md)

Read the [design and spec](docs/DESIGN.md) before changing durable ordering,
recovery, ownership or authentication. Use the
[straw-protocol-review skill](.agents/skills/straw-protocol-review/SKILL.md)
to compare rules to TLA+, then trace the models to actual callers.

Protocol changes should explain success, uncertain effects, crash/retry and
compatibility. Add a reproducer for a real failure boundary and preserve a valid
success path. Update the rule, model, caller and regression together. Ordinary
local functions use compiler checks, Clippy and meaningful tests; do not rebuild
per-function proof scaffolding. Unknown wire versions fail explicitly; framing
changes require a format version.

When changing an environmental premise, update its stable ID in
[environment_contracts.json](verification/environment_contracts.json), the
[spec](docs/DESIGN.md#environment), consuming models and sensitivity controls.
Do not turn straw caller obligations into assumed filesystem guarantees.
User/contributor guides have paired English `.md` and Chinese `_zh.md` versions;
keep links, APIs and commands aligned. Application codecs and scheduling belong
in the consuming application. The project uses the [MIT license](LICENSE).

<a id="checks"></a>

## Development and checks

Use Rust 1.89+, a C linker, CPython 3.10+ and Java 17+ for TLC. Install the
appropriate CPU/CUDA PyTorch build, then the editable development package:

```sh
python -m pip install -e '.[test,build,dev]'
python -m black --check src tests examples tools verification
python -m isort --check-only src tests examples tools verification
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/runners/environment_manifest.py --check
python verification/runners/check_tla.py
python verification/runners/check_mutations.py
python verification/protocol/checks/ownership_model.py --output /tmp/straw-ownership.json
python verification/wal/checks/crash_campaign.py --output /tmp/straw-crashes.json
```

Rebuild/reinstall the native extension after Rust changes before running Python
tests. For focused model work, use `check_tla.py --area wal --area gc` or
`--case NAME`; run the full retained suite for release acceptance. Supply
`--jar /path/to/tla2tools.jar` to select TLC; otherwise the runner downloads its
configured release. It records the actual jar hash without enforcing a checksum.
Logs, configs and counterexamples default to `target/tla/`. Positives must pass;
negatives must fail the named invariant/temporal property. Parse errors, timeouts
and launch failures never count as semantic rejection.

The implementation mutation runner first passes an unmodified regression baseline,
then edits isolated Rust copies to reuse uncertain registry state, cache a directory
before its barriers, and skip GC parent barriers. Each must fail its named runtime
test; compilation failures, missing tests and timeouts do not count. Reports default
to `target/implementation-mutations/` and are retained by CI.

The inventory checks declared IDs, paths and environment imports. The independent
ownership model explores a bounded abstraction; the crash campaign checks byte
prefixes/corruption and process exits. None certifies storage-service or power-loss
behavior. Multi-client qualification is in [deployment](docs/FILESYSTEM.md#qualification).

<a id="build"></a>

## Build and check distributions

Only the build machine needs Rust. Dependencies are declared in `pyproject.toml`
and pinned by `Cargo.lock`:

```sh
python -m pip install build maturin twine
python tools/release_build.py python -m build --wheel --sdist
python -m twine check dist/*
python tools/check_artifact_privacy.py dist/*
python tools/check_wheel.py dist/<compatible-wheel>.whl --tests --report /tmp/straw-wheel-check.json
python tools/release_build.py python -m pip wheel --no-deps dist/straw_queue-*.tar.gz --wheel-dir rebuilt
```

Use the actual compatible wheel filename and check the rebuilt wheel too.
`check_wheel.py` creates a temporary environment, reuses installed NumPy/PyTorch,
installs only the wheel without an index, rejects source imports and removes Rust
from the test PATH. It runs examples, multiprocess recovery, benchmark/GC and
trace checks; `--tests` adds the full Python suite, ownership model and crash campaign.

The release wrapper removes local source/toolchain paths from Rust output.
Artifact scans check archive contents for recognizable paths/secrets without
printing their values. Rust SBOM generation is disabled because it embeds local
crate paths; locked dependency versions remain in `Cargo.lock`.

The release workflow targets Linux x86_64, CPython 3.10–3.13 and manylinux/glibc
2.28+. Wheels use interpreter-specific buffer APIs, not ABI3. Local builds receive
the platform tag they actually satisfy; never rename a tag to imply compatibility.
Use the [manylinux workflow](.github/workflows/wheels.yml), or on Linux:

```sh
python -m pip install 'maturin[zig,patchelf]>=1.12,<2'
python tools/release_build.py maturin build --release --locked --strip --zig \
  --compatibility manylinux_2_28 --out dist
```

<a id="release"></a>

## CI and releases

[Tests](.github/workflows/tests.yml) run all TLA cases in 16 disjoint shards,
formatting, both Clippy configurations, Rust tests and installed-wheel checks on
CPython 3.10–3.13. [Wheels](.github/workflows/wheels.yml) requires that same-commit
workflow, checks each portable wheel and a wheel rebuilt from the sdist, then
uploads artifacts. Failed, cancelled, skipped or timed-out checks block release.
Hosted CI uses a local Linux filesystem and CPU PyTorch; inspect hosted results
separately from local checks and distributed deployment tests.

For a release, align the versions in `Cargo.toml`, `pyproject.toml` and the
`straw` entry in `Cargo.lock`; record compatibility/limits in the changelog.
A matching new `v*` tag triggers publication of the already tested artifacts.
PRs and manual dispatch build/test without publishing. Keep the source revision
and wheel hash with release artifacts; rebuilding changes the artifact identity.
Do not overwrite an existing published version/tag.

The publish job uses GitHub OIDC with environment `pypi`. Configure PyPI's
publisher for project `straw-queue`, repository `zhuzilin/straw`, workflow
`wheels.yml`, environment `pypi`; the workflow filename has no directory prefix.
No PyPI token is required by this workflow. See the
[PyPI publisher setup](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
for service setup. Both workflows also support manual dispatch from Actions.

<a id="evidence"></a>

## Reports and public benchmarks

Keep per-run verification reports, attempted fixes, wheels, credentials, machine
addresses, payloads and raw logs in CI artifacts or ignored local directories.
Use fresh bounded test roots and clean them only after owned processes stop.

`verification/measurements/` contains only selected public benchmark data cited
by [benchmarks](docs/BENCHMARKS.md#published-results). A retained result needs
workload/method, build/probe identity, units, samples and limitations. Compare
payload sizes, publication granularity, fanout, durability and cache conditions;
do not imply an old build's numbers describe current code. Record one-off task
acceptance in the PR or CI result rather than adding another dated JSON file.
