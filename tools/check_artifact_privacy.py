"""Reject local build paths and recognizable secrets in wheels and source archives.

Archives are read in memory, never extracted. Repeat --forbid for deployment
identifiers that cannot be recognized generically. Reports omit matching values.
"""

import argparse
import json
import re
import tarfile
import zipfile
from collections import Counter
from pathlib import Path

# Alternatives and character classes keep the checker itself distributable.
PATH_PATTERNS = {
    "local_home_path": re.compile(rb"/(?:root|home/[A-Za-z0-9_.-]{1,128}|Users/[A-Za-z0-9_.-]{1,128})/"),
    "local_build_path": re.compile(
        rb"/(?:github/workspace|__w/[^/\s\x00]{1,128}/[^/\s\x00]{1,128}"
        rb"|builds/[^/\s\x00]{1,128}/[^/\s\x00]{1,128}|workspaces?/[^/\s\x00]{1,128}"
        rb"|usr/local/(?:cargo|rustup)/(?:registry|git|toolchains))/"
    ),
    "windows_home_path": re.compile(rb"[A-Za-z]:[\\/](?:Users|Documents and Settings)[\\/][^\\/\s\x00]{1,128}[\\/]"),
}
SECRET_PATTERNS = {
    "private_key": re.compile(rb"-----BEGIN (?:RSA |EC |DSA |OPENSSH |ENCRYPTED |PGP )?PRIVATE KEY(?: BLOCK)?-----"),
    "credential": re.compile(
        rb"\b(?:[A]KIA[A-Z0-9]{16}|[A]SIA[A-Z0-9]{16}|gh[pousr]_[A-Za-z0-9_]{20,}"
        rb"|github_pat_[A-Za-z0-9_]{20,}|sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}"
        rb"|xox[baprs]-[A-Za-z0-9-]{20,})\b"
    ),
    "ssh_key_material": re.compile(rb"\bssh-(?:rsa|ed25519)\s+[A-Za-z0-9+/]{40,}={0,3}"),
    "authorization_token": re.compile(rb"(?i)authorization\s*:\s*bearer\s+[A-Za-z0-9._~-]{16,}"),
}
PUBLIC_URL = re.compile(rb"(?:https?|ftp)://[^\s\x00<>\"'`]+")
RELATIVE_PREFIX = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_./-"


def matches(data, forbidden=()):
    """Return category counts; never return or log the matching bytes."""
    counts = Counter()
    urls = [(m.start(), m.end()) for m in PUBLIC_URL.finditer(data)]
    binary = b"\x00" in data
    for kind, pattern in PATH_PATTERNS.items():
        for match in pattern.finditer(data):
            start = match.start()
            if any(left <= start < right for left, right in urls):
                continue
            # Source text may contain relative paths with similar components.
            # Binary string tables can concatenate an absolute path with text.
            if not binary and start and data[start - 1] in RELATIVE_PREFIX:
                continue
            counts[kind] += 1
    for kind, pattern in SECRET_PATTERNS.items():
        counts[kind] += sum(1 for _ in pattern.finditer(data))
    for value in forbidden:
        counts["forbidden_text"] += data.count(value)
    return +counts


def members(path):
    if zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                if not member.is_dir():
                    yield member.filename, archive.read(member)
    else:
        with tarfile.open(path, "r:*") as archive:
            for member in archive:
                if member.isfile():
                    stream = archive.extractfile(member)
                    if stream is None:
                        raise ValueError("Archive member is unreadable")
                    with stream:
                        yield member.name, stream.read()
                elif member.issym() or member.islnk():
                    yield member.name, member.linkname.encode("utf-8", "surrogatepass")
                else:
                    yield member.name, b""


def scan(paths, forbidden=()):
    forbidden = tuple(dict.fromkeys(value.encode("utf-8") for value in forbidden))
    if any(not value for value in forbidden):
        raise ValueError("Forbidden text must not be empty")
    findings = []
    scanned = 0
    for artifact, path in enumerate(paths, 1):
        try:
            for index, (name, contents) in enumerate(members(path), 1):
                scanned += 1
                name_matches = matches(name.encode("utf-8", "surrogatepass"), forbidden)
                counts = name_matches + matches(contents, forbidden)
                # A member name can itself contain the private value. Redact it
                # rather than moving that value from the archive into CI logs.
                label = f"<member-{index}>" if name_matches else name
                for kind, count in sorted(counts.items()):
                    findings.append({"artifact": artifact, "member": label, "type": kind, "count": count})
        except (OSError, ValueError, RuntimeError, EOFError, tarfile.TarError, zipfile.BadZipFile):
            # Exception messages often contain local input paths or ZIP names.
            findings.append({"artifact": artifact, "member": "<archive>", "type": "archive_error", "count": 1})
    return {
        "passed": not findings,
        "artifacts_scanned": len(paths),
        "members_scanned": scanned,
        "findings": findings,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(prog="check_artifact_privacy.py", description=__doc__)
    parser.add_argument("artifacts", type=Path, nargs="+")
    parser.add_argument("--forbid", action="append", default=[], help="Reject this exact text without printing it")
    args = parser.parse_args(argv)
    if any(not value for value in args.forbid):
        parser.error("--forbid must not be empty")
    report = scan(args.artifacts, args.forbid)
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
