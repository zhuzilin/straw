"""Offline cleanup must preserve accepted files and their reference bindings."""

import errno
import importlib.util
import os
from pathlib import Path

import pytest

from straw.errors import QueueError, UnsafeRecovery
from straw.inspect import cleanup_orphans, inspect_run

spec = importlib.util.spec_from_file_location(
    "offline_alias_probe", Path(__file__).resolve().parents[1] / "verification/offline/probes/offline_alias_probe.py"
)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


@pytest.mark.parametrize("mode", probe.MODES)
@pytest.mark.parametrize("canonical", [False, True])
def test_cleanup_preserves_reference_routes_and_removes_real_orphans(tmp_path, mode, canonical):
    assert probe.case(tmp_path / "pool", mode, 2048, canonical, "fixed")["private_removed"]


@pytest.mark.parametrize("error_number", [errno.ENOENT, errno.EIO, errno.EACCES])
def test_unknown_retained_binding_blocks_all_deletion(tmp_path, monkeypatch, error_number):
    root = tmp_path / "pool"
    probe.fixture(root, "partial_file", 2048, False)
    original = os.readlink

    def unavailable(path, *args, **kwargs):
        if Path(path) == root / "alias.pack":
            raise OSError(error_number, "unknown reference binding")
        return original(path, *args, **kwargs)

    monkeypatch.setattr(os, "readlink", unavailable)
    report = inspect_run(root)
    assert not report["deletion_allowed"] and report["retained_dependency_errors"]
    with pytest.raises(UnsafeRecovery):
        cleanup_orphans(root, report, all_participants_stopped=True)


@pytest.mark.parametrize("target", ["outside", "cycle"])
def test_unknown_candidate_identity_cannot_authorize_cleanup(tmp_path, target):
    root = tmp_path / "pool"
    probe.fixture(root, "manifest_file", 2048, False)
    report = inspect_run(root)
    path = root / "raw/aliases/routes/unknown.partial"
    path.symlink_to(tmp_path / "outside" if target == "outside" else path)
    with pytest.raises((QueueError, OSError, ValueError, RuntimeError)):
        inspect_run(root)
    with pytest.raises((QueueError, OSError, ValueError, RuntimeError)):
        cleanup_orphans(root, report, all_participants_stopped=True)
    assert path.is_symlink()
