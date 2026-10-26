"""Compare legacy and certified catalog replay on identical private fixtures.

Offline completed receipts measure replay work, not payload bandwidth. Optional
LD_PRELOAD counting covers intercepted readlink APIs, not every kernel lookup
or metadata-service RPC. No cache eviction or fault response substitution.
"""

import argparse
import ctypes
import hashlib
import importlib.util
import json
import os
import time
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "path_identity_probe", Path(__file__).with_name("path_identity_probe.py")
)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


def prepare(root, count, encoding):
    fixture = probe.prepare_history(root, count)
    log = root / "storage.log"
    data = log.read_bytes()
    result = bytearray()
    offset = sequence = 0
    while offset < len(data):
        seq, size = probe.struct.unpack_from("<QQ", data, offset + 8)
        assert seq == sequence
        event = json.loads(data[offset + 56 : offset + 56 + size])
        if encoding == "resolved":
            event["path_encoding"] = 1
        else:
            event.pop("path_encoding", None)
        result.extend(probe.frame(sequence, event))
        offset += 96 + size
        sequence += 1
    # No client is active. Rewrite the fixture in place, retaining its inode.
    with log.open("wb") as stream:
        stream.write(result)
        stream.flush()
        os.fsync(stream.fileno())
    fixture.update(
        encoding=encoding,
        catalog_bytes=len(result),
        catalog_sha256=hashlib.sha256(result).hexdigest(),
    )
    probe.save(root / "history.json", fixture)
    return fixture


def measure(root, samples):
    fixture = json.loads((root / "history.json").read_text())
    library = ctypes.CDLL(None)
    counted = hasattr(library, "straw_lookup_count")
    if counted:
        library.straw_lookup_count.restype = ctypes.c_ulong
        os.environ["STRAW_LOOKUP_ROOT"] = str(root)

    def healthy(result):
        assert result["reclaimed_files"] == result["open_packs"] == 0
        assert result["owners"] == result["retained_packs"] == 1
        assert result["tombstones"] == fixture["count"]

    def sample(reader):
        if counted:
            library.straw_lookup_reset()
        start = time.perf_counter()
        result = reader.collect_garbage()
        elapsed = (time.perf_counter() - start) * 1000
        calls = library.straw_lookup_count() if counted else None
        healthy(result)
        return {"ms": elapsed, "readlink_calls": calls}

    cold = []
    for _ in range(samples):
        with probe.store(root) as reader:
            cold.append(sample(reader))
    with probe.store(root) as reader:
        healthy(reader.collect_garbage())
        warm = [sample(reader) for _ in range(samples)]
    assert hashlib.sha256((root / "storage.log").read_bytes()).hexdigest() == fixture["catalog_sha256"]
    return {**fixture, "counted": counted, "cold": cold, "warm": warm}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--action", choices=("prepare", "measure", "cleanup"), required=True)
    parser.add_argument("--encoding", choices=("legacy", "resolved"), default="resolved")
    parser.add_argument("--count", type=int, default=4096)
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    if args.count < 0 or args.samples < 1:
        parser.error("count must be nonnegative and samples positive")
    if args.action == "prepare":
        result = prepare(args.root, args.count, args.encoding)
    elif args.action == "measure":
        result = measure(args.root, args.samples)
    else:
        probe.step(args.root, "drop-staging")
        result = probe.step(args.root, "collect")
        assert result["result"]["reclaimed_files"] == 1
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(Path(probe._native.__file__).read_bytes()).hexdigest(),
                **result,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
