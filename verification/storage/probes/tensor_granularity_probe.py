"""Exercise the real Python producer past 512 default chunks, then normal GC.

Uses a repeating byte sequence for API/metadata compatibility checks, not a
physical bandwidth benchmark. Run in an installed-wheel environment. Failed
fixtures are preserved; only successfully collected private metadata is removed.
"""

import argparse
import dataclasses
import hashlib
import json
import math
import resource
import shutil
import tempfile
import time
from pathlib import Path

import torch

import straw
from straw import SharedFilesystemStore, _native
from straw.protocol import encode
from straw.tensor import publish_tensors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expect-chunk", type=int, required=True)
    args = parser.parse_args()
    length = 2**31 + 17
    private = Path(tempfile.mkdtemp(prefix="straw-python-granularity-", dir=args.parent))
    report = {"passed": False, "private": str(private), "payload_bytes": length, "reads": []}
    try:
        torch.set_num_threads(1)
        value = torch.arange(length, dtype=torch.uint8)
        with SharedFilesystemStore(
            private / "pool", "python-granularity", codecs=("tensor.v1",), online_gc=True, segment_target_bytes=0
        ) as store:
            start = time.monotonic()
            (ref,) = publish_tensors(store, {"value": value}, submission_id="fixed")
            report["publication_seconds"] = time.monotonic() - start
            entry = store.inspect_segment(ref.record_ref.segment)["records"][ref.record_ref.ordinal]
            metadata = entry["envelope"]["metadata"]
            chunk = metadata["chunk_bytes"]
            assert chunk == args.expect_chunk
            assert len(metadata["chunks"]) == math.ceil(length / chunk)
            report.update(chunk_bytes=chunk, digests=len(metadata["chunks"]), file_bytes=Path(ref.path).stat().st_size)
            raw = encode(dataclasses.asdict(ref.record_ref)).decode()
            with store.read_session() as reader:
                for begin, end in [(0, 1), (chunk - 1, chunk + 17), (2**31 - 1, length), (length, length)]:
                    start = time.monotonic()
                    _, data, checked = reader._native.read_tensor_range(raw, begin, end)
                    seconds = time.monotonic() - start
                    assert data == bytes(i % 256 for i in range(begin, end))
                    assert checked == (
                        0 if begin == end else min(length, ((end - 1) // chunk + 1) * chunk) - begin // chunk * chunk
                    )
                    report["reads"].append({"start": begin, "stop": end, "checked": checked, "seconds": seconds})
            (again,) = publish_tensors(store, {"value": value}, submission_id="fixed")
            assert again.record_ref == ref.record_ref
            store.release_publications([ref.blob_set])
            store.seal()
            result = store.collect_garbage()
            assert result["reclaimed_files"] == 1 and result["reclaimed_bytes"] == report["file_bytes"]
            assert result["owners"] == result["open_packs"] == result["retained_packs"] == 0
            report["gc"] = result
        assert not [p for p in private.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]
        shutil.rmtree(private)
        report.update(
            passed=True,
            private_removed=not private.exists(),
            rss_kib=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
            installed_package=straw.__file__,
            native_sha256=hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
            tensor_source_sha256=hashlib.sha256(Path(straw.__file__).with_name("tensor.py").read_bytes()).hexdigest(),
            probe_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        )
    finally:
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
