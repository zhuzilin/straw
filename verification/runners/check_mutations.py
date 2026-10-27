"""Check real Rust mutations in temporary copies against named regressions.

Build errors, missing tests and timeouts never count as detected mutations.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MUTATIONS = (
    (
        "registry_reuses_uncertain_state",
        "rust/writer_gate.rs",
        r"if registry\.ready\s*&& !registry\.journal\.poisoned\s*&& fresh\.metadata\(\)\?\.len\(\) == registry\.journal\.state\.offset",
        "if true",
        "writer_gate::tests::registry_reuses_only_confirmed_state_and_rebuilds_after_caught_unwind",
    ),
    (
        "directory_cached_before_sync",
        "rust/store.rs",
        r"mkdir\(path, fault\)\?;\s*self\.directory = Some\(\(path\.to_path_buf\(\), File::open\(path\)\?\)\);",
        "fs::create_dir_all(path)?;\n        self.directory = Some((path.to_path_buf(), File::open(path)?));\n        mkdir(path, fault)?;",
        "store::directory_tests::writer_reuses_only_fully_synced_directory_identity",
    ),
    (
        "gc_skips_parent_barriers",
        "rust/gc_batch.rs",
        re.escape("effects.sync_parent(parent).map_err(Failure::Effect)?;"),
        "let _ = parent;",
        "gc::replay_tests::collection_binds_shared_parent_barrier_to_complete_receipt_targets",
    ),
)


def run(command, source, environment, log, timeout):
    with log.open("w") as output:
        process = subprocess.run(
            command, cwd=source, env=environment, stdout=output, stderr=subprocess.STDOUT, timeout=timeout
        )
    return process.returncode, log.read_text()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/implementation-mutations")
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(Path(environment.get("CARGO_TARGET_DIR", ROOT / "target")).resolve())
    report = {"passed": False, "baseline_passed": False, "mutations": []}
    try:
        with tempfile.TemporaryDirectory(prefix="straw-mutations-") as private:
            source = Path(private)
            for name in ("Cargo.toml", "Cargo.lock", "rust", "verification", "tests", "README.md"):
                origin = ROOT / name
                if origin.is_dir():
                    shutil.copytree(origin, source / name, ignore=shutil.ignore_patterns("__pycache__"))
                else:
                    shutil.copy2(origin, source / name)
            command = ["cargo", "test", "--locked", "--lib"]
            code, log = run(command, source, environment, output / "baseline.log", args.timeout)
            report["baseline_passed"] = code == 0 and all(f"test {m[4]} ... ok" in log for m in MUTATIONS)
            if not report["baseline_passed"]:
                raise RuntimeError("Unmodified baseline failed or omitted a required test")
            for name, filename, pattern, replacement, test in MUTATIONS:
                path = source / filename
                original = path.read_text()
                changed, matches = re.subn(pattern, lambda _: replacement, original)
                if matches != 1:
                    raise RuntimeError(f"{name}: expected one mutation site, found {matches}")
                try:
                    path.write_text(changed)
                    code, log = run(
                        command + [test, "--", "--exact"], source, environment, output / f"{name}.log", args.timeout
                    )
                finally:
                    path.write_text(original)
                detected = (
                    code == 101
                    and f"test {test} ... FAILED" in log
                    and "test result: FAILED. 0 passed; 1 failed" in log
                )
                report["mutations"].append(
                    {
                        "name": name,
                        "file": filename,
                        "test": test,
                        "detected": detected,
                        "original_sha256": hashlib.sha256(original.encode()).hexdigest(),
                        "mutated_sha256": hashlib.sha256(changed.encode()).hexdigest(),
                    }
                )
                print(f"{'PASS' if detected else 'FAIL'} {name}", flush=True)
            report["passed"] = all(case["detected"] for case in report["mutations"])
    finally:
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
