"""Bytes, JSON, a custom codec, packed publications and explicit reclamation."""

import json
from pathlib import Path
from tempfile import TemporaryDirectory

from straw import Publication, Record, SharedFilesystemStore


def main():
    with TemporaryDirectory(prefix="straw-records-") as root:
        with SharedFilesystemStore(
            root, "records", online_gc=True, codecs=("bytes.v1", "json.v1", "document.v1")
        ) as store:
            refs = store.publish_many(
                [
                    Publication((Record("text", "hello 世界".encode()),)),
                    Publication((Record("json", json.dumps({"count": 3}).encode(), "json.v1"),)),
                    Publication((Record("document", b"application-defined bytes", "document.v1"),)),
                ],
                submission_id="batch-1",
            )
            assert next(store.read(refs[0])).payload.decode() == "hello 世界"
            assert json.loads(next(store.read(refs[1])).payload) == {"count": 3}
            assert next(store.read(refs[2])).codec == "document.v1"
            assert len(list(Path(root).rglob("*.pack"))) == 1

            store.retain("application:checkpoint", refs)
            store.release_publications(refs)
            store.seal()
            with store.pin(refs):
                store.release("application:checkpoint")
                assert store.collect_garbage()["reclaimed_files"] == 0
                assert next(store.read(refs[0])).payload.decode() == "hello 世界"
            assert store.collect_garbage()["reclaimed_files"] == 1
    print("Verified bytes, JSON, custom codec, one packed file, reader pin and GC.")


if __name__ == "__main__":
    main()
