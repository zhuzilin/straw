# Measure your workload

[中文版](BENCHMARKS_zh.md)

The installed wheel includes `python -m straw.benchmark` (also available as
`straw-benchmark`). It measures packed writes, durable queue acceptance and
checked reads. No model or GPU is required.

## One machine

```sh
python -m straw.benchmark run --local \
  --root /tmp/new-straw-bench --report benchmark-results/local.json \
  --record-bytes 1048576 262144 --gib 0.0625 \
  --writers 2 --readers 2 --segment-mib 8 --online-gc \
  --max-files 64 --max-gib 0.25 --timeout 120
```

This publishes two records per task: 1 MiB and 256 KiB. One local worker process
hosts the requested writer/reader threads. The coordinator runs in the parent.
The root must be new; the report must live outside it. Payloads are generated
locally, reads verify the stored checksums, and total read bytes must match
the chosen fanout. `--online-gc` collects concurrently and verifies reclamation
after all benchmark readers have finished.

## Several machines

Install the same wheel on every host. Each needs the same writable mount path,
SSH access from the launcher, and network access to the launcher's coordinator.
Launch on the first listed host:

```sh
python -m straw.benchmark run --hosts HOST_A HOST_B HOST_C HOST_D \
  --root /shared/new-straw-bench --report benchmark-results/four-host.json \
  --record-bytes 12582912 4194304 4194304 --gib 8 \
  --writers 2 --readers 2 --segment-mib 1024 --online-gc \
  --max-files 128 --max-gib 12 --timeout 600
```

The first host also participates as a writer and reader. Default fanout 1 uses
a ring, so each remote worker reads another worker's writes. Set `--fanout 4`
for all four hosts to read every publication. Fanout must be 1 or the number
of hosts. Different fanout changes read volume; compare like-for-like runs.

The launcher starts **one worker process per host**. Within that process,
`--writers` and `--readers` select thread counts, not process counts; each writer
thread opens an independent store/pack stream. The coordinator runs in the
launcher process. Local mode uses one worker process on the launcher machine.
straw itself creates no process pool; applications choose their own topology.

`--python /path/to/python` chooses the installed interpreter on remote hosts.
SSH configuration controls usernames/ports. `--advertise-host ADDRESS` selects
a reachable coordinator address when SSH aliases differ. The default bind is
`0.0.0.0`; use a private job network. For source development only, set
`--source-dir /shared/checkout/src`. Installed wheels need no shared source tree.

Without `--record-bytes` or `--profile`, the default is an explicit AI
shape model: four 8K-token samples, each with int32 routes `[tokens,48,8]`,
int32 candidate IDs `[tokens,128]` and float32 scores `[tokens,128]`.
`--tokens` and `--samples` adjust it. This is synthetic sizing, not a model run.

## Profile and replay an application

```sh
STRAW_TRACE_DIR=/shared/my-trace python your_application.py
python -m straw.benchmark profile --trace /shared/my-trace \
  --output benchmark-results/profile.json
python -m straw.benchmark run --hosts HOST_A HOST_B HOST_C HOST_D \
  --root /shared/new-straw-replay --report benchmark-results/replay.json \
  --profile benchmark-results/profile.json --gib 8 \
  --segment-mib 1024 --online-gc --max-files 128 --max-gib 12
```

The profile file must be available at the same path to all workers. Traces use
one append file per process and contain sizes, timing, codec and tensor shape
metadata, not tensor values or application record metadata. Keep traces outside
payload roots and apply your own trace retention policy.

Replay samples the observed publication-size distribution with a fixed seed.
It preserves one publication per task by default, not original request arrival
times, queue/optimizer semantics or every row-slice read. The finite number of
tasks can round the requested GiB down or up; the actual report is authoritative.
`--publications-per-task N` explicitly combines N observed publications into
one task/write, changing transaction granularity. Do not label that a fair
same-workload comparison against an unbatched run.

## Read the report

| Field | Meaning |
|---|---|
| `written_bytes`, `read_bytes` | Application payload bytes completed by benchmark workers |
| `write_mib_s`, `read_mib_s` | Payload volume / slowest worker's elapsed time |
| Worker latency percentiles | Publication, acceptance RPC and read completion latency |
| `data_files`, `file_count`, `physical_bytes` | Files/logical file lengths before final acknowledged reclamation |
| `gc`, `gc_seconds` | Measured final collection result and latency |
| `concurrent_gc_passes` | Collections while benchmark workers were active |
| Worker CPU/RSS | Per-process CPU time and peak resident memory |

Fresh paths and cross-client reads do not mean cold backing storage. No global
cache eviction is performed. Application throughput does not measure object
store/device bandwidth, network traffic or filesystem-internal request count.
Compare hardware, filesystem/cache settings, shapes, publication sizes, reader
fanout, durability and retained work together.

