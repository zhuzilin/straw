"""Measure failed early preparations and their private owner cleanup."""

import argparse
import hashlib
import json
import shutil
import sys
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.errors import InvalidReference, StorageUnavailable, UnsupportedSchema


def store(root, target=0):
    return SharedFilesystemStore(root, "provisional-probe", online_gc=True, segment_target_bytes=target)


def case(root, api, target, size, cut, expected):
    root.mkdir(parents=True, exist_ok=False)
    with store(root, target) as source, store(root, target) as writer, store(root) as keeper:
        raw = source.write_records([Record("raw", b"x" * size)], submission_id="raw")
        dep = source.publish([Record("dep", b"y" * size)], submission_id="dep")
        keep = keeper.publish([Record("keep", b"reader")], submission_id="keep")
        keeper.retain("reader", [keep])
        keeper.release_publications([keep])
        fired = False
        phases = []

        def fail(at):
            nonlocal fired
            phases.append(at)
            point = {"hold_response": "after_storage_catalog_sync", "write": "after_payload_chunk"}.get(cut)
            if at == point and not fired:
                fired = True
                raise OSError(5, "provisional probe interruption")

        writer.backend.fault = fail
        start = time.monotonic()
        try:
            if api == "record_set":
                writer.record_set([*raw, *raw] if cut == "validation" else raw, submission_id="result")
            else:
                writer.publish(
                    [Record("result", b"z" * size, codec="unsupported" if cut == "validation" else "bytes.v1")],
                    submission_id="result",
                    dependencies=[dep],
                )
        except (InvalidReference, UnsupportedSchema, StorageUnavailable) as error:
            first_ms = (time.monotonic() - start) * 1000
            category = type(error).__name__
        else:
            raise AssertionError("Expected failed preparation")
        assert fired == (cut != "validation")
        writer.backend.fault = lambda _: None
        source.release_publications([*raw, dep])
        source.seal()
        during = source.collect_garbage()
        assert during["owners"] == 2  # failed preparation and independent reader
        kept = source.read_record(raw[0]) if api == "record_set" else next(source.read(dep))
        assert kept.payload == (b"x" if api == "record_set" else b"y") * size
        assert "before_provisional_release" not in phases
        start = time.monotonic()
        writer.close()
        close_ms = (time.monotonic() - start) * 1000
        source.release_publications([*raw, dep])
        source.close()
        start = time.monotonic()
        gc = source.collect_garbage()
        assert gc["owners"] == (2 if expected == "baseline" else 1)
        assert next(keeper.read(keep)).payload == b"reader"
        keeper.release("reader")
        final = keeper.collect_garbage()
        gc_ms = (time.monotonic() - start) * 1000
        assert final["owners"] == int(expected == "baseline") and final["open_packs"] == 0
        leftovers = [p for p in root.rglob("*") if p.suffix in (".pack", ".partial", ".sealed")]
        assert len(leftovers) == int(expected == "baseline")
        result = dict(
            api=api,
            target=target,
            size=size,
            cut=cut,
            error=category,
            first_ms=first_ms,
            close_ms=close_ms,
            gc_ms=gc_ms,
            during_gc=during,
            gc=gc,
            final_gc=final,
            provisional_release_calls=phases.count("before_provisional_release"),
            leaked_files=len(leftovers),
            leaked_bytes=sum(p.stat().st_size for p in leftovers),
        )
    # Only this newly allocated case root is removed. Old-binary payload
    # leftovers are explicitly counted; fixed payloads must all pass normal GC.
    shutil.rmtree(root)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--expected", choices=("baseline", "fixed"), required=True)
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    native = Path(_native.__file__).resolve()
    assert native.is_relative_to(Path(sys.prefix).resolve())
    results = []
    for api in ("publish", "record_set"):
        for target in (0, 64 * 1024 * 1024):
            for size in (2048, 8 * 1024 * 1024):
                for cut in ("validation", "hold_response", "write"):
                    for _ in range(args.samples):
                        results.append(case(args.root / str(len(results)), api, target, size, cut, args.expected))
    print(
        json.dumps(
            {
                "passed": True,
                "native_sha256": hashlib.sha256(native.read_bytes()).hexdigest(),
                "native_path": str(native),
                "cases": results,
            }
        )
    )


if __name__ == "__main__":
    main()
