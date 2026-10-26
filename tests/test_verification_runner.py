"""The release matrix must cover every control and cannot omit a configured model."""

import hashlib
import importlib.util
import io
import json
import sys
from collections import Counter
from pathlib import Path

import pytest


def load_runner(name):
    directory = Path(__file__).resolve().parents[1] / "verification" / "runners"
    spec = importlib.util.spec_from_file_location(f"straw_{name}_test", directory / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(directory))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


@pytest.mark.parametrize("count", [1, 2, 16, 100])
def test_tla_partitions_cover_every_configured_model_and_control(count):
    tla = load_runner("check_tla")
    configured = list(tla.cases())
    shards = [tla.shard_cases(configured, i, count) for i in range(count)]
    assert Counter(case[0] for shard in shards for case in shard) == Counter(case[0] for case in configured)


@pytest.mark.parametrize("index,count", [(-1, 16), (16, 16), (0, 0), (0, -1)])
def test_invalid_tla_shards_are_rejected(index, count):
    with pytest.raises(ValueError):
        load_runner("check_tla").shard_cases([], index, count)


@pytest.mark.parametrize("existing", [False, True])
def test_tla_download_and_explicit_jar_have_no_checksum_gate(tmp_path, monkeypatch, existing):
    tla = load_runner("check_tla")
    data = b"test jar with no pinned digest"
    jar = tmp_path / "tla2tools.jar"
    arguments = ["check_tla.py", "--output", str(tmp_path), "--case", "ownership"]
    downloads, executions = [], []
    if existing:
        jar.write_bytes(data)
        arguments += ["--jar", str(jar)]

    def download(url, *, timeout):
        downloads.append(url)
        return io.BytesIO(data)

    def run_case(java, selected_jar, output, *args):
        executions.append(selected_jar.read_bytes())
        return {"passed": True}

    monkeypatch.setattr(sys, "argv", arguments)
    monkeypatch.setattr(tla.shutil, "which", lambda name: "java")
    monkeypatch.setattr(tla.urllib.request, "urlopen", download)
    monkeypatch.setattr(tla, "run_case", run_case)
    with pytest.raises(SystemExit) as exit_info:
        tla.main()
    assert exit_info.value.code == 0
    assert downloads == ([] if existing else [tla.TLC_URL])
    assert executions == [data]
    assert json.loads((tmp_path / "report.json").read_text())["tlc_sha256"] == hashlib.sha256(data).hexdigest()
