"""Archive privacy gates and release path remapping, without network or builds."""

import importlib.util
import io
import json
import sys
import tarfile
import zipfile
from pathlib import Path

import pytest

TOOLS = Path(__file__).resolve().parents[1] / "tools"


def load_tool(name):
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


privacy = load_tool("check_artifact_privacy")
release = load_tool("release_build")


def archive(path, entries):
    if path.suffix == ".whl":
        with zipfile.ZipFile(path, "w") as output:
            for name, data in entries.items():
                output.writestr(name, data)
    else:
        with tarfile.open(path, "w:gz") as output:
            for name, data in entries.items():
                member = tarfile.TarInfo(name)
                member.size = len(data)
                output.addfile(member, io.BytesIO(data))
    return path


@pytest.mark.parametrize("suffix", [".whl", ".tar.gz"])
def test_detects_binary_build_paths_without_extracting_or_disclosing_values(tmp_path, suffix, capsys):
    private = b"/".join([b"", b"root", b".cargo", b"registry", b"private-fixture", b"source.rs"])
    path = archive(tmp_path / ("artifact" + suffix), {"module.so": b"\x00prefix" + private + b"\x00" + private})
    before = set(tmp_path.iterdir())
    assert privacy.main([str(path)]) == 1
    output = capsys.readouterr().out
    assert private.decode() not in output
    report = json.loads(output)
    assert report["findings"] == [{"artifact": 1, "member": "module.so", "type": "local_home_path", "count": 2}]
    assert set(tmp_path.iterdir()) == before


def test_allows_public_urls_relative_paths_generic_temporary_examples_and_virtual_roots(tmp_path):
    home = b"/".join([b"home", b"example", b"file.rs"])
    allowed = b"\n".join(
        [
            b"https://example.org/" + home,
            b"docs/" + home,
            b"python run.py --output " + b"/".join([b"", b"tmp", b"example-report.json"]),
        ]
    )
    path = archive(
        tmp_path / "safe.whl",
        {
            "documentation.txt": allowed,
            "module.so": b"/cargo/registry/src/library.rs\x00/rustup/toolchains/lib.rs\x00/straw/rust/lib.rs"
            b"\x00/build-home/config\x00/build-tmp/scratch",
        },
    )
    # Check text separately because native string tables intentionally relax
    # relative-prefix detection to find concatenated compiler source paths.
    assert not privacy.matches(b"docs/" + home)
    assert privacy.scan([path])["passed"]


@pytest.mark.parametrize(
    "value,kind",
    [
        (b"/".join([b"", b"home", b"synthetic-user", b"project", b"lib.rs"]), "local_home_path"),
        (b"/".join([b"", b"github", b"workspace", b"project", b"lib.rs"]), "local_build_path"),
        (b"-----BEGIN " + b"OPENSSH PRIVATE KEY-----", "private_key"),
        (b"gh" + b"p_" + b"a" * 30, "credential"),
        (b"Authorization: " + b"Bearer " + b"a" * 30, "authorization_token"),
    ],
)
def test_rejects_home_ci_and_recognizable_credentials(value, kind):
    assert privacy.matches(value) == {kind: 1}


def test_forbidden_text_and_private_member_names_are_redacted(tmp_path, capsys):
    value = "private-" + "fixture-identifier"
    path = archive(tmp_path / "custom.whl", {f"package/{value}.txt": value.encode()})
    assert privacy.main([str(path), "--forbid", value]) == 1
    output = capsys.readouterr().out
    assert value not in output
    assert json.loads(output)["findings"] == [
        {"artifact": 1, "member": "<member-1>", "type": "forbidden_text", "count": 2}
    ]


def test_tar_symlink_target_is_scanned_without_following_it(tmp_path):
    path = tmp_path / "links.tar.gz"
    with tarfile.open(path, "w:gz") as output:
        member = tarfile.TarInfo("package/link")
        member.type = tarfile.SYMTYPE
        member.linkname = "/".join(["", "home", "synthetic-user", "private"])
        output.addfile(member)
    assert privacy.scan([path])["findings"][0]["type"] == "local_home_path"
    assert not (tmp_path / "package").exists()


def test_unreadable_archive_reports_no_input_path(tmp_path, capsys):
    private = tmp_path / "private-fixture.whl"
    private.write_bytes(b"not an archive")
    assert privacy.main([str(private)]) == 1
    output = capsys.readouterr().out
    assert str(private) not in output and private.name not in output
    assert json.loads(output)["findings"][0]["type"] == "archive_error"


def test_source_distribution_can_include_checker_and_its_tests(tmp_path):
    path = archive(
        tmp_path / "self.tar.gz",
        {
            "tools/check_artifact_privacy.py": (TOOLS / "check_artifact_privacy.py").read_bytes(),
            "tests/test_artifact_privacy.py": Path(__file__).read_bytes(),
        },
    )
    assert privacy.scan([path])["passed"]


@pytest.mark.parametrize("encoded", [None, "", "-C\x1fdebuginfo=1\x1f--cfg=retained"])
def test_remapping_preserves_effective_existing_flags_and_specific_prefix_order(tmp_path, encoded):
    home = tmp_path / "synthetic-home"
    source = home / "project"
    cargo = home / "cargo-cache"
    rustup = home / "toolchains"
    environment = {
        "HOME": str(home),
        "CARGO_HOME": str(cargo),
        "RUSTUP_HOME": str(rustup),
        "RUSTFLAGS": "-C opt-level=2 --cfg=previous",
        "UNCHANGED": "value",
    }
    if encoded is not None:
        environment["CARGO_ENCODED_RUSTFLAGS"] = encoded
    before = dict(environment)
    result = release.build_environment(environment, source)
    expected = (encoded.split("\x1f") if encoded else []) if encoded is not None else before["RUSTFLAGS"].split()
    flags = result["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
    assert flags[: len(expected)] == expected
    assert result["UNCHANGED"] == "value" and "RUSTFLAGS" not in result
    assert environment == before
    mappings = [value.removeprefix("--remap-path-prefix=").split("=", 1) for value in flags[len(expected) :]]
    assert [len(old) for old, _ in mappings] == sorted(len(old) for old, _ in mappings)
    assert (str(source.resolve()), "/straw") in [tuple(value) for value in mappings]
    for path, destination in [(cargo, "/cargo"), (rustup, "/rustup"), (source, "/straw")]:
        selected = [(old, new) for old, new in mappings if str(path.resolve()).startswith(old)][-1]
        assert selected == (str(path.resolve()), destination)


def test_build_wrapper_does_not_echo_command_or_environment(tmp_path, monkeypatch, capsys):
    secret = "private-" + "argument-fixture"
    seen = []
    monkeypatch.setattr(sys, "argv", ["release_build.py", "fake-builder", secret])
    monkeypatch.setattr(release.subprocess, "call", lambda command, env: seen.append((command, env)) or 7)
    monkeypatch.chdir(tmp_path)
    with pytest.raises(SystemExit) as error:
        release.main()
    assert error.value.code == 7
    assert seen[0][0] == ["fake-builder", secret]
    assert f"--remap-path-prefix={tmp_path.resolve()}=/straw" in seen[0][1]["CARGO_ENCODED_RUSTFLAGS"].split("\x1f")
    output = capsys.readouterr()
    assert output.out == output.err == ""
