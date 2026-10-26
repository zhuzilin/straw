"""Staging release retries survive removal of all their manifest files."""

import errno

import pytest
from test_catalog_encoding import events, write_events

from straw import Record, SharedFilesystemStore
from straw.errors import CorruptData, StorageUnavailable

PACKED = 64 * 1024 * 1024


def store(root, target=0):
    return SharedFilesystemStore(root, "consumption", online_gc=True, segment_target_bytes=target)


def make(s, api):
    raw = s.write_records([Record("raw", b"x" * 2048)], submission_id="raw")
    if api == "write_records":
        return raw
    dep = s.publish([Record("dep", b"dep")], submission_id="dep")
    if api == "publish":
        return [s.publish([Record("data", b"x" * 2048)], submission_id="result", dependencies=[dep]), *raw]
    return [s.record_set(raw, submission_id="result", dependencies=[dep])]


CUTS = [
    ("before_storage_catalog_write", 1, False),
    ("after_storage_catalog_part", 1, False),
    ("after_storage_catalog_part", 2, False),
    ("after_storage_catalog_part", 3, True),
    ("before_storage_catalog_sync", 1, True),
    ("after_storage_catalog_sync", 1, True),
]


@pytest.mark.parametrize("api", ["write_records", "publish", "record_set"])
@pytest.mark.parametrize("target", [0, PACKED])
@pytest.mark.parametrize("phase,occurrence,complete", CUTS)
def test_uncertain_consumption_survives_peer_gc_and_fresh_replay(tmp_path, api, target, phase, occurrence, complete):
    with store(tmp_path, target) as s:
        refs = make(s, api)
        s.seal()
        seen = 0

        def fail(at):
            nonlocal seen
            if at == phase:
                seen += 1
                if seen == occurrence:
                    raise OSError(errno.EIO, "consume response lost")

        s.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="consume response lost"):
            s.release_publications(refs)
        s.backend.fault = lambda _: None
        with store(tmp_path) as peer:
            gc = peer.collect_garbage()
            assert (gc["owners"] == 0) == complete
            assert (gc["reclaimed_files"] > 0) == complete
        with store(tmp_path) as recovered:
            phases = []
            recovered.backend.fault = phases.append
            recovered.release_publications([*refs, *refs])
            if complete:
                assert "before_staging_manifest_read" not in phases
                assert "before_storage_catalog_write" not in phases
            assert recovered.collect_garbage()["owners"] == 0
            before = (tmp_path / "storage.log").read_bytes()
            recovered.release_publications(list(reversed(refs)))
            assert (tmp_path / "storage.log").read_bytes() == before
    assert not [p for p in tmp_path.rglob("*") if p.suffix in (".pack", ".sealed", ".partial")]


@pytest.mark.parametrize("depth", [1, 4, 12])
@pytest.mark.parametrize("target", [0, PACKED])
def test_direct_edges_are_linear_and_shared_roots_are_deduplicated(tmp_path, depth, target):
    with store(tmp_path, target) as s:
        refs = []
        for i in range(depth):
            refs.append(s.publish([Record(str(i), b"x" * 2048)], submission_id=str(i), dependencies=refs[-1:]))
        s.seal()
        phases = []
        s.backend.fault = phases.append
        s.release_publications([refs[-1], refs[-1], refs[0]])
        assert phases.count("before_staging_manifest_read") == depth
        graph = {}
        for event in events(tmp_path / "storage.log"):
            graph.update(event.get("staging_edges", {}))
        assert len(graph) == depth * 2
        assert sum(map(len, graph.values())) == depth * 2 - 1
        assert all(child in graph for children in graph.values() for child in children)
        assert s.collect_garbage()["owners"] == 0
    with store(tmp_path) as recovered:
        for ref in refs:
            recovered.release_publications([ref])
        assert recovered.collect_garbage()["reclaimed_files"] == 0


@pytest.mark.parametrize("count", [1, 512])
@pytest.mark.parametrize("target", [0, PACKED])
def test_direct_raw_releases_do_not_persist_unnecessary_dependency_nodes(tmp_path, count, target):
    with store(tmp_path, target) as s:
        refs = s.write_records([Record(str(i), b"x" * 2048) for i in range(count)], submission_id="raw")
        s.seal()
        s.release_publications(refs)
        assert not any("staging_edges" in event for event in events(tmp_path / "storage.log"))
        assert s.collect_garbage()["owners"] == 0
        before = (tmp_path / "storage.log").read_bytes()
        s.release_publications([*refs, *refs])
        assert (tmp_path / "storage.log").read_bytes() == before


