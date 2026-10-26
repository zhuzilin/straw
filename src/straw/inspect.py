"""Conservative offline inspection and explicit stopped-run orphan cleanup."""

import argparse
import errno
import hashlib
import json
import os
from collections import deque
from pathlib import Path

from .backend import FilesystemBackend
from .errors import QueueError, UnsafeRecovery
from .journal import Journal
from .protocol import RecordRef, RecordSetRef, decode
from .store import SharedFilesystemStore


def _require_standalone(root):
    # Advisory stat absence cannot authorize deletion. Trust only a fresh
    # read-only open's NotFound, under a stable, stopped storage namespace.
    # Other errors leave the storage layout unknown and must fail closed.
    for name in ("storage.log", "queues"):
        try:
            descriptor = os.open(root / name, os.O_RDONLY)
        except FileNotFoundError:
            continue
        except OSError as error:
            raise UnsafeRecovery(f"Cannot establish standalone storage layout: {name}") from error
        os.close(descriptor)
        raise UnsafeRecovery(
            "This offline inspector only supports a single queue without a catalog; use the shared storage catalog GC API"
        )


def _reference_bindings(root, relative):
    """Trace existing POSIX bindings, including links and paths before '..'.

    The caller separately checks the final in-pool identity. Namespace bindings
    must remain stable throughout inspection and cleanup; this is not fencing.
    """
    pending = deque(Path(relative).parts)
    current, bindings, links = root, {root}, 0
    while pending:
        part = pending.popleft()
        if part == "..":
            current = current.parent
        elif part != ".":
            current = current / part
            bindings.add(current)
            try:
                target = Path(os.readlink(current))
            except OSError as error:
                if error.errno != errno.EINVAL:
                    raise
            else:
                links += 1
                if links > 40:
                    raise UnsafeRecovery("Too many reference symlinks")
                current = Path(target.anchor) if target.is_absolute() else current.parent
                pending.extendleft(reversed(target.parts[1:] if target.is_absolute() else target.parts))
    return {path.relative_to(root).as_posix() for path in bindings if path.is_relative_to(root)}


def inspect_run(root):
    backend = FilesystemBackend(root)
    _require_standalone(backend.root)
    with backend.open_committed("run.json") as stream:
        identity = decode(stream.read())
    store = SharedFilesystemStore(root, identity["run_id"], codecs=identity["codecs"])
    journal = Journal(backend, identity, recover=True, read_only=True)
    roots, retained, failures, seen = [], set(), [], set()
    identities, protected, traced = {}, set(), set()

    def identity_of(relative):
        if relative not in identities:
            identities[relative] = backend.path(relative).resolve().relative_to(backend.root).as_posix()
        return identities[relative]

    def protect(relative):
        retained.add(identity_of(relative))
        if relative not in traced:
            protected.update(_reference_bindings(backend.root, relative))
            traced.add(relative)

    def orphan(relative):
        # Keep lexical names for unlink: resolving an orphan link must never
        # turn its deletion into deletion of the target. Protect both the final
        # identity and every binding traversed by accepted references.
        return relative not in protected and identity_of(relative) not in protected

    def find_refs(value):
        pending = [value]
        while pending:
            item = pending.pop()
            if isinstance(item, dict):
                if set(item) == {"manifest", "digest", "records", "payload_bytes", "tokens"}:
                    roots.append(RecordSetRef.from_dict(item))
                else:
                    pending.extend(item.values())
            elif isinstance(item, list):
                pending.extend(item)

    def failed(path, error):
        failures.append({"root": path, "error": type(error).__name__, "message": str(error)})

    def retain(ref, pending):
        if ref in seen:
            return
        seen.add(ref)
        try:
            protect(ref.manifest.segment.path)
            manifest = store.manifest(ref)
            # Record all paths before validating so a damaged dependency can
            # never be downgraded into an apparently deletable orphan.
            for item in manifest["records"]:
                record_ref = RecordRef.from_dict(item)
                protect(record_ref.segment.path)
                store.inspect_segment(record_ref.segment, verify_payload=True)
            for dependency in manifest["dependencies"]:
                pending.append(RecordSetRef.from_dict(dependency))
        except (QueueError, OSError, KeyError, TypeError, ValueError) as error:
            failed(ref.manifest.segment.path, error)

    try:
        # Recovery also needs these control files. They may legally resolve
        # through names that match the raw orphan enumeration.
        for relative in ("run.json", "control/journal.log"):
            try:
                protect(relative)
            except (QueueError, OSError, ValueError) as error:
                failed(relative, error)
        for transaction in journal.transactions:
            find_refs(transaction)
        pending = list(roots)
        while pending:
            retain(pending.pop(), pending)
        # Native validation checks the complete closure with its own heap
        # frontier. Validate each root once, not every suffix of a deep graph.
        for ref in dict.fromkeys(roots):
            try:
                store.validate(ref)
            except (QueueError, OSError, KeyError, TypeError, ValueError) as error:
                failed(ref.manifest.segment.path, error)
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
            "unaccepted_sealed": [path for path in sealed if orphan(path)],
            "partial": [path for path in partial if orphan(path)],
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
