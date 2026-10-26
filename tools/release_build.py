"""Run a release build with local filesystem prefixes removed from Rust output.

The --print-flags form supports container build actions. It emits Cargo's encoded
flags for an environment variable; it is not a shell script.
"""

import argparse
import os
import subprocess
import tempfile
from pathlib import Path


def build_environment(environment=None, source=None):
    env = dict(os.environ if environment is None else environment)
    build_home = Path(env.get("HOME", str(Path.home()))).resolve()
    source = Path(source or Path.cwd()).resolve()
    prefixes = {
        str(build_home): "/build-home",
        str(Path(tempfile.gettempdir()).resolve()): "/build-tmp",
        str(Path(env.get("CARGO_HOME", build_home / ".cargo")).resolve()): "/cargo",
        str(Path(env.get("RUSTUP_HOME", build_home / ".rustup")).resolve()): "/rustup",
        str(source): "/straw",
    }
    encoded = env.get("CARGO_ENCODED_RUSTFLAGS")
    flags = (encoded.split("\x1f") if encoded else []) if encoded is not None else env.get("RUSTFLAGS", "").split()
    # rustc uses the last matching prefix; the most specific mappings go last.
    flags.extend(f"--remap-path-prefix={old}={prefixes[old]}" for old in sorted(prefixes, key=len))
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
    env.pop("RUSTFLAGS", None)
    return env


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--print-flags", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    env = build_environment()
    if args.print_flags:
        if args.command:
            parser.error("--print-flags does not take a build command")
        print(env["CARGO_ENCODED_RUSTFLAGS"])
    elif args.command:
        raise SystemExit(subprocess.call(args.command, env=env))
    else:
        parser.error("Provide a build command or --print-flags")


if __name__ == "__main__":
    main()