def test_raw_root_also_reached_from_manifest_is_recorded_once(tmp_path):
    with store(tmp_path) as s:
        raw = s.write_records([Record("data", b"x")], submission_id="raw")
        result = s.record_set(raw, submission_id="result")
        s.release_publications([*raw, result, *raw])
        assert s.collect_garbage()["owners"] == 0
    with store(tmp_path) as recovered:
        recovered.release_publications([result])


def test_empty_manifest_still_records_its_recipe(tmp_path):
    with store(tmp_path) as s:
        result = s.record_set([], submission_id="empty")
        s.release_publications([result])
        assert s.collect_garbage()["reclaimed_files"] == 1
    with store(tmp_path) as recovered:
        recovered.release_publications([result])


@pytest.mark.parametrize("target", [0, PACKED])
def test_retry_consumes_restaged_descendant_after_root_was_collected(tmp_path, target):
    with store(tmp_path, target) as child_writer, store(tmp_path, target) as root_writer:
        child = child_writer.publish([Record("child", b"keep")], submission_id="child")
        root = root_writer.publish([Record("root", b"root")], submission_id="root", dependencies=[child])
        root_writer.retain("reader", [child])
        root_writer.seal()
        root_writer.release_publications([root])
        assert root_writer.collect_garbage()["reclaimed_files"] == 1
        # The original writer's acknowledged cache reestablishes child staging.
        assert child_writer.publish([Record("child", b"keep")], submission_id="child") == child
        child_writer.seal()
        with store(tmp_path) as recovered:
            recovered.release_publications([root])
            assert recovered.collect_garbage()["owners"] == 1
            assert next(recovered.read(child)).payload == b"keep"
            recovered.release("reader")
            assert recovered.collect_garbage()["reclaimed_files"] == 1


@pytest.mark.parametrize("phase", ["before_staging_manifest_read", "after_staging_manifest_read"])
@pytest.mark.parametrize("api", ["publish", "record_set"])
def test_manifest_error_cannot_release_a_partial_closure(tmp_path, phase, api):
    with store(tmp_path) as s:
        refs = make(s, api)
        before = (tmp_path / "storage.log").read_bytes()

        def fail(at):
            if at == phase:
                raise OSError(errno.EIO, "manifest unavailable")

        s.backend.fault = fail
        with pytest.raises(StorageUnavailable, match="manifest unavailable"):
            s.release_publications(refs)
        assert (tmp_path / "storage.log").read_bytes() == before
        s.backend.fault = lambda _: None
        assert s.collect_garbage()["reclaimed_files"] == 0
        s.release_publications(refs)
        assert s.collect_garbage()["owners"] == 0


@pytest.mark.parametrize("reclaimed", [False, True])
def test_legacy_consumption_requires_surviving_manifests_to_learn_edges(tmp_path, reclaimed):
    with store(tmp_path) as s:
        refs = make(s, "publish")
        s.release_publications(refs)
        if reclaimed:
            assert s.collect_garbage()["reclaimed_files"] > 0
    path = tmp_path / "storage.log"
    values = events(path)
    for event in values:
        event.pop("staging_edges", None)
    write_events(path, values)
    with store(tmp_path) as recovered:
        if reclaimed:
            with pytest.raises(StorageUnavailable):
                recovered.release_publications(refs)
        else:
            recovered.release_publications(refs)
            assert recovered.collect_garbage()["owners"] == 0
            recovered.release_publications(refs)


@pytest.mark.parametrize("mutation", ["missing_node", "conflict", "bad_key", "bad_children", "bad_map"])
def test_replay_rejects_invalid_persisted_graph_before_releasing_owner(tmp_path, mutation):
    with store(tmp_path) as s:
        refs = make(s, "publish")
        s.retain("reader", refs)
        s.release_publications(refs)
    path = tmp_path / "storage.log"
    values = events(path)
    graph = {}
    for event in values:
        graph.update(event.get("staging_edges", {}))
    root = next(key for key, children in graph.items() if children)
    changes = {
        "missing_node": {"staged:" + "e" * 64: ["staged:" + "f" * 64]},
        "conflict": {root: []},
        "bad_key": {"reader": []},
        "bad_children": {root: None},
        "bad_map": [],
    }
    values.append({"run_id": "consumption", "owners": {"reader": None}, "staging_edges": changes[mutation]})
    write_events(path, values)
    before = path.read_bytes()
    with store(tmp_path) as recovered:
        with pytest.raises(CorruptData, match="staging dependency"):
            recovered.collect_garbage()
    assert path.read_bytes() == before
    assert list(tmp_path.rglob("*.sealed"))
