"""Failed unpublished writes must not pin private files or dead pack tails."""

import errno
import hashlib
import multiprocessing
import os
import threading
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import IndeterminateCommit, StorageUnavailable

PACKED = 64 * 1024 * 1024


def make(root, target=PACKED):
    return SharedFilesystemStore(root, "incomplete", online_gc=True, segment_target_bytes=target)


@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize("api", ["write_records", "publish", "record_set"])
@pytest.mark.parametrize("phase", ["after_segment_header", "after_payload_chunk", "after_record", "before_footer"])
def test_failed_extent_is_removed_without_touching_committed_prefix(tmp_path, target, api, phase):
    with make(tmp_path, target) as writer:
        good = writer.write_records([Record("good", b"g" * 2048)], submission_id="good")
        path = tmp_path / good[0].segment.path
        prefix = path.read_bytes()

        def publish():
            if api == "record_set":
                return [writer.record_set(good, submission_id="retry")]
            records = [Record("retry", b"x" * 8192)]
            return (
                writer.write_records(records, submission_id="retry")
                if api == "write_records"
                else [writer.publish(records, submission_id="retry")]
            )

        def fail(observed):
            if observed == phase:
                raise OSError(errno.EIO, "incomplete write")

        writer.backend.fault = fail
        for _ in range(2):
            with pytest.raises(StorageUnavailable, match="incomplete write"):
                publish()
            assert not list(tmp_path.rglob("*.partial"))
            assert path.read_bytes() == prefix
            assert writer.read_record(good[0]).payload == b"g" * 2048
        writer.backend.fault = lambda _: None
        refs = publish()
        segment = refs[0].segment if api == "write_records" else refs[0].manifest.segment
        assert segment.offset == (len(prefix) if target else 0)
        assert (writer.read_record(refs[0]) if api == "write_records" else next(writer.read(refs[0]))).payload == (
            b"g" * 2048 if api == "record_set" else b"x" * 8192
        )
        writer.seal()
        writer.release_publications([*good, *refs])
        gc = writer.collect_garbage()
        assert gc["open_packs"] == gc["owners"] == 0
        assert gc["reclaimed_files"] == (2 if target == 0 else 1)
        assert not [p for p in tmp_path.rglob("*") if p.suffix in (".partial", ".pack", ".sealed")]


@pytest.mark.parametrize(
    "target,cleanup_phase",
    [
        (0, "before_incomplete_unlink"),
        (0, "after_incomplete_unlink"),
        (0, "before_directory_sync"),
        (0, "before_incomplete_seal"),
        (0, "after_incomplete_seal"),
        (PACKED, "before_incomplete_truncate"),
        (PACKED, "after_incomplete_truncate"),
        (PACKED, "before_file_sync"),
    ],
)
def test_failed_cleanup_fences_reuse_and_retries_after_peer_gc(tmp_path, target, cleanup_phase):
    with make(tmp_path, target) as writer:
        good = writer.write_records([Record("good", b"good")], submission_id="good")
        interrupted = False
        starts = 0

        def fail(phase):
            nonlocal interrupted, starts
            if phase == "after_segment_header":
                interrupted = True
                starts += 1
                raise OSError(errno.EIO, "incomplete write")
            if interrupted and phase == cleanup_phase:
                raise OSError(errno.EIO, "cleanup unavailable")

        writer.backend.fault = fail
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.write_records([Record("retry", b"data")], submission_id="retry")
        # A seal may have persisted before its error. Peer GC can permanently
        # tombstone that unexposed final name while cleanup is still pending.
        with make(tmp_path) as peer:
            gc = peer.collect_garbage()
            assert gc["reclaimed_files"] == 0
            if cleanup_phase == "after_incomplete_seal":
                assert gc["tombstones"] >= 1
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.write_records([Record("other", b"other")], submission_id="other")
        with pytest.raises(IndeterminateCommit, match="cleanup"):
            writer.seal()
        assert starts == 1
        writer.backend.fault = lambda _: None
        retry = writer.write_records([Record("retry", b"data")], submission_id="retry")
        assert writer.read_record(good[0]).payload == b"good"
        assert writer.read_record(retry[0]).payload == b"data"
        writer.seal()
        writer.release_publications([*good, *retry])
        assert writer.collect_garbage()["open_packs"] == 0
        assert not list(tmp_path.rglob("*.partial"))


