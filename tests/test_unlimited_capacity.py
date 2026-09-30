"""Large logical results must survive publication, acceptance and recovery.

The dependency DAG test uses shared payloads: >64 GiB of logical accounting
requires only a few MiB on disk, and exercises real manifests, not forged refs.
"""

import dataclasses
import multiprocessing

from straw import Publication, Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.protocol import Limits, encode


def test_large_result_and_dependency_totals_are_not_admission_limits(tmp_path):
    store = SharedFilesystemStore(tmp_path, "large", max_buffer_bytes=65536)
    queue = Coordinator(store, exclusive_owner="test")
    queue.submit_tasks("inputs", [TaskSpec("collection"), TaskSpec("next")])
    lease = queue.acquire("builder").assignments[0].lease
    ref = store.publish([Record("tensor", b"x" * (8 * 1024**2), tokens=2_000_000)], submission_id="tensor")
    for i in range(3):
        ref = store.publish([Record(f"level-{i}", b"refs")], dependencies=[ref] * 32, submission_id=f"level-{i}")
    assert ref.payload_bytes > 64 * 1024**3
    assert ref.tokens > 2_000_000_000
    result = store.publish(
        [Record("collection", b"batch", metadata={"task_id": lease.task_id, "attempt_id": lease.attempt_id})],
        dependencies=[ref],
        submission_id="collection",
    )
    bytes_before = store.metrics["payload_bytes"]
    receipt = queue.complete_task(lease, submission_id="accepted", result_ref=result, result_digest=result.digest)
    assert store.metrics["payload_bytes"] == bytes_before  # acceptance never rewrites tensors
    assert queue.lookup_submission("accepted") == receipt
    assert queue.acquire("next").status == "acquired"  # no 64 GiB / token backlog throttle
    queue.close()
    recovered = Coordinator(store, exclusive_owner="previous owner stopped", recover=True)
    try:
        assert recovered.read_commits(limit=2000).commits == (receipt,)
        assert recovered.lookup_submission("accepted") == receipt
    finally:
        recovered.close()
        store.close()


def test_record_index_manifest_and_dependency_counts_exceed_old_caps(tmp_path):
    store = SharedFilesystemStore(tmp_path, "wide", max_buffer_bytes=65536)
    # 10,001 records and >8 MiB index; caller metadata is not a payload buffer.
    records = [Record(str(i), bytes([i % 256]), metadata={"description": "x" * 850}) for i in range(10001)]
    refs = store.write_records(records, submission_id="wide-records")
    result = store.record_set(refs, submission_id="wide-manifest")
    assert len(store.validate(result)) == len(records)
    assert store.read_record(refs[-1]).payload == records[-1].payload
    # Distinct manifest nodes, with only one physical payload per sample.
    leaves = store.publish_many(
        [Publication((Record(f"leaf-{i}", b"x"),)) for i in range(10001)], submission_id="leaves"
    )
    aggregate = store.publish([Record("aggregate", b"refs")], dependencies=leaves, submission_id="aggregate")
    assert len(store.validate(aggregate)) == 1
    assert len(store.manifest(aggregate)["dependencies"]) == 10001
    store.close()


def test_large_control_journal_and_legacy_quotas_allow_work(tmp_path):
    limits = Limits(**{field.name: 1 for field in dataclasses.fields(Limits)})
    store = SharedFilesystemStore(tmp_path, "control")
    queue = Coordinator(store, exclusive_owner="test", limits=limits)
    # Exceeds the former 8 MiB journal and 256 KiB control metadata caps.
    metadata = {"description": "x" * (9 * 1024**2)}
    tasks = [TaskSpec("large", metadata=metadata, estimated_bytes=10**12, estimated_tokens=2_000_000_000)]
    tasks += [
        TaskSpec("other"),
        TaskSpec("control1", control=True, estimated_records=0),
        TaskSpec("control2", control=True, estimated_records=0),
    ]
    queue.submit_tasks("inputs", tasks)
    assert len(queue.acquire("worker", max_tasks=2).assignments) == 2
    assert len(queue.acquire("builder", max_tasks=2, control=True).assignments) == 2
    queue.close()
    recovered = Coordinator(store, exclusive_owner="old owner stopped", limits=limits, recover=True)
    try:
        assert recovered.task_status("large")["spec"]["metadata"] == metadata
        assert len(recovered.acquire("worker", max_tasks=2).assignments) == 2
    finally:
        recovered.close()
        store.close()


def _rpc_server(root, ready):
    from straw.rpc import serve

    store = SharedFilesystemStore(root, "rpc")
    queue = Coordinator(store, exclusive_owner="test", limits=Limits(metadata_bytes=1))
    serve(queue, ("127.0.0.1", 0), token="test-token", ready=ready.send)


def test_rpc_large_requests_and_responses_ignore_legacy_metadata_quota(tmp_path):
    from straw.rpc import QueueClient

    context = multiprocessing.get_context("spawn")
    parent, child = context.Pipe()
    process = context.Process(target=_rpc_server, args=(str(tmp_path), child))
    process.start()
    try:
        assert parent.poll(30)
        host, port = parent.recv()
        client = QueueClient(f"http://{host}:{port}", token="test-token", metadata_bytes=1)
        metadata = {"description": "x" * (512 * 1024)}
        client.call("submit_tasks", request_id="rpc", tasks=[TaskSpec("task", metadata=metadata)])
        result = client.call("acquire", worker_id="worker", max_tasks=1)
        assert result["assignments"][0]["task"]["metadata"] == metadata
        assert len(encode(result)) > 256 * 1024
    finally:
        process.terminate()
        process.join(10)
        if process.is_alive():
            process.kill()
            process.join(10)
        parent.close()
        child.close()
