"""Submit, acquire, complete, replay, acknowledge use and reclaim a task result."""

from tempfile import TemporaryDirectory

from straw import Coordinator, Record, SharedFilesystemStore, TaskSpec
from straw.protocol import encode


def main():
    with TemporaryDirectory(prefix="straw-queue-") as root:
        with SharedFilesystemStore(root, "queue-example", online_gc=True) as store:
            with Coordinator(store, queue_id="work", exclusive_owner="this process") as queue:
                source = store.publish([Record("input", b"hello")], submission_id="input-1")
                queue.submit_tasks("submit-1", [TaskSpec("uppercase-1", input_ref=source)])
                assignment = queue.acquire("worker-1").assignments[0]
                lease = assignment.lease
                payload = next(store.read(assignment.task.input_ref)).payload.upper()
                result = store.publish(
                    [Record("result", payload, metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id})],
                    submission_id="result-1",
                )
                receipt = queue.complete_task(
                    lease, submission_id="complete-1", result_ref=result, result_digest=result.digest
                )
                assert (
                    queue.complete_task(
                        lease, submission_id="complete-1", result_ref=result, result_digest=result.digest
                    )
                    == receipt
                )
                queue.seal_input("producer-finished")
                page = queue.read_commits()
                assert page == queue.read_commits()  # Independent replay before retirement.
                assert next(store.read(page.commits[0].result_ref)).payload == b"HELLO"

                store.retain("application:checkpoint", [result])
                store.seal()  # Separate the old data pack from the current consumer state.
                state = store.publish([Record("state", encode({"version": 1}), "json.v1")], submission_id="state-1")
                token = queue.open_consumer("training", exclusive_owner="all required readers have finished")
                # "training" is the designated processing consumer in protocol v1.
                queue.save_consumer_state(
                    "training",
                    token=token,
                    request_id="ack-1",
                    state_ref=state,
                    fetch_cursor=page.cursor,
                    processed_cursor=page.cursor,
                )
                assert queue.collect_garbage()["reclaimed_files"] == 0
                store.release("application:checkpoint")
                assert queue.collect_garbage()["reclaimed_files"] == 1
                # Current consumer state is still retained; close does not discard it.
                assert next(store.read(state)).codec == "json.v1"
    print("Verified task acceptance, retry, replay, consumption and checkpoint-aware GC.")


if __name__ == "__main__":
    main()
