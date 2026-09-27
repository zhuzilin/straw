# Verify the protocol and deployment

[中文版](VERIFICATION_zh.md)

The [protocol document](PROTOCOL_AND_VERIFICATION.md) states invariants,
durability order, assumptions and prior work. Each check below tests a different
boundary. A green unit suite is not a proof of filesystem power-loss safety.

## Fast local checks

From a development checkout with Rust 1.89+, PyTorch and the package installed:

```sh
cargo fmt --check
cargo clippy --all-targets --features python -- -D warnings
cargo test --locked
python -m pytest
python verification/ownership_model.py --output /tmp/straw-ownership.json
python verification/crash_campaign.py --output /tmp/straw-crashes.json
```

| Check | Evidence |
|---|---|
| Rust tests | Native packing, interrupted tails, atomic acceptance and fencing |
| Python regression suite | Native core through public bindings: corrupt/truncated data, bounded capacity, retry/receipt semantics, leases, continuations, tensors, ownership and GC |
| Independent ownership model | Exhaustive bounded interleavings of two queues, a reader, checkpoint and crash; five deliberately unsafe protocols, including checkpoint rewind must produce counterexamples |
| Native crash campaign | Every byte prefix and one XOR mutation at every byte in new WAL frames; abrupt process exits at ownership/WAL handoff boundaries |
| Examples and wheel smoke | Installation and public usage, without source imports or a Rust runtime toolchain |

The finite model explores an abstraction, not the Rust implementation, arbitrary
dependency graphs or unbounded histories. It is not TLA+/TLAPS refinement or a
liveness proof. The crash campaign exercises prefix persistence and process
death; the kernel remains alive. Its private temporary directory is removed
after owned child processes exit. Choose an output outside that directory.

## Several filesystem clients

On a shared checkout and mount, with the same package/interpreter available
on four hosts and SSH configured:

```sh
python verification/crash_campaign.py \
  --hosts HOST_A HOST_B HOST_C HOST_D --parent /shared/test-tmp \
  --output /tmp/straw-four-client-crashes.json
```

The launcher runs on the first host. The harness moves child crash/handoff
cases among four clients and checks recovered ownership before source release.
It uses the shared verification script; application users running installed
wheel benchmarks do not need this source checkout.

The [multi-host benchmark](BENCHMARKS.md) adds concurrent packed I/O, durable
acceptance, independent readers and online collection. Bound its storage budget
and use a fresh root. Verify the returned byte counts, exit status and cleanup
record, not just throughput or the launcher's success message.

## Focused GC probes

`verification/gc_scale.py` measures ownership cost with tiny records. Start one
copy on each client, with a distinct queue name and the same fresh root:

```sh
python verification/gc_scale.py --root /shared/new-gc-scale \
  --queue HOST_A --count 1024 --participants 4
```

Repeat on HOST_B/C/D concurrently. Each process submits 1,024 roots in its
namespaced queue, joins a four-client barrier, reopens its coordinator, times
GC, reads sampled live records and verifies deletion of a dead pack. Increase
to `--count 16384` for 65,536 total roots. Timing excludes coordinator/catalog
replay and does not measure tensor bandwidth or cold storage. The caller owns
cleanup **after all four processes stop**. To continue an interrupted fixture,
confirm its old workers stopped before using `--resume` on the same queues.

`verification/gc_cross_client.py` isolates cached positive attributes after a
remote deletion. Prepare a unique root on one client:

```sh
python verification/gc_cross_client.py --root /shared/new-gc-cache
```

Start the following on each of the four clients with distinct `--worker` names:

```sh
python verification/gc_cross_client.py --root /shared/new-gc-cache --worker HOST_A
```

Each client primes its stat cache and writes one `ready-<worker>` marker. After
all four markers are present, the supervisor creates `/shared/new-gc-cache/go`.
The workers collect concurrently; each must exit successfully and their total
`reclaimed_files` must be one. Only then remove the private root. The 60-second
barrier deadline bounds a failed setup. This fixture has one pack and a fixed
number of markers, not a file per sample.

## Interpreting failures

Preserve the failing source/wheel hash, configuration, operation IDs and a small
reproducer. Keep payloads bounded and do not delete storage still used by a
reader or an uncertain process. A lost RPC reply can mean a committed operation;
resolve it through its stable identity. Complete checksum errors must remain
errors, not be truncated away as if they were a partial tail.

Before broader production claims, qualify actual client/mount cache settings,
distributed locks and sync semantics, storage-service interruption, device/VM
power loss, ENOSPC/EIO and long-duration retention/recovery. Application model
training and joint optimizer checkpoints need their own end-to-end tests.
