# Binary wheels and source builds

[中文版](BUILDING_zh.md)

## Users: install a binary

Install `straw-queue` from PyPI. On a supported platform, pip selects a wheel
containing the Rust extension; installing that wheel does not need Cargo,
rustc, maturin or the source tree. The Python import name is `straw`.

```sh
pip install straw-queue
python -c 'import straw; from straw.tensor import TensorRef'
```

The release workflow targets Linux x86_64, CPython 3.10–3.13 and glibc 2.28+.
Other platforms need a source build with the toolchain described below.
NumPy/PyTorch remain runtime dependencies; install the CPU or CUDA PyTorch build
appropriate to your application first if needed.

For offline installation or testing a CI artifact, download the compatible wheel:

```sh
python -m pip install ./straw_queue-*.whl
```

Use a directory containing only the compatible wheel. For example, `cp312-cp312`
means CPython 3.12; it is not interchangeable with a CPython 3.10 wheel. The
filename's actual platform tag is authoritative. Version-tag pushes also publish
the verified artifacts to PyPI; other workflow runs only build and test them.

## Maintainers: build locally

Requires Rust 1.89+, a C linker/toolchain, and CPython 3.10+. Only the build
machine needs Rust. Build dependencies are declared in `pyproject.toml`, and
Cargo dependencies are pinned by `Cargo.lock`.

```sh
python -m pip install build maturin twine
python -m build --wheel --sdist
python -m twine check dist/*
```

`dist/` contains the `.whl` and source `.tar.gz`. A local build receives the
platform tag it actually satisfies; building on a new Linux distribution does
not automatically make an older-glibc wheel. Do not rename the tag manually.

For a portable Linux x86_64 wheel on a Linux build host, use Zig with maturin:

```sh
python -m pip install 'maturin[zig,patchelf]>=1.12,<2'
maturin build --release --locked --strip --zig \
  --compatibility manylinux_2_28 --out dist
```

Alternatively use the manylinux container build in
[the wheel workflow](../.github/workflows/wheels.yml). This follows
[maturin's distribution guidance](https://www.maturin.rs/distribution.html).
Wheels are interpreter-specific because the binding uses Python's buffer API;
the project does not claim one ABI3 wheel for every supported interpreter.

## Test the distributable artifact

Install compatible NumPy/PyTorch in the checking interpreter, then:

```sh
python tools/check_wheel.py dist/<the-compatible-wheel>.whl \
  --report /tmp/straw-wheel-check.json
```

Replace the filename with the actual artifact. This creates a temporary venv,
reuses the already installed runtime dependencies, installs **only the wheel**
without an index, and runs with Rust/maturin absent from its PATH. It rejects
source-tree imports and exercises records, tensors/COW, task/GC behavior,
multiprocess crash recovery, README examples, a local benchmark and trace
profiling. It checks that records pack together and payload roots are removed.

With pytest installed, add `--tests` to run the complete Python core suite,
independent model and native crash campaign against that installed wheel too.

Also check that the source archive is sufficient to rebuild:

```sh
python -m pip wheel --no-deps dist/straw_queue-*.tar.gz --wheel-dir rebuilt
```

That command runs on a build machine with Rust. Binary-only users never need
the source archive. Keep generated wheels, local experiment logs, datasets and
training outputs out of source control and source distributions.

Automatic Rust SBOM generation is disabled because it embeds absolute local
crate paths. `check_wheel.py` rejects generated JSON metadata containing local
crate path references. Rust dependency versions remain recorded in `Cargo.lock`.

## CI and release procedure

[Tests](../.github/workflows/tests.yml) run formatting, Clippy with and without
Python bindings, and Rust tests. A CPython 3.10–3.13 matrix builds and installs a
wheel, then runs all Python regressions, the independent ownership model, native
crash campaign, public examples, bounded I/O/GC benchmark and trace checks through
`tools/check_wheel.py --tests`. Results are uploaded as `test-report-py*` artifacts.
The workflow runs on pushes, pull requests and manual dispatch.

[Wheels](../.github/workflows/wheels.yml) builds manylinux 2.28 x86_64 wheels for
CPython 3.10–3.13 and runs the same installed-wheel checks on each. It also builds
a source archive, rebuilds a wheel from that archive, and tests the rebuilt
wheel. It runs on pull requests, manual dispatch and `v*` tags. Download wheels
from the `wheel-cp*-linux-x86_64` artifacts and the archive from
`source-distribution`; verification reports are uploaded alongside them.
On a `v*` tag push, a separate `publish` job waits for all wheel tests and the
source rebuild checks, verifies that package versions match the tag, and uploads
those same artifacts to PyPI. Pull requests and manual dispatch do not publish.

To launch either workflow manually, open the repository's **Actions** tab,
select **tests** or **wheels**, choose **Run workflow**, and select the branch.
GitHub exposes manual dispatch after the workflow exists on the default branch.
See [GitHub's workflow trigger documentation](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow).
No package-registry credentials are needed to build and download artifacts.

### Configure PyPI Trusted Publishing

Create a GitHub environment named `pypi` under the repository's **Settings →
Environments**. For the first release, add a pending publisher on your
[PyPI publishing page](https://pypi.org/manage/account/publishing/) with:

| Field | Value |
|---|---|
| PyPI project name | `straw-queue` |
| GitHub owner | `zhuzilin` |
| Repository name | `straw` |
| Workflow filename | `wheels.yml` |
| Environment name | `pypi` |

The workflow filename is only `wheels.yml`, without `.github/workflows/`. Commit
and push that file to the configured repository before releasing. The publishing
job uses GitHub OIDC with `id-token: write`; no PyPI API token or GitHub secret
is needed. See [PyPI's setup guide](https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/)
and [publishing guide](https://docs.pypi.org/trusted-publishers/using-a-publisher/).

After committing the release changes, ensure that `pyproject.toml`, `Cargo.toml`
and the `straw` entry in `Cargo.lock` share the release version. For example, for
version `0.1.2`, create and push a new tag on that commit:

```sh
git tag v0.1.2
git push origin v0.1.2
```

The first successful upload creates the PyPI project; registering a pending
publisher alone does not reserve the name. Later releases use a new matching
version and tag. An existing tag does not include later working-tree changes;
check its target before publishing and do not overwrite a published release.
If a build or test fails, fix it before publishing; failed jobs
prevent the publishing job from running.

Hosted CI uses local Linux filesystems and CPU PyTorch. Multi-client JuiceFS
qualification, storage-service faults and GPU training remain separate
[deployment/application checks](VERIFICATION.md). Inspect hosted results
separately; a local build is not proof that the matrix ran.

Before distributing a release, align the versions in `Cargo.toml`,
`pyproject.toml` and the `straw` entry in `Cargo.lock`, run the
[verification checks](VERIFICATION.md), inspect wheel
and sdist contents, run `check_wheel.py`, and preserve the wheel SHA-256 plus
the exact source revision. The changelog should state protocol/API compatibility
and unresolved limits. Upload the already tested artifact; rebuilding afterward
produces a different artifact that needs its own check.
