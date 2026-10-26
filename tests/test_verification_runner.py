"""The release matrix must cover every control and cannot omit the positive proof."""

import importlib.util
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
