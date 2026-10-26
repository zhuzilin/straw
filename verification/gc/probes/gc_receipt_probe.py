"""Cross-client/version receipt replay on one supervisor-owned private pool."""

import argparse
import hashlib
import json
from collections import Counter
from dataclasses import asdict
from pathlib import Path

from straw import Record, SharedFilesystemStore, _native
from straw.protocol import RecordSetRef


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--action", choices=["prepare", "legacy", "finish", "verify"], required=True)
    parser.add_argument("--state", default="{}")
    args = parser.parse_args()
    state = json.loads(args.state)
    if args.action == "prepare" and args.root.exists():
        parser.error("Prepare requires a fresh root")
    effects = Counter()
    with SharedFilesystemStore(args.root, "receipt-interop", online_gc=True) as store:
        if args.action == "prepare":
            live = store.publish([Record("live", b"keep")], submission_id="live")
            store.seal()
            dead = store.publish([Record("dead", b"discard")], submission_id="dead")
            store.seal()
            store.release_publications([dead])
            assert store.collect_garbage()["reclaimed_files"] == 1
            state = {"live": asdict(live)}
        elif args.action == "legacy":
            live = RecordSetRef.from_dict(state["live"])
            assert next(store.read(live)).payload == b"keep"
            assert store.collect_garbage()["reclaimed_files"] == 0
            dead = store.publish([Record("old-writer", b"discard too")], submission_id="old-writer")
            store.seal()
            store.release_publications([dead])
            assert store.collect_garbage()["reclaimed_files"] == 1
            assert next(store.read(live)).payload == b"keep"
        elif args.action == "finish":
            store.backend.fault = lambda at: effects.update([at])
            result = store.collect_garbage()
            assert result["reclaimed_files"] == 0 and result["tombstones"] == 2
            assert (
                effects["before_gc_stat"] == effects["before_gc_directory_sync"] == effects["before_gc_receipt"] == 1
            )
            effects.clear()
            assert store.collect_garbage()["reclaimed_files"] == 0
            assert (
                effects["before_gc_stat"] == effects["before_gc_directory_sync"] == effects["before_gc_receipt"] == 0
            )
            live = RecordSetRef.from_dict(state["live"])
            assert next(store.read(live)).payload == b"keep"
            store.release_publications([live])
            assert store.collect_garbage()["reclaimed_files"] == 1
        else:
            store.backend.fault = lambda at: effects.update([at])
            result = store.collect_garbage()
            assert result["reclaimed_files"] == result["reclaimed_bytes"] == 0
            assert result["tombstones"] == 3
            assert (
                effects["before_gc_stat"] == effects["before_gc_directory_sync"] == effects["before_gc_receipt"] == 0
            )
    print(
        json.dumps(
            {
                "passed": True,
                "action": args.action,
                "state": state,
                "native_sha256": hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
                "effects": dict(effects),
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
