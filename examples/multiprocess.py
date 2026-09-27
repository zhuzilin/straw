"""CPU-only multiprocessing example with durable queue recovery.

Usage after installing the wheel: python examples/multiprocess.py --root /shared/new-run
"""

import argparse
import multiprocessing
from pathlib import Path

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.protocol import Lease
from straw.rpc import QueueClient, serve


def coordinator(root, pipe):
    queue = Coordinator(SharedFilesystemStore(root, "cpu-example"), exclusive_owner="example process supervisor")
    serve(queue, ("127.0.0.1", 0), token="local-example", ready=lambda address: pipe.send(address[1]))


def producer_worker(root, port, identity):
    client = QueueClient(f"http://127.0.0.1:{port}", token="local-example")
    store = SharedFilesystemStore(root, "cpu-example")
    client.call(
        "submit_tasks", request_id=f"producer-{identity}", tasks=[TaskSpec(f"task-{identity}-{i}") for i in range(4)]
    )
    assignments = client.call("acquire", worker_id=str(identity), max_tasks=4)["assignments"]
    leases = [Lease(**assignment["lease"]) for assignment in assignments]
    refs = store.write_records(
        [
            Record(
                lease.task_id,
                f"generated:{lease.task_id}".encode(),
                metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id},
            )
            for lease in leases
        ],
        submission_id=f"microbatch-{identity}",
    )
    for lease, ref in zip(leases, refs, strict=True):
        result = store.record_set([ref], submission_id=lease.task_id)
        client.call(
            "complete_task", lease=lease, submission_id=lease.task_id, result_ref=result, result_digest=result.digest
        )
    store.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    args = parser.parse_args()
    root = str(Path(args.root).resolve())
    Path(root).mkdir(parents=True, exist_ok=False)
    context = multiprocessing.get_context("spawn")
    receiver, sender = context.Pipe(duplex=False)
    owner = context.Process(target=coordinator, args=(root, sender))
    workers = []
    owner.start()
    try:
        if not receiver.poll(30):
            raise TimeoutError("Coordinator did not become ready")
        port = receiver.recv()
        for identity in range(4):
            worker = context.Process(target=producer_worker, args=(root, port, identity))
            workers.append(worker)
            worker.start()
        for worker in workers:
            worker.join(60)
            if worker.exitcode != 0:
                raise RuntimeError(f"Worker failed: {worker.exitcode}")
        # Demonstrate a crash rather than a graceful state save. A process handle,
        # not a stale marker, proves the previous owner has actually terminated.
        owner.kill()
        owner.join(10)
        if owner.exitcode is None:
            raise RuntimeError("Old coordinator is still running; cannot recover")
        store = SharedFilesystemStore(root, "cpu-example")
        queue = Coordinator(store, exclusive_owner="old process joined", recover=True)
        try:
            queue.seal_input("producer-end")
            first = queue.read_commits()
            second = queue.read_commits()
            assert first == second and len(first.commits) == 16 and first.end_of_input
            for commit in first.commits:
                record = next(store.read(commit.result_ref))
                assert record.payload == f"generated:{commit.task_id}".encode()
            print("Verified 4 producers/workers, 16 results, crash recovery and independent replay.")
        finally:
            queue.close()
            store.close()
    finally:
        for process in [*workers, owner]:
            if process.is_alive():
                process.kill()
            process.join(10)
        receiver.close()
        sender.close()


if __name__ == "__main__":
    main()
