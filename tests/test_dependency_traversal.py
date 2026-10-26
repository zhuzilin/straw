"""Dependency depth consumes heap worklists, not one native stack frame per node."""

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

from straw import Record, SharedFilesystemStore
from straw.errors import InvalidReference, StorageUnavailable

PROBE = Path(__file__).resolve().parents[1] / "verification/storage/probes/dependency_probe.py"


def prepare(*args, **kwargs):
    spec = importlib.util.spec_from_file_location("dependency_probe", PROBE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.prepare(*args, **kwargs)


@pytest.mark.parametrize("mode", ["validate", "retain"])
def test_deep_valid_dependency_graph_on_small_stack(tmp_path, mode):
    result = subprocess.run(
        [
            sys.executable,
            "-I",
            str(PROBE),
            "--root",
            str(tmp_path / mode),
            "--depth",
            "2048",
            "--mode",
            mode,
            "--stack-kib",
            "512",
        ],
        text=True,
        capture_output=True,
        timeout=60,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["passed"]


@pytest.mark.parametrize("reopen", [False, True])
def test_graph_closure_preserves_dependencies_and_late_retain_fence(tmp_path, reopen):
    store, reference, leaf, raw = prepare(tmp_path, 32)
    orphan = store.publish([Record("unreachable", b"unreachable")], submission_id="orphan")
    store.seal()
    store.retain("root", [reference])
    store.release_publications([*raw, leaf, orphan])
    if reopen:
        store.close()
        store = SharedFilesystemStore(tmp_path, "dependency-probe", online_gc=True)
    try:
        store.retain("second", [reference])
        store.release("root")
        assert store.collect_garbage()["reclaimed_files"] == 1
        assert list(store.read(leaf))[0].payload == b"x" * 1024
        store.release("second")
        assert store.collect_garbage()["reclaimed_files"] == 2
        with pytest.raises(InvalidReference, match="reclaimed"):
            store.retain("late", [reference])
    finally:
        store.close()


def test_validation_reaches_a_malformed_transitive_dependency(tmp_path):
    store, reference, _, _ = prepare(tmp_path, 32, corrupt_leaf=True)
    try:
        with pytest.raises(InvalidReference, match="Logical result digest mismatch"):
            store.validate(reference)
    finally:
        store.close()


def test_failed_cold_closure_retry_retains_every_dependency(tmp_path):
    options = dict(online_gc=True, segment_target_bytes=0)
    with SharedFilesystemStore(tmp_path, "closure-retry", **options) as writer:
        leaf = writer.publish([Record("leaf", b"protected leaf")], submission_id="leaf")
        root = writer.publish([Record("root", b"root")], submission_id="root", dependencies=(leaf,))
        writer.seal()
    leaf_path = tmp_path / leaf.manifest.segment.path
    paths = {leaf_path, tmp_path / root.manifest.segment.path}
    assert len(paths) == 2
    hidden = leaf_path.with_suffix(".injected-missing")
    with SharedFilesystemStore(tmp_path, "closure-retry", **options) as reader:
        leaf_path.rename(hidden)
        try:
            with pytest.raises(StorageUnavailable):
                reader.retain("reader", [root])
        finally:
            hidden.rename(leaf_path)
        reader.retain("reader", [root])
        reader.release_publications([root, leaf])
        assert reader.collect_garbage()["reclaimed_files"] == 0
        assert next(reader.read(leaf)).payload == b"protected leaf"
        reader.release("reader")
        assert reader.collect_garbage()["reclaimed_files"] == len(paths)
