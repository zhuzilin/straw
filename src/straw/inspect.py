"""Conservative offline inspection and explicit stopped-run orphan cleanup."""

import argparse
import hashlib
import json
from pathlib import Path

from .backend import FilesystemBackend
from .errors import QueueError, UnsafeRecovery
from .journal import Journal
from .protocol import RecordRef, RecordSetRef, decode
from .store import SharedFilesystemStore


def inspect_run(root):
    backend = FilesystemBackend(root)
    if (backend.root / "storage.log").exists() or (backend.root / "queues").exists():
        raise UnsafeRecovery(
            "This offline inspector only supports a single queue without a catalog; use the shared storage catalog GC API"
        )
    with backend.open_committed("run.json") as stream:
        identity = decode(stream.read())
    store = SharedFilesystemStore(root, identity["run_id"], codecs=identity["codecs"])
    journal = Journal(backend, identity, recover=True, read_only=True)
    roots, retained, failures, seen = [], set(), [], set()

    def find_refs(value):
        if isinstance(value, dict):
            if set(value) == {
                "manifest",
                "digest",
                "records",
                "payload_bytes",
                "tokens",
            }:
                roots.append(RecordSetRef.from_dict(value))
            else:
                for item in value.values():
                    find_refs(item)
        elif isinstance(value, list):
            for item in value:
                find_refs(item)

    def retain(ref):
        if ref in seen:
            return
        seen.add(ref)
        retained.add(ref.manifest.segment.path)
        try:
            manifest = store.manifest(ref)
            # Record all paths before validating so a damaged dependency can
            # never be downgraded into an apparently deletable orphan.
            for item in manifest["records"]:
                record_ref = RecordRef.from_dict(item)
                retained.add(record_ref.segment.path)
                store.inspect_segment(record_ref.segment, verify_payload=True)
            for dependency in manifest["dependencies"]:
                retain(RecordSetRef.from_dict(dependency))
            store.validate(ref)
        except (QueueError, OSError, KeyError, TypeError, ValueError) as error:
            failures.append(
                {
                    "root": ref.manifest.segment.path,
                    "error": type(error).__name__,
                    "message": str(error),
                }
            )

    try:
        for transaction in journal.transactions:
            find_refs(transaction)
        for ref in roots:
            retain(ref)
        raw = backend.root / "raw"
        sealed = sorted(
            path.relative_to(backend.root).as_posix()
            for pattern in ("*/*/*.sealed", "*/*/*.pack")
            for path in raw.glob(pattern)
        )
        partial = sorted(path.relative_to(backend.root).as_posix() for path in raw.glob("*/*/*.partial"))
        # All historically journal-referenced data are retained in v1. This is
        # deliberately more conservative than only retaining current cursors.
        return {
            "version": 1,
            "run_id": identity["run_id"],
            "retained_paths": sorted(retained),
            "unaccepted_sealed": sorted(set(sealed) - retained),
            "partial": partial,
            "retained_dependency_errors": failures,
            "incomplete_journal_tail_bytes": journal.incomplete_tail_bytes,
            "journal_sha256": hashlib.sha256(journal.path.read_bytes()).hexdigest(),
            "deletion_allowed": not failures and not journal.incomplete_tail_bytes,
            "requires_stopped_run": True,
        }
    finally:
        journal.close()


def cleanup_orphans(root, report, *, all_participants_stopped):
    if not all_participants_stopped:
        raise UnsafeRecovery("Offline cleanup requires stopped coordinator, writers and readers")
    current = inspect_run(root)
    if current != report or not current["deletion_allowed"]:
        raise UnsafeRecovery("Inspection changed or has unresolved dependencies/torn journal; refuse deletion")
    backend = FilesystemBackend(root)
    paths = current["unaccepted_sealed"] + current["partial"]
    for relative in paths:
        path = backend.path(relative)
        path.unlink()
        backend.sync_directory(path.parent)
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root")
    parser.add_argument("--delete-orphans", action="store_true")
    parser.add_argument("--all-participants-stopped", action="store_true")
    args = parser.parse_args()
    report = inspect_run(Path(args.root))
    print(json.dumps(report, indent=2))
    if args.delete_orphans:
        deleted = cleanup_orphans(args.root, report, all_participants_stopped=args.all_participants_stopped)
        print(json.dumps({"deleted": deleted}))


if __name__ == "__main__":
    main()
