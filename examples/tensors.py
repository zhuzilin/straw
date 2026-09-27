"""NumPy/PyTorch, checked row reads, two-queue sharing and tensor-level COW."""

from tempfile import TemporaryDirectory

import numpy as np
import torch

from straw import Coordinator, SharedFilesystemStore, TaskSpec
from straw.tensor import publish_tensors, release_tensor_publications


def main():
    with TemporaryDirectory(prefix="straw-tensors-") as root:
        with SharedFilesystemStore(root, "tensors", codecs=("tensor.v1",), online_gc=True) as store:
            original, scores = publish_tensors(
                store,
                {"embeddings": np.arange(24, dtype=np.float32).reshape(6, 4), "scores": torch.ones(6)},
                submission_id="source",
            )
            assert original[2:4].shape == (2, 4)
            shared, ordinal = original.share(store, submission_id="share-source")
            store.seal()
            changed = original.updated(store, torch.zeros((1, 4)), rows=slice(0, 1), submission_id="changed")
            assert original.load()[0].tolist() == [0.0, 1.0, 2.0, 3.0]
            assert changed.load()[0].tolist() == [0.0] * 4
            store.seal()

            with Coordinator(store, queue_id="stage-a", namespace=True, exclusive_owner="this process") as a:
                with Coordinator(store, queue_id="stage-b", namespace=True, exclusive_owner="this process") as b:
                    a.submit_tasks("input-a", [TaskSpec("a", input_ref=shared)])
                    b.submit_tasks("input-b", [TaskSpec("b", input_ref=shared)])
                    a.cancel_task("a", request_id="finished-a")
                    assert a.collect_garbage()["reclaimed_files"] == 0
                    assert scores.load().tolist() == [1.0] * 6
                    b.cancel_task("b", request_id="finished-b")
                    assert b.collect_garbage()["reclaimed_files"] == 1
            # The changed tensor has independent staging and remains readable.
            assert changed[0:1].tolist() == [[0.0] * 4]
            release_tensor_publications(store, [changed])
            assert store.collect_garbage()["reclaimed_files"] == 1
    print("Verified arrays, checked slices, two owners, tensor COW and reclamation.")


if __name__ == "__main__":
    main()