Trace writes count framed extent bytes and additionally report payload bytes;
reads count bytes returned to callers. `checked_read_bytes` counts authenticated
chunks, excluding index/journal traffic. `checked_read_amplification` divides
checked chunks by returned bytes; it is unknown when the trace lacks range-read
counters. `read_to_extent_write_ratio` describes the read/write mix separately.
Older experimental profiles called that ratio `read_amplification`; their
publication-size lists remain replayable. Completion peaks
credit bytes when an operation finishes in aligned 1/10-second windows and
require aligned host clocks.

## Storage and cleanup

`--max-files` and `--max-gib` guard the finite run. The benchmark records a JSON
report, payload-free `.logs.tar.gz` and `.cleanup.json` outside the temporary
root. On successful completion, it confirms owned worker processes stopped before deleting
the root. An SSH connection closing is not sufficient proof of remote exit.
If a remote host cannot be inspected/stopped, cleanup fails and preserves the
root for supervised recovery. Do not remove it until its workers are confirmed
dead. Failed runs, including capacity exhaustion, retain their root for inspection;
the benchmark does not reset the queue, raise limits or restart automatically.
Root budgets do not apply to the external report/trace directory.

<a id="published-results"></a>

## Selected public measurements

These records describe the 2026-10-26 0.1.4 candidate, before the subsequent
verification reduction and owner fix; they are not measurements of current code.
Only data, samples and hashes supporting the public results below are retained.
Mount/hardware conditions matter; old build measurements are not current
performance promises.

### 0.1.4 candidate performance comparison

The [release comparison](../verification/measurements/20261026-release-comparison.json)
ran 0.1.3 and the 0.1.4 candidate on one host and the same JuiceFS mount, in
baseline/current/current/baseline order with a fresh root each time and online
GC enabled. Each round published and authenticated 1 GiB (32 × 32 MiB), appended
500 WAL frames, replayed a 2,000-frame catalog, and exercised task acceptance,
retirement and collection. WAL and catalog recovery each ran five times.

| Metric | 0.1.3 | 0.1.4 candidate | Change |
|---|---:|---:|---:|
| Publication throughput | 134.93 MiB/s | 107.00 MiB/s | −20.7% |
| Authenticated read throughput | 314.18 MiB/s | 322.91 MiB/s | +2.8% |
| 500 synchronous WAL appends | 8.395 s | 8.094 s | −3.6% time |
| Acceptance of 64 tasks | 2.843 s | 3.832 s | +34.8% time |
| Collection of 24 packs | 153.84 ms | 161.28 ms | +4.8% time |

Values are medians across two process rounds per build; the GC value first
takes the median of three collections per round. All phases passed. Caches
were not evicted, so immediate reads and repeated recovery may use warm client
and server caches. These samples establish neither peak multi-host bandwidth
nor a latency bound. The publication and task-acceptance regressions are visible
in both current rounds. This compares complete builds, including changed
durability and writer-fencing work; no profiling was performed to attribute
the differences. The linked record retains every sample and the exact wheel,
native-library and probe hashes. The build hashes identify this historical candidate, not the current source tree.

### 0.1.4 candidate: one 100 GB record across four nodes

A separate [100 GB run](../verification/measurements/20261026-release-100gb.json)
used one writer node to publish a record containing 100,000,000,000 position-dependent payload
bytes. Four readers divided the record and jointly checked every byte against
an independent expected-value calculation. This is a different workload from
the 32 MiB publication comparison above; it is not a version-to-version result.

| Stage | Observed result |
|---|---|
| Checksum preparation before publication | 113.501 s, excluded from publication timing |
| Single-node publication | 358.855 s; 265.755 MiB/s |
| Four-node read and byte-oracle wall time | 55.275 s; 1,725.333 MiB/s combined |
| Per-node native full-read throughput | 593.8–625.8 MiB/s |

Publication still includes regenerating the positioned payload, authentication
and synchronization. Read wall time includes orchestration, boundary slices
and the byte oracle; per-node native timings exclude the subsequent oracle.
Caches were not evicted, so these logical rates are not cold-read or backend
bandwidth ceilings. One publication supplies no publication p99.

After ownership release, normal GC reported one reclaimed file and
100,003,195,567 bytes, including framing and metadata. This confirms durable
deletion from the filesystem namespace; it does not measure when an underlying
object store physically releases that space. The linked record identifies the
exact probe source and native binary.

### Four-node visibility sample

The [visibility probe](../verification/measurements/20261026-release-visibility.json)
used one writer and three remote readers, with 1,000 publications of 64 KiB per
mode. All 6,000 reads succeeded without failed attempts. The empirical p99 from
fsync return to validated reply was 9.515 ms while keeping the writer open and
9.035 ms after closing it. This includes notification, fresh open, read, hash
validation and reply; it is neither a pure filesystem visibility delay nor a
guaranteed latency bound. Both modes used the same mounted deployment, without
cache eviction. All probe workers exited and their temporary data was removed.
