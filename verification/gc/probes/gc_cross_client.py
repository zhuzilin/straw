"""Small four-client regression: cached attributes after another client's GC."""

import argparse
import json
import time
from pathlib import Path

from straw import Record, SharedFilesystemStore


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--worker")
    args = parser.parse_args()
    root = args.root
    with SharedFilesystemStore(root, root.name, online_gc=True) as store:
        if args.worker is None:
            ref = store.publish([Record("garbage", bytes(1024))], submission_id="garbage")
            store.seal()
            store.release_publications([ref])
            (root / "target.txt").write_text(ref.manifest.segment.path)
            print("prepared")
            return

        target = root / (root / "target.txt").read_text()
        target.stat()
        target.resolve(strict=True)
        (root / ("ready-" + args.worker)).write_text("ready")
        deadline = time.monotonic() + 60
        while not (root / "go").exists():
            if time.monotonic() > deadline:
                raise TimeoutError("barrier")
            time.sleep(0.02)
        try:
            result = store.collect_garbage()
            print(json.dumps(dict(worker=args.worker, passed=True, collection=result)))
        except Exception as error:
            print(json.dumps(dict(worker=args.worker, passed=False, error=repr(error))))
            raise


if __name__ == "__main__":
    main()
