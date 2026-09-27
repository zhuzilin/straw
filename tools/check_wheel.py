"""Install and exercise a wheel without source imports or Rust on the test PATH.

Runtime NumPy/PyTorch dependencies must already be installed in this interpreter.
They are reused through a temporary venv; the straw package comes from the wheel.
No package index is contacted by this check.
"""

import argparse
import email
import hashlib
import json
import os
import re
import subprocess
import tempfile
import venv
import zipfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument(
        "--tests", action="store_true", help="Also run core regressions and protocol campaigns; needs pytest"
    )
    args = parser.parse_args()
    wheel = args.wheel.resolve()
    source = Path(__file__).resolve().parents[1]
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()
        forbidden = {"__pycache__", "reports", "target", ".git"}
        assert not any(forbidden.intersection(Path(name).parts) for name in names), names
        extensions = [name for name in names if name.startswith("straw/_native") and name.endswith(".so")]
        assert len(extensions) == 1, extensions
        metadata = email.message_from_bytes(archive.read(next(n for n in names if n.endswith("/METADATA"))))
        assert metadata["Name"] == "straw-queue"
        for name in names:
            if ".dist-info/" in name and name.endswith(".json"):
                assert b"path+file://" not in archive.read(name), f"Local build path in wheel metadata: {name}"

    with tempfile.TemporaryDirectory(prefix="straw-wheel-check-") as directory:
        root = Path(directory)
        environment = root / "venv"
        venv.EnvBuilder(with_pip=True, system_site_packages=True).create(environment)
        python = environment / "bin/python"
        env = {k: v for k, v in os.environ.items() if k not in {"PYTHONPATH", "PYTHONHOME", "STRAW_TRACE_DIR"}}
        env["PATH"] = str(environment / "bin")

        def run(*command):
            subprocess.run([str(python), "-I", *map(str, command)], env=env, cwd=root, check=True, timeout=180)

        run("-m", "pip", "install", "--no-index", "--no-deps", "--only-binary=:all:", "--force-reinstall", wheel)
        run(
            "-c",
            "import pathlib, shutil, sys, straw; "
            "assert pathlib.Path(straw.__file__).is_relative_to(sys.prefix), straw.__file__; "
            "assert all(shutil.which(x) is None for x in ('rustc', 'cargo', 'maturin')); "
            "print('Installed wheel:', straw.__file__, 'without Rust on PATH')",
        )
        for script in ("records.py", "tensors.py", "work_queue.py"):
            run(source / "examples" / script)
        run(source / "examples/multiprocess.py", "--root", root / "recovery")
        for snippet in re.findall(r"```python\n(.*?)```", (source / "README.md").read_text(), re.S):
            run("-c", snippet)
        if args.tests:
            run("-m", "pytest", "-q", source / "tests", "--basetemp", root / "pytest")
            run(source / "verification/ownership_model.py", "--output", root / "model.json")
            run(source / "verification/crash_campaign.py", "--parent", root, "--output", root / "crashes.json")
            assert json.loads((root / "model.json").read_text())["protocol"]["passed"]
            assert json.loads((root / "crashes.json").read_text())["passed"]

        env["STRAW_TRACE_DIR"] = str(root / "trace")
        run(
            "-m",
            "straw.benchmark",
            "run",
            "--local",
            "--root",
            root / "benchmark",
            "--report",
            root / "benchmark.json",
            "--gib",
            "0.008",
            "--record-bytes",
            "65536",
            "131072",
            "--segment-mib",
            "2",
            "--online-gc",
            "--max-files",
            "64",
            "--max-gib",
            "0.1",
            "--timeout",
            "60",
        )
        run("-m", "straw.benchmark", "profile", "--trace", root / "trace", "--output", root / "profile.json")
        measured = json.loads((root / "benchmark.json").read_text())
        profile = json.loads((root / "profile.json").read_text())
        assert measured["status"] == "passed" and measured["written_bytes"] == measured["read_bytes"]
        assert measured["data_files"] < measured["publications"]
        assert measured["gc"]["reclaimed_files"] == measured["data_files"]
        assert profile["publications"] and not (root / "benchmark").exists()
        report = {
            "wheel": wheel.name,
            "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
            "version": metadata["Version"],
            "native_extension": extensions[0],
            "passed": True,
            "runtime_dependencies": "existing NumPy/PyTorch reused; no Rust or maturin on test PATH",
            "checks": [
                "no local source paths in generated metadata",
                "binary-only install",
                "installed-package import",
                "records",
                "tensors/COW",
                "queue/GC",
                "multiprocess crash/recovery",
                "README Python examples",
                "local benchmark/GC",
                "trace profile",
            ],
            "benchmark_payload_bytes": measured["written_bytes"],
            "benchmark_publications": measured["publications"],
            "benchmark_data_files": measured["data_files"],
            "benchmark_root_removed": True,
            "core_and_protocol_checks": args.tests,
        }
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
