"""Measure writer history cost, sync hooks and alternating-extent reads.

Run the same probe against each installed wheel. Logical read bytes include
cache hits and are not physical device/network traffic. Roots are private and
removed after closing the probe's stores; reports stay outside those roots.
"""

import argparse
import collections
import hashlib
import importlib.metadata
import json
import statistics
import tempfile
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native


def logical_reads():
    return int(dict(line.split(": ") for line in Path("/proc/self/io").read_text().splitlines())["rchar"])


def measure(parent, count, online_gc):
    result = {
        "version": importlib.metadata.version("straw-queue"),
        "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
        "payload_bytes_per_write": 1,
        "online_gc": online_gc,
        "writer": [],
    }
    with tempfile.TemporaryDirectory(prefix="straw-metadata-", dir=parent) as private:
        root = Path(private)
        with SharedFilesystemStore(root, "scaling", online_gc=online_gc, segment_target_bytes=1024**3) as writer:
            elapsed = []
            samples = {1, min(100, count), max(1, count // 2), count}
            for n in range(1, count + 1):
                before = logical_reads() if n in samples else 0
                start = time.perf_counter()
                writer.write_records([Record("r", b"x")], submission_id=str(n))
                elapsed.append(time.perf_counter() - start)
                if n in samples:
                    read_bytes = logical_reads() - before
                    journal = next(root.rglob("operations/control/journal.log"))
                    result["writer"].append(
                        {
                            "call": n,
                            "last_20_median_ms": 1000 * statistics.median(elapsed[-20:]),
                            "logical_read_bytes": read_bytes,
                            "registry_bytes": journal.stat().st_size,
                        }
                    )
            phases = collections.Counter()
            writer.backend.fault = lambda at: phases.update([at])
            writer.write_records([Record("r", b"x")], submission_id="sync-count")
            # before_journal_sync and before_file_sync refer to the same fsync.
            result["sync_hooks"] = {k: v for k, v in sorted(phases.items()) if "before" in k and "sync" in k}
            writer.backend.fault = lambda _: None

            refs = []
            for extent in range(4):
                records = [Record(f"r-{i}", b"x") for i in range(128)]
                refs.append(writer.write_records(records, submission_id=f"read-{extent}")[0])
            with writer.read_session() as reader:
                for ref in refs:
                    assert reader.read_record(ref).payload == b"x"
                before = logical_reads()
                start = time.perf_counter()
                for n in range(400):
                    assert reader.read_record(refs[n % len(refs)]).payload == b"x"
                result["alternating_reads"] = {
                    "extents": len(refs),
                    "records_per_extent": 128,
                    "calls": 400,
                    "seconds": time.perf_counter() - start,
                    "logical_read_bytes": logical_reads() - before,
                }
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--parent", type=Path, default=Path(tempfile.gettempdir()))
    parser.add_argument("--count", type=int, default=1000)
    parser.add_argument("--online-gc", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.count <= 100000:
        parser.error("count must be in 1..100000")
    result = measure(args.parent, args.count, args.online_gc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
