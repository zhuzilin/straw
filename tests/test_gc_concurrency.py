"""Physical GC I/O must not hold the pool-wide catalog lock."""

from concurrent.futures import ThreadPoolExecutor
from threading import Event

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import InvalidReference, StorageUnavailable


def store(root):
    return SharedFilesystemStore(root, "concurrent-gc", online_gc=True, segment_target_bytes=0)


def garbage(writer, name):
    refs = writer.write_records([Record(name, name.encode())], submission_id=name)
    writer.release_publications(refs)
    return refs


def pause_gc(collector, reached, resume):
    def pause(at):
        if at == "after_gc_tombstone":
            reached.set()
            if not resume.wait(10):
                raise RuntimeError("GC pause timed out")

    collector.backend.fault = pause


def test_gc_physical_io_allows_peer_publication_and_rejects_late_retain(tmp_path):
    with store(tmp_path) as writer, store(tmp_path) as collector, store(tmp_path) as peer:
        dead = garbage(writer, "dead")
        live = writer.write_records([Record("live", b"live")], submission_id="live")
        reached, resume = Event(), Event()
        pause_gc(collector, reached, resume)
        with ThreadPoolExecutor(max_workers=2) as executor:
            collecting = executor.submit(collector.collect_garbage)
            try:
                assert reached.wait(5)

                def publish():
                    with pytest.raises(InvalidReference, match="reclaimed"):
                        peer.retain("late", dead)
                    return peer.write_records([Record("peer", b"peer")], submission_id="peer")

                published = executor.submit(publish).result(timeout=5)
                assert peer.read_record(published[0]).payload == b"peer"
                assert peer.read_record(live[0]).payload == b"live"
            finally:
                resume.set()
            assert collecting.result(timeout=5)["reclaimed_files"] == 1


@pytest.mark.parametrize("new_pending", [False, True])
def test_overlapping_collectors_receipt_only_their_completed_plan(tmp_path, new_pending):
    with store(tmp_path) as writer, store(tmp_path) as first, store(tmp_path) as second:
        garbage(writer, "old")
        reached, resume = Event(), Event()
        pause_gc(first, reached, resume)
        with ThreadPoolExecutor(max_workers=2) as executor:
            collecting = executor.submit(first.collect_garbage)
            try:
                assert reached.wait(5)

                def collect_peer():
                    if new_pending:
                        garbage(writer, "new")

                        def stop(at):
                            if at == "before_gc_directory_sync":
                                raise OSError(5, "peer barrier failed")

                        second.backend.fault = stop
                        with pytest.raises(StorageUnavailable, match="peer barrier failed"):
                            second.collect_garbage()
                    else:
                        assert second.collect_garbage()["reclaimed_files"] == 1

                executor.submit(collect_peer).result(timeout=5)
                before = (tmp_path / "storage.log").read_bytes()
            finally:
                resume.set()
            assert collecting.result(timeout=5)["reclaimed_files"] == 0
        if not new_pending:
            assert (tmp_path / "storage.log").read_bytes() == before
        seen = []
        second.backend.fault = seen.append
        assert second.collect_garbage()["reclaimed_files"] == 0
        assert seen.count("before_gc_unlink") == int(new_pending)
        assert seen.count("before_gc_receipt") == int(new_pending)
