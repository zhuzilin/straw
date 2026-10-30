"""Wide validation must retain authorization and operation-scoped failure checks."""

from pathlib import Path

import pytest

from straw import Publication, Record, SharedFilesystemStore
from straw.errors import CorruptData, InvalidReference


def wide_graph(store):
    leaves = store.publish_many(
        [Publication((Record(f"value-{i}", b"value"),)) for i in range(64)], submission_id="leaves"
    )
    # The wide branch and all of its children are also direct dependencies;
    # workers must handle overlap without skipping a failed owning worker.
    branch = store.publish([Record("branch", b"branch")], submission_id="branch", dependencies=leaves)
    wide = store.publish([Record("wide", b"wide")], submission_id="wide", dependencies=(*leaves, branch))
    root = store.publish(
        [Record("root", b"root", metadata={"task_id": "task", "attempt_id": "attempt"})],
        submission_id="root",
        dependencies=(wide,),
    )
    return root, leaves


def flip_index(root, segment):
    path = Path(root) / segment.path
    with path.open("r+b") as stream:
        stream.seek(segment.offset + segment.size - 80)
        length = int.from_bytes(stream.read(8), "little")
        offset = segment.offset + segment.size - 80 - length
        stream.seek(offset)
        value = stream.read(1)[0]
        stream.seek(offset)
        stream.write(bytes([value ^ 1]))


def test_wide_overlapping_graph_below_narrow_chain_and_root_authorization(tmp_path):
    with SharedFilesystemStore(tmp_path, "parallel-validation") as store:
        root, _ = wide_graph(store)
        with store.read_session() as reader:
            assert len(reader.validate(root, task_id="task", attempt_id="attempt")) == 1
            with pytest.raises(InvalidReference, match="task/attempt"):
                reader.validate(root, task_id="other", attempt_id="attempt")
            with pytest.raises(InvalidReference, match="task/attempt"):
                reader.validate(root, task_id="task", attempt_id="other")
            assert len(reader.validate(root, task_id="task", attempt_id="attempt")) == 1


def test_wide_worker_corruption_never_becomes_a_validation_proof(tmp_path):
    with SharedFilesystemStore(tmp_path, "parallel-validation") as store:
        root, leaves = wide_graph(store)
        segment = leaves[0].manifest.segment
        with store.read_session() as reader:
            assert len(reader.validate(root)) == 1
            flip_index(tmp_path, segment)
            for _ in range(2):
                with pytest.raises(CorruptData, match="index checksum"):
                    reader.validate(root)
            flip_index(tmp_path, segment)
            assert len(reader.validate(root)) == 1


def test_narrow_graph_still_revalidates_dependencies(tmp_path):
    with SharedFilesystemStore(tmp_path, "parallel-validation") as store:
        child = store.publish([Record("child", b"child")], submission_id="child")
        root = store.publish([Record("root", b"root")], submission_id="root", dependencies=(child,))
        assert len(store.validate(root)) == 1
        flip_index(tmp_path, child.manifest.segment)
        with pytest.raises(CorruptData, match="index checksum"):
            store.validate(root)
