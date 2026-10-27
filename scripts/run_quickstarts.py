#!/usr/bin/env python3
"""Writes each README quickstart to the file the README names, runs the README's
command for it as written, and checks the output. Run by scripts/verify.sh.

README markers look like:
    <!-- quickstart <language> <file> :: <shell command> -->
followed by one fenced code block. The expected output is the first fenced
block after the line "calls no API and prints:".

Each command gets an empty BAML_HOME and no telemetry or bridge settings from
the caller's environment, so telemetry is on (the SDK default) and records
locally: files may appear under BAML_HOME/btel, nowhere else.

Go and Rust load BAML's runtime library at run time, and may not download it
here. With the published release, their BAML_HOME starts with the library
already in its cache (bridges/<version>/, linked to --bridge-lib and the
release manifest next to it), as it is for a user after the first run, which
downloads it. With a local BAML build, BAML_BRIDGE_PATH names --bridge-lib,
as the README tells users of a local build to do.

Usage: run_quickstarts.py --out DIR --bridge-lib PATH --local-build 0|1
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
MARKER = re.compile(r"<!-- quickstart (\w+) (\S+) :: (.+?) -->\n```\w*\n(.*?)\n```", re.S)


def seed_bridge_cache(home: Path, bridge_lib: Path) -> list[Path]:
    """Links the release's runtime library (<version>/<target>/<file>) and the
    manifest next to it into home/bridges/<version>/, where the Go and Rust
    loaders look before they download. Returns the links."""
    version_dir = bridge_lib.parent.parent
    links = []
    for source in [bridge_lib, version_dir / "manifest.json"]:
        link = home / "bridges" / source.relative_to(version_dir.parent)
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(source)
        links.append(link)
    return links


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--bridge-lib", required=True)
    parser.add_argument("--local-build", required=True, choices=["0", "1"])
    args = parser.parse_args()
    bridge_lib = Path(args.bridge_lib)
    readme = (REPO / "README.md").read_text(encoding="utf-8")
    expected = re.search(r"calls no API and prints:\n\n```\n(.*?)\n```", readme, re.S)
    if not expected:
        print("README: expected-output block not found")
        return 1
    want = expected.group(1).strip()
    blocks = MARKER.findall(readme)
    languages = sorted(lang for lang, *_ in blocks)
    if languages != ["go", "python", "rust", "typescript"]:
        print(f"README: expected one quickstart per language, found {languages}")
        return 1
    failures = 0
    for lang, rel, command, code in blocks:
        path = REPO / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(code + "\n", encoding="utf-8")
        home = Path(args.out) / "quickstart" / lang / ".baml"
        shutil.rmtree(home.parent, ignore_errors=True)
        home.mkdir(parents=True)
        unset = ("BAML_BRIDGE_PATH", "ALIEN_TELEMETRY", "BOUNDARY_API_KEY", "BOUNDARY_API_URL", "BOUNDARY_PROJECT")
        env = {k: v for k, v in os.environ.items() if k not in unset}
        env["BAML_HOME"] = str(home)
        seeded = []
        if lang in ("go", "rust"):
            env.update({"BAML_BRIDGE_DISABLE_DOWNLOAD": "true", "CGO_ENABLED": "1"})
            if args.local_build == "1":
                env["BAML_BRIDGE_PATH"] = str(bridge_lib)
            else:
                seeded = seed_bridge_cache(home, bridge_lib)
        proc = subprocess.run(["bash", "-c", command], cwd=REPO, env=env, capture_output=True, text=True, timeout=900)
        got = proc.stdout.strip()
        files = [p.relative_to(home) for p in home.rglob("*") if p.is_file() and p not in seeded]
        stray = sum(1 for f in files if f.parts[0] != "btel")
        ok = proc.returncode == 0 and got == want and stray == 0
        failures += not ok
        print(f"{'PASS' if ok else 'FAIL'} {lang}: `{command}` -> exit {proc.returncode}, output {'matches' if got == want else 'DIFFERS'}, "
              f"{len(files)} files under BAML_HOME, {stray} outside btel/")
        if not ok:
            print(f"  stdout: {proc.stdout[-1500:]}\n  stderr: {proc.stderr[-1500:]}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
