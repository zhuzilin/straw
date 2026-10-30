from pathlib import Path

import pytest

from straw.backend import FilesystemBackend
from straw.errors import InvalidReference


def test_confined_open_preserves_inside_symlinks_and_root_symlink(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    (root / "sub").mkdir()
    (root / "sub/data").write_bytes(b"unchanged")
    (root / "relative").symlink_to("sub/data")
    (root / "absolute").symlink_to(root / "sub/data")
    root_link = tmp_path / "root-link"
    root_link.symlink_to(root, target_is_directory=True)
    backend = FilesystemBackend(root_link)
    for name in ("sub/data", "relative", "absolute"):
        assert backend.open_committed(name).read() == b"unchanged"


@pytest.mark.parametrize("name", ["../outside", "/outside", "sub/../outside", "sub//data", "./data", "sub\\data"])
def test_confined_open_rejects_lexical_escape(tmp_path, name):
    root = tmp_path / "root"
    root.mkdir()
    (tmp_path / "outside").write_bytes(b"outside")
    with pytest.raises(InvalidReference, match="Path escapes"):
        FilesystemBackend(root).open_committed(name)


@pytest.mark.parametrize("absolute", [False, True])
def test_confined_open_rejects_external_symlink(tmp_path, absolute):
    root = tmp_path / "root"
    root.mkdir()
    outside = tmp_path / "outside"
    outside.write_bytes(b"outside")
    (root / "link").symlink_to(outside if absolute else "../outside")
    with pytest.raises(InvalidReference, match="Symlink escapes"):
        FilesystemBackend(root).open_committed("link")


def test_confined_open_closes_both_descriptors(tmp_path):
    if not Path("/proc/self/fd").exists():
        pytest.skip("Linux procfs descriptor audit")
    (tmp_path / "data").write_bytes(b"value")
    backend = FilesystemBackend(tmp_path)
    count = len(list(Path("/proc/self/fd").iterdir()))
    for _ in range(100):
        assert backend.open_committed("data").read() == b"value"
    assert len(list(Path("/proc/self/fd").iterdir())) == count
