"""Producer continuation ownership participates in the queue's actual GC roots."""

from dataclasses import asdict

import pytest

from straw import Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.protocol import RecordSetRef


def open_store(root):
    return SharedFilesystemStore(root, "producer-roots", online_gc=True, segment_target_bytes=0)


def open_queue(store, name="sampler", *, recover=False):
    return Coordinator(store, queue_id=name, namespace=True, exclusive_owner="previous owner stopped", recover=recover)


def blob(store, name, *, dependencies=()):
    return store.publish([Record(name, name.encode())], submission_id=name, dependencies=dependencies)


def saved(queue):
    return RecordSetRef.from_dict(queue.producer_state("dataset")["state_ref"])


@pytest.mark.parametrize("reopen", [False, True])
def test_producer_only_root_survives_gc_and_recovery(tmp_path, reopen):
    with open_store(tmp_path) as store:
        queue = open_queue(store)
        state = blob(store, "rng-and-cursor")
        queue.submit_tasks(
            "initial", [TaskSpec("sample")], producer_id="dataset", producer_state={"state_ref": state, "cursor": 1}
        )
        # Neither task input nor consumer/checkpoint ownership can protect this
        # payload. Its sole durable queue owner is the producer continuation.
        assert queue.tasks["sample"]["spec"]["input_ref"] is None
        if reopen:
            queue.close()
            queue = open_queue(store, recover=True)
        try:
            queue.collect_garbage()
            assert saved(queue) == state
            assert next(store.read(saved(queue))).payload == b"rng-and-cursor"
        finally:
            queue.close()


@pytest.mark.parametrize("reopen", [False, True])
def test_producer_replacement_releases_only_old_closure_and_retry_keeps_latest(tmp_path, reopen):
    with open_store(tmp_path) as store:
        queue = open_queue(store)
        shared = blob(store, "shared-input")
        old_only = blob(store, "old-private-input")
        old = blob(store, "old-continuation", dependencies=[shared, old_only])
        initial = {"state_ref": asdict(old), "cursor": 1}
        assert queue.submit_tasks("first", [], producer_id="dataset", producer_state=initial) == []
        queue.collect_garbage()
        assert next(store.read(old)).payload == b"old-continuation"
        new = blob(store, "new-continuation", dependencies=[shared])
        queue.submit_tasks("second", [], producer_id="dataset", producer_state={"state_ref": new, "cursor": 2})
        if reopen:
            queue.close()
            queue = open_queue(store, recover=True)
        try:
            queue.collect_garbage()
            assert not (tmp_path / old.manifest.segment.path).exists()
            assert not (tmp_path / old_only.manifest.segment.path).exists()
            assert saved(queue) == new
            assert next(store.read(new)).payload == b"new-continuation"
            assert next(store.read(shared)).payload == b"shared-input"
            # The receipt acknowledges the old request without resurrecting its
            # producer state or trying to reopen its already collected payload.
            assert queue.submit_tasks("first", [], producer_id="dataset", producer_state=initial) == []
            assert saved(queue) == new
            queue.collect_garbage()
            assert next(store.read(new)).payload == b"new-continuation"
            assert next(store.read(shared)).payload == b"shared-input"
        finally:
            queue.close()


def test_peer_producer_retains_old_state_until_both_replace(tmp_path):
    with open_store(tmp_path) as store, open_queue(store, "a") as a, open_queue(store, "b") as b:
        shared = blob(store, "shared-dependency")
        old = blob(store, "shared-old-state", dependencies=[shared])
        for queue in (a, b):
            queue.submit_tasks("initial", [], producer_id="dataset", producer_state={"state_ref": old})
        new = blob(store, "shared-new-state", dependencies=[shared])
        a.submit_tasks("replacement", [], producer_id="dataset", producer_state={"state_ref": new})
        a.collect_garbage()
        assert saved(b) == old
        assert next(store.read(old)).payload == b"shared-old-state"
        assert next(store.read(new)).payload == b"shared-new-state"
        b.submit_tasks("replacement", [], producer_id="dataset", producer_state={"state_ref": new})
        b.collect_garbage()
        assert not (tmp_path / old.manifest.segment.path).exists()
        assert next(store.read(new)).payload == b"shared-new-state"
        assert next(store.read(shared)).payload == b"shared-dependency"
