# straw

[中文版](README_zh.md)

**Filesystem-based durable queues and shared tensor storage for AI applications.**

straw uses a **shared filesystem as its storage layer**: workers on multiple
machines read and write packed data through the same mounted filesystem and
exchange small references. Storage and queue protocols run in Rust; Python APIs
accept bytes, NumPy arrays and PyTorch tensors.

The current implementation and multi-machine validation focus on **[JuiceFS](https://juicefs.com/)**
shared mounts, accessed through Linux/POSIX filesystem operations. Local
filesystems are also used for development and single-machine tests. Support for
additional network filesystems, including NFS deployments, is planned; each
needs validation of its locking, visibility and durability semantics.

straw currently serves the rollout and training pipelines in
**[slime](https://github.com/THUDM/slime)**, including rollout-to-rollout and
rollout-to-training queues that share tensor storage. It is an independent AI
application library, also suited to preprocessing and inference pipelines. See
the [design rationale](docs/DESIGN.md) and [application integration](docs/APPLICATIONS.md).

## Multi-machine parallel reads and writes

Multiple producers append concurrently to their own packs; readers on other
machines access committed immutable extents by reference. The protocol makes
payloads durable before publishing references, journals task ownership, accepted
results and consumer progress, and coordinates shared ownership and GC through
cross-client catalog locks. Reader pins and explicit completion signals protect
data until every consumer has finished. See the
[concurrency rules](docs/USAGE.md#concurrency-processes-and-threads) and
[protocol](docs/PROTOCOL_AND_VERIFICATION.md).

## Avoid the small-file bottleneck

Creating a file for every sample or tensor puts pressure on shared-filesystem
metadata operations, especially on NFS. straw packs many records and their
manifests into large append-only files, batches publication, and uses fixed
append logs for queue metadata. **There is no file per sample or tensor.**
File counts grow with packs and writer lifetimes instead of individual samples;
[packing and GC](#keep-file-counts-bounded-by-packs) control their lifecycle.

**Status:** early release, Linux/POSIX. One externally fenced coordinator per
queue; no automatic leader election or journal/live-pack compaction. Applications
must provide real reader-completion signals before reclamation. See
[filesystem requirements](docs/FILESYSTEM.md) and [protocol guarantees](docs/PROTOCOL_AND_VERIFICATION.md).

## Install

Install from PyPI:

```sh
pip install straw-queue
```

pip selects a compatible wheel containing the Rust extension.
**Installing a wheel does not require Rust or maturin.**
NumPy and PyTorch are runtime dependencies; an existing compatible PyTorch
installation can be reused. Distribution name: `straw-queue`; import: `straw`.
See [supported platforms, offline installation and source builds](docs/BUILDING.md).

## Write, read, keep, reclaim

This complete example uses a private temporary directory. Use a new directory
on your shared mount for a real job, with the same run ID on every client.

```python
from tempfile import TemporaryDirectory
from straw import Record, SharedFilesystemStore

with TemporaryDirectory() as root:
    with SharedFilesystemStore(root, "example", online_gc=True) as store:
        ref = store.publish([Record("message", b"hello")], submission_id="write-1")
        store.retain("application:checkpoint-1", [ref])
        store.release_publications([ref])  # The checkpoint now owns this data.
        assert next(store.read(ref)).payload == b"hello"

        store.seal()  # Stop appending to this pack before it can be collected.
        assert store.collect_garbage()["reclaimed_files"] == 0
        store.release("application:checkpoint-1")
        assert store.collect_garbage()["reclaimed_files"] == 1
```

A returned reference is an address, not a permanent lifetime guarantee.
Publication staging initially protects it. A queue, checkpoint, explicit owner
or reader pin must protect it while it is in use. Garbage collection removes
only **sealed packs with no remaining owners**; closing a writer does not
release application ownership. See [data and lifetime APIs](docs/USAGE.md).

## What can I store?

| Data | Write | Read |
|---|---|---|
| Opaque bytes, encoded text, JSON, images or application formats | `Record` + `store.publish` | `store.read` / `read_record`, then your decoder |
| NumPy arrays and PyTorch tensors | `publish_tensors` | `TensorRef.load()` or contiguous row slices |
| Several records or tensors forming one result | `publish` / `publish_many` | One `RecordSetRef` addresses the ordered records |
| Shared immutable data used by several queues | Dependencies and `TensorRef.share` | Each queue retains the same underlying packs |
| A changed tensor | `TensorRef.updated` | A new tensor; the original remains unchanged |

straw never unpickles stored payloads. Codec names are explicit and versioned;
applications own serialization of their custom objects. Tensor helpers copy
device data to contiguous CPU storage and support checked row reads. They are
not GPU IPC or page-level copy-on-write. [Supported types and examples](docs/USAGE.md#tensors).

Applications may use multiple processes/machines or threads. straw exposes
synchronous Rust-backed calls and does not start an I/O process pool itself.
See [concurrency and writer ownership](docs/USAGE.md#concurrency-processes-and-threads).

## Keep file counts bounded by packs

Reuse a store per writer process. Its default 1 GiB pack contains many
publications, including their manifests. Queue metadata uses fixed append logs,
and GC uses one shared catalog. There is **no file per sample or tensor**.

Rotate by bytes, batch small writes with `publish_many`, release consumed data,
and seal idle writers. A pack containing even one live record remains retained.
Repeated writer restarts, per-sample `close()`/`seal()`, or retaining every
checkpoint can still grow storage; the target size is not a global quota.
[File-count and retention planning](docs/FILESYSTEM.md#file-count-and-space-planning).

## Run something small

After installing the wheel, examples from this source tree run directly:

```sh
python examples/records.py
python examples/tensors.py
python examples/work_queue.py
python examples/multiprocess.py --root /tmp/new-straw-recovery-run
```

Run a bounded local benchmark without SSH or GPUs:

```sh
python -m straw.benchmark run --local \
  --root /tmp/new-straw-benchmark --report benchmark-results/local.json \
  --gib 0.0625 --record-bytes 1048576 262144 \
  --segment-mib 8 --online-gc --max-files 64 --max-gib 0.25
```

The benchmark verifies reads, measures durable publication/acceptance latency,
reports bytes and file counts, and removes its temporary payload root after
workers stop. [Multi-host benchmarks and trace replay](docs/BENCHMARKS.md).

## Read next

- [Rationale and architecture](docs/DESIGN.md)
- [Records, tensors, tasks, recovery and GC](docs/USAGE.md)
- [Filesystem deployment and file-count planning](docs/FILESYSTEM.md)
- [Protocol, prior work and correctness boundaries](docs/PROTOCOL_AND_VERIFICATION.md)
- [Executable verification](docs/VERIFICATION.md) and [binary format](docs/FORMAT.md)
- [Wheel builds and release checks](docs/BUILDING.md)
- [Release notes](CHANGELOG.md)
- [Application integration](docs/APPLICATIONS.md) and [contributing](CONTRIBUTING.md)

## Build and test on GitHub

- [Full tests](.github/workflows/tests.yml): Rust tests, formatting and Clippy,
  plus the complete Python suite, ownership model, crash campaign, examples and
  bounded I/O/GC benchmark against installed wheels on CPython 3.10–3.13.
- [Wheels](.github/workflows/wheels.yml): build and verify Linux x86_64 wheels
  for CPython 3.10–3.13 (manylinux/glibc 2.28+), plus a source archive and rebuild
  check. Download the outputs from the run's **Artifacts** section. Version-tag
  pushes publish the verified artifacts to PyPI through Trusted Publishing.

Both workflows support **Actions → select workflow → Run workflow**. Tests also
run on pushes and pull requests; wheels run on pull requests and `v*` tags.
Hosted CI uses a local filesystem and CPU PyTorch; deployment qualification on
multiple JuiceFS clients and GPU training use [separate checks](docs/VERIFICATION.md).
See [build and release instructions](docs/BUILDING.md).

## License

[MIT](LICENSE).
