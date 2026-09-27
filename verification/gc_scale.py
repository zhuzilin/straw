"""Measure cold-coordinator GC with many live roots, in a caller-owned pool.

Run concurrently with one distinct --queue per host on a fresh shared --root.
The caller removes the pool only after all workers exit. Payloads stay packed;
the tiny records isolate ownership/descriptor cost rather than data bandwidth.
"""

import argparse
import json
import socket
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.protocol import Limits, Publication, RecordSetRef


def run(root, name, count, *, resume=False, participants=1):
    root = Path(root).resolve()
    limits = Limits(pending_tasks=count + 1)

    def open_store():
        return SharedFilesystemStore(root, root.name, online_gc=True)

    def open_queue(writer, recover=False):
        return Coordinator(writer, queue_id=name, namespace=True, exclusive_owner=name, limits=limits, recover=recover)

    started = time.monotonic()
    writer = open_store()
    owner = open_queue(writer, recover=resume)
    samples = []
    tasks = owner.tasks if resume else {}
    recovered_roots = len(tasks)
    assert set(tasks) == {str(i) for i in range(recovered_roots)}, "Fixture tasks must be a contiguous prefix"
    assert recovered_roots <= count and (recovered_roots % 16 == 0 or recovered_roots == count)
    for offset in range(0, recovered_roots, 16):
        samples.append((offset, RecordSetRef.from_dict(tasks[str(offset)]["spec"]["input_ref"])))
    del tasks
    for offset in range(recovered_roots, count, 16):
        publications = [
            Publication((Record(f"{name}:{i}", bytes([i % 251]) * 64),))
            for i in range(offset, min(offset + 16, count))
        ]
        refs = writer.publish_many(publications, submission_id=f"{name}:{offset}")
        owner.submit_tasks(str(offset), [TaskSpec(str(offset + i), input_ref=ref) for i, ref in enumerate(refs)])
        samples.append((offset, refs[0]))
    writer.close()
    owner.close()
    populated_seconds = time.monotonic() - started

    # A fixed number of per-worker markers ensures all live roots exist before
    # timing any collector. This is not part of straw's storage protocol.
    if participants > 1:
        (root / f"fixture-ready-{name}").write_text(str(count))
        deadline = time.monotonic() + 300
        while len(list(root.glob("fixture-ready-*"))) < participants:
            if time.monotonic() > deadline:
                raise TimeoutError("Other fixture workers did not finish populating")
            time.sleep(0.2)

    # A fresh Config/Coordinator has no cached descriptor closures.
    reader = open_store()
    recovered = open_queue(reader, recover=True)
    started = time.monotonic()
    collection = recovered.collect_garbage()
    cold_gc_seconds = time.monotonic() - started
    for offset, ref in samples:
        assert all(record.payload == bytes([offset % 251]) * 64 for record in reader.read(ref))

    dead = reader.publish([Record("garbage", bytes(1024))], submission_id=f"{name}:garbage")
    reader.seal()
    reader.release_publications([dead])
    recovered.collect_garbage()
    # Remote unlink can leave a positive stat cache entry on this client.
    # Verify through open, with a bounded visibility wait, not cached exists().
    visibility_started = time.monotonic()
    while True:
        try:
            with (root / dead.manifest.segment.path).open("rb"):
                pass
        except FileNotFoundError:
            break
        if time.monotonic() - visibility_started > 5:
            raise AssertionError("Collected garbage can still be opened")
        time.sleep(0.05)
    recovered.close()
    reader.close()
    return dict(
        hostname=socket.gethostname(),
        queue=name,
        live_roots=count,
        logical_records=count,
        populated_seconds=populated_seconds,
        recovered_roots=recovered_roots,
        resumed=resume,
        participants=participants,
        cold_gc_seconds=cold_gc_seconds,
        sampled_publications_read=len(samples),
        collection=collection,
        garbage_unlinked=True,
        unlink_visibility_seconds=time.monotonic() - visibility_started,
        passed=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("--queue", required=True)
    parser.add_argument("--count", type=int, default=16384)
    parser.add_argument(
        "--resume", action="store_true", help="Continue an interrupted fixture after all old workers stop"
    )
    parser.add_argument("--participants", type=int, default=1)
    args = parser.parse_args()
    if not 0 < args.count <= 65536:
        parser.error("--count must be in [1, 65536]")
    if not 1 <= args.participants <= 16:
        parser.error("--participants must be in [1, 16]")
    print(
        json.dumps(
            run(args.root, args.queue, args.count, resume=args.resume, participants=args.participants), sort_keys=True
        )
    )
