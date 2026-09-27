import argparse
import json
import os
import subprocess
import sys
import time

import pytest

from straw.benchmark import profile


@pytest.mark.parametrize("checked", [True, False])
def test_profile_preserves_publication_sizes_and_reports_read_amplification(tmp_path, checked):
    mib = 1024**2
    writes = [
        {
            "op": "write",
            "time": timestamp,
            "bytes": size * mib + 128,
            "host": "writer",
            "seconds": 0.1,
            "records": [{"codec": "bytes.v1", "bytes": size * mib}],
        }
        for timestamp, size in [(100.1, 3), (100.9, 5), (101.2, 2)]
    ]
    read = {"op": "read_range", "time": 100.5, "bytes": mib, "host": "reader", "seconds": 0.2}
    if checked:
        read["checked_bytes"] = 4 * mib
    trace = tmp_path / "trace"
    trace.mkdir()
    (trace / "writer.jsonl").write_text("\n".join(json.dumps(e) for e in writes))
    (trace / "reader.jsonl").write_text(json.dumps(read))
    output = tmp_path / "profile.json"
    profile(argparse.Namespace(trace=str(trace), output=str(output)))
    result = json.loads(output.read_text())
    assert result["publications"] == [[3 * mib], [5 * mib], [2 * mib]]
    assert result["observed_write_bytes"] == 10 * mib + 384
    assert result["observed_write_payload_bytes"] == 10 * mib
    assert result["observed_read_bytes"] == mib
    assert result["checked_read_bytes"] == (4 * mib if checked else None)
    assert result["read_to_extent_write_ratio"] == mib / (10 * mib + 384)
    assert result["checked_read_amplification"] == (4 if checked else None)
    one, ten = result["payload_completion_rates"]
    assert one["write_peak_mib_s"] == 8
    assert one["extent_write_peak_mib_s"] == 8 + 256 / mib
    assert ten["write_peak_mib_s"] == 1
    assert one["read_peak_mib_s"] == 1
    assert one["checked_read_peak_mib_s"] == (4 if checked else None)


def test_capacity_failure_stops_workers_and_preserves_queue(tmp_path):
    root = tmp_path / "run"
    report = tmp_path / "result.json"
    with (tmp_path / "driver.log").open("w") as log:
        process = subprocess.Popen(
            [
                sys.executable,
                "-m",
                "straw.benchmark",
                "run",
                "--local",
                "--root",
                str(root),
                "--report",
                str(report),
                "--record-bytes",
                "8",
                "--gib",
                "0.00000001",
                "--writers",
                "1",
                "--readers",
                "1",
                "--max-files",
                "33",
                "--timeout",
                "30",
            ],
            stdout=log,
            stderr=subprocess.STDOUT,
            env={**os.environ, "PYTHONUNBUFFERED": "1"},
        )
        try:
            deadline = time.monotonic() + 20
            while not root.exists() and process.poll() is None and time.monotonic() < deadline:
                time.sleep(0.02)
            assert root.exists()
            # Exhaust the file budget after startup validation, before workers
            # begin. A failed run must retain its original queue and evidence.
            for i in range(40):
                (root / f"capacity-{i}").write_bytes(b"preserve")
            assert process.wait(timeout=45) != 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
    cleanup = json.loads(report.with_suffix(".cleanup.json").read_text())
    assert cleanup["workers_stopped"] and not cleanup["removed"]
    assert (root / "queue/control/journal.log").is_file()
    assert all((root / f"capacity-{i}").read_bytes() == b"preserve" for i in range(40))
    assert "temporary storage limit exceeded" in (tmp_path / "driver.log").read_text()
