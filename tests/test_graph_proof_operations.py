"""Queue operations must never carry graph validation across calls or failures."""

from pathlib import Path

import pytest

from straw import Publication, Record, SharedFilesystemStore, TaskSpec
from straw.coordinator import Coordinator
from straw.errors import CorruptData


def flip_index(root, reference):
    segment = reference.manifest.segment
    with (Path(root) / segment.path).open("r+b") as stream:
        stream.seek(segment.offset + segment.size - 80)
        length = int.from_bytes(stream.read(8), "little")
        position = segment.offset + segment.size - 80 - length
        stream.seek(position)
        value = stream.read(1)[0]
        stream.seek(position)
        stream.write(bytes([value ^ 1]))


def roots(store, width):
    leaves = store.publish_many(
        [Publication((Record(str(i), b"leaf"),)) for i in range(width)], submission_id="leaves"
    )
    root = store.publish([Record("root", b"root")], submission_id="root", dependencies=leaves)
    return leaves[0], root


@pytest.mark.parametrize("width", [1, 64])
def test_new_queue_call_rechecks_a_previously_accepted_dependency(tmp_path, width):
    with SharedFilesystemStore(tmp_path, "proof", online_gc=True) as store:
        leaf, root = roots(store, width)
        queue = Coordinator(store, namespace=True, queue_id="q", exclusive_owner="one")
        try:
            queue.submit_tasks("first", [TaskSpec("first", input_ref=root)])
            flip_index(tmp_path, leaf)
            with pytest.raises(CorruptData, match="index checksum"):
                queue.submit_tasks("second", [TaskSpec("second", input_ref=root)])
            flip_index(tmp_path, leaf)
            assert queue.submit_tasks("second", [TaskSpec("second", input_ref=root)]) == ["second"]
        finally:
            queue.close()


@pytest.mark.parametrize("phase", ["before_storage_catalog_write", "after_storage_catalog_sync"])
@pytest.mark.parametrize("width", [1, 64])
def test_failed_catalog_transfer_drops_proofs_before_retry(tmp_path, phase, width):
    with SharedFilesystemStore(tmp_path, "proof", online_gc=True) as store:
        leaf, root = roots(store, width)
        queue = Coordinator(store, namespace=True, queue_id="q", exclusive_owner="one")
        try:

            def crash(current):
                if current == phase:
                    raise RuntimeError("interrupted transfer")

            store.backend.fault = crash
            with pytest.raises(RuntimeError, match="interrupted transfer"):
                queue.submit_tasks("submit", [TaskSpec("task", input_ref=root)])
            store.backend.fault = lambda _: None
            flip_index(tmp_path, leaf)
            with pytest.raises(CorruptData, match="index checksum"):
                queue.submit_tasks("submit", [TaskSpec("task", input_ref=root)])
            flip_index(tmp_path, leaf)
            assert queue.submit_tasks("submit", [TaskSpec("task", input_ref=root)]) == ["task"]
        finally:
            store.backend.fault = lambda _: None
            queue.close()
