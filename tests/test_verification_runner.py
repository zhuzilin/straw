"""The release matrix must cover every control and cannot omit the positive proof."""

import hashlib
import importlib.util
import io
import json
import sys
from collections import Counter
from pathlib import Path

import pytest


@pytest.fixture(scope="module")
def runner():
    return load_runner("check_verus")


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


@pytest.mark.parametrize("count", [1, 2, 16, 100])
def test_shards_cover_every_negative_once_and_always_verify_production(runner, count):
    configured = [("production", {}, False)] + [(f"negative_{i}", {}, True) for i in range(53)]
    shards = [runner.select_cases(configured, shard_index=i, shard_count=count) for i in range(count)]
    assert all([case[0] for case in shard if not case[2]] == ["production"] for shard in shards)
    controls = Counter(case[0] for shard in shards for case in shard if case[2])
    assert controls == Counter(case[0] for case in configured if case[2])


@pytest.mark.parametrize("index,count", [(-1, 16), (16, 16), (0, 0), (0, -1)])
def test_invalid_shards_are_rejected(runner, index, count):
    with pytest.raises(ValueError):
        runner.select_cases([("production", {}, False)], shard_index=index, shard_count=count)


def test_named_controls_cannot_remove_positive_or_silently_select_nothing(runner):
    configured = [("production", {}, False), ("first", {}, True), ("second", {}, True)]
    assert [case[0] for case in runner.select_cases(configured, ["second"])] == ["production", "second"]
    with pytest.raises(ValueError):
        runner.select_cases(configured, ["missing"])
    with pytest.raises(ValueError):
        runner.select_cases(configured, ["second"], shard_count=16)
    with pytest.raises(ValueError):
        runner.select_cases([("first", {}, True)])


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


@pytest.mark.parametrize(
    "output,status,negative,timeout,passed",
    [
        ("verification results:: 1008 verified, 0 errors", 0, False, False, True),
        ("verification results:: 1008 verified, 0 errors", 0, True, False, False),
        ("verification results:: 0 verified, 0 errors", 0, False, False, False),
        ("verification results:: 1008 verified, 0 errors", 1, False, False, False),
        ("error: postcondition not satisfied\nverification results:: 0 verified, 1 errors", 1, True, False, True),
        ("error: postcondition not satisfied\nverification results:: 0 verified, 1 errors", 1, False, False, False),
        ("error: postcondition not satisfied\nverification results:: 0 verified, 1 errors", 0, True, False, False),
        ("error: postcondition not satisfied\nverification results:: 0 verified, 1 errors", -9, True, False, False),
        ("error: postcondition not satisfied\nverification results:: 0 verified, 1 errors", None, True, True, False),
        ("error[E0308]: mismatched types", 1, True, False, False),
        ("error: expected expression", 1, True, False, False),
        ("verification results:: 0 verified, 1 errors", 1, True, False, False),
        ('12 | let text = "assertion failed";\nverification results:: 0 verified, 1 errors', 1, True, False, False),
        (
            "error: assertion failed\nResource limit exceeded\nverification results:: 0 verified, 1 errors",
            1,
            True,
            False,
            False,
        ),
        (
            "error: assertion failed\nerror[E0308]: mismatched types\nverification results:: 0 verified, 1 errors",
            1,
            True,
            False,
            False,
        ),
        (
            "error: assertion failed\nthread 'main' panicked at solver\nverification results:: 0 verified, 1 errors",
            1,
            True,
            False,
            False,
        ),
    ],
)
def test_verus_requires_the_expected_proof_outcome(runner, output, status, negative, timeout, passed):
    assert runner.classify_result(output, status, negative, timeout=timeout)["passed"] is passed


@pytest.mark.parametrize(
    "diagnostic",
    [
        "precondition not satisfied",
        "postcondition not satisfied",
        "possible arithmetic underflow/overflow",
        "loop invariant not satisfied",
        "assertion failed",
    ],
)
def test_verus_accepts_semantic_rejections_only_for_negative_controls(runner, diagnostic):
    output = f"error: {diagnostic}\nverification results:: 0 verified, 1 errors"
    assert runner.classify_result(output, 1, True)["passed"]
    assert not runner.classify_result(output, 1, False)["passed"]
