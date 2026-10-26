"""Writer callbacks and paused writes must not trap synchronous accessors."""

import json
import subprocess
import sys
from pathlib import Path

import pytest

PROBE = Path(__file__).resolve().parents[1] / "verification/storage/probes/writer_access_probe.py"


@pytest.mark.parametrize("operation", ["metrics", "seal", "close"])
@pytest.mark.parametrize("mode", ["callback", "thread"])
@pytest.mark.parametrize("target", [0, 64 * 1024 * 1024])
@pytest.mark.parametrize("size", [2048, 8 * 1024 * 1024])
def test_busy_writer_accessor_rejects_without_changing_publication(tmp_path, operation, mode, target, size):
    child = subprocess.run(
        [
            sys.executable,
            "-I",
            str(PROBE),
            "--root",
            str(tmp_path / "pool"),
            "--operation",
            operation,
            "--mode",
            mode,
            "--target",
            str(target),
            "--bytes",
            str(size),
        ],
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert child.returncode == 0, child.stderr
    result = json.loads(child.stdout.splitlines()[-1])
    assert result["passed"] and result["busy_response"]["error"] == "ResourceLimitExceeded"
