"""Deployment declarations and test hooks for native filesystem operations."""

from __future__ import annotations

import io
from pathlib import Path

from . import _native
from .errors import InvalidReference


class FilesystemBackend:
    def __init__(
        self,
        root,
        *,
        profile="local",
        declaration=None,
        fault=None,
    ):
        self.root = Path(root).resolve()
        if profile not in {"local", "juicefs"}:
            raise ValueError(f"Unsupported filesystem profile: {profile}")
        self.profile = profile
        self.declaration = declaration or {}
        self.fault = fault or (lambda phase: None)
        if profile == "juicefs":
            required = {
                "direct_mount": True,
                "writeback": False,
                "open_cache": 0,
                "readdir_cache": False,
            }
            for key, expected in required.items():
                if key not in self.declaration or self.declaration[key] != expected:
                    raise ValueError(f"JuiceFS requires an explicit declaration of {key}={expected!r}")
            if not self.declaration.get("client_version") or not self.declaration.get("durability_description"):
                raise ValueError("Declare JuiceFS client_version and metadata/object storage durability_description")
        self.metrics = {"syncs": 0}

    def path(self, relative: str) -> Path:
        if not isinstance(relative, str):
            raise InvalidReference("Expected a relative POSIX path")
        return Path(_native.filesystem_path(str(self.root), relative))

    def sync_directory(self, path):
        _native.sync_directory(str(path), self.fault)
        self.metrics["syncs"] += 1

    def open_committed(self, relative):
        """Read small control metadata through the native visibility boundary."""
        return io.BytesIO(_native.read_committed(str(self.root), relative))

    def diagnostics(self):
        return {
            "profile": self.profile,
            "declaration": self.declaration,
            "mount_options_automatically_verified": False,
            "multi_machine_admission": "not_verified",
            "directory_sync": "required; errors are fatal",
        }