@pytest.mark.parametrize("offset", [2**31 - 5, 2**32 - 5, 2**41 - 5])
def test_rollback_preserves_sparse_large_prefix_exactly(tmp_path, offset):
    with make(tmp_path, 8 * 2**40) as writer:
        good = writer.write_records([Record("good", b"good")], submission_id="good")
        path = tmp_path / good[0].segment.path
        with path.open("r+b") as stream:
            stream.truncate(offset)
            stream.flush()
            os.fsync(stream.fileno())

        def fail(phase):
            if phase == "after_payload_chunk":
                raise OSError(errno.EIO, "incomplete write")

        writer.backend.fault = fail
        with pytest.raises(StorageUnavailable):
            writer.write_records([Record("retry", b"x" * 2048)], submission_id="retry")
        assert path.stat().st_size == offset
        assert writer.read_record(good[0]).payload == b"good"
        writer.backend.fault = lambda _: None
        (ref,) = writer.write_records([Record("retry", b"x" * 2048)], submission_id="retry")
        assert ref.segment.offset == offset
        assert (
            writer.read_range(ref, 7, 23, chunk_bytes=2048, checksums=[hashlib.sha256(b"x" * 2048).hexdigest()])
            == b"x" * 16
        )
        writer.seal()
        writer.release_publications([*good, ref])
        assert writer.collect_garbage()["reclaimed_bytes"] == offset + ref.segment.size


@pytest.mark.parametrize("operation", ["seal", "close", "metrics", "collect", "write", "publish", "record_set"])
def test_inherited_writer_rejects_before_mutex_or_cleanup(tmp_path, operation):
    if "fork" not in multiprocessing.get_all_start_methods():
        pytest.skip("requires fork")
    context = multiprocessing.get_context("fork")
    with make(tmp_path) as writer:
        good = writer.write_records([Record("good", b"good")], submission_id="good")
        entered, proceed = threading.Event(), threading.Event()
        errors = []

        def pause(phase):
            if phase == "after_segment_header":
                entered.set()
                assert proceed.wait(15)
                raise OSError(errno.EIO, "abort parent append")

        writer.backend.fault = pause

        def append():
            try:
                writer.write_records([Record("retry", b"data")], submission_id="retry")
            except Exception as error:
                errors.append(error)

        worker = threading.Thread(target=append)
        worker.start()
        receive, send = context.Pipe(duplex=False)

        def child():
            try:
                if operation == "metrics":
                    writer.metrics
                elif operation == "collect":
                    writer.collect_garbage()
                elif operation == "write":
                    writer.write_records([Record("child", b"data")], submission_id="child")
                elif operation == "publish":
                    writer.publish([Record("child", b"data")], submission_id="child")
                elif operation == "record_set":
                    writer.record_set(good, submission_id="child")
                else:
                    getattr(writer, operation)()
            except Exception as error:
                send.send(str(error))
            else:
                send.send("unexpected success")
            finally:
                send.close()

        process = context.Process(target=child)
        try:
            assert entered.wait(5)
            process.start()
            send.close()
            assert receive.poll(3), "Child waited on an inherited writer mutex"
            assert "cannot be reused after fork" in receive.recv()
            process.join(timeout=3)
            assert process.exitcode == 0
        finally:
            if process.pid is not None and process.is_alive():
                process.kill()
                process.join(timeout=3)
            proceed.set()
            worker.join(timeout=5)
            receive.close()
            send.close()
            writer.backend.fault = lambda _: None
        assert not worker.is_alive()
        assert len(errors) == 1 and "abort parent append" in str(errors[0])
        assert writer.read_record(good[0]).payload == b"good"
        writer.seal()
        writer.release_publications(good)
        assert writer.collect_garbage()["reclaimed_files"] == 1
