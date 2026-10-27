#!/usr/bin/env python3
"""Scans the repository for secrets, private paths and internal names.

Covers every file a fresh clone would contain (tracked plus untracked files
that .gitignore does not exclude). conformance/secrets.json is the only file
allowed to hold the fixture secrets. Private paths include home-relative ones
(any but the documented `~/.baml`), temporary directories and the login name of
whoever runs the scan, unless it is a generic account name. Exits non-zero on
any finding.

SECRET_SCAN_DENYLIST may name a file outside the repository with extra terms
(one per line, case-insensitive, whole words), for names and wording that
must not appear but cannot be listed here.
"""

from __future__ import annotations

import getpass
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SECRETS_FILE = "conformance/secrets.json"

PATTERNS = {
    "AWS access key id": re.compile(r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b"),
    "private key block": re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    "GitHub token": re.compile(r"\bgh[pousr]_[A-Za-z0-9]{36,}\b"),
    "Slack token": re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
    "OpenAI/Anthropic-style key": re.compile(r"\bsk-(?:ant-)?[A-Za-z0-9_-]{20,}"),
    "Boundary credential": re.compile(r"\bbdry_(?:session|secret|public|access)_[A-Za-z0-9_]+"),
    "private path": re.compile(
        # Written so that this source does not match itself. `~0` and `~1` are
        # JSON pointer escapes, not home directories.
        r"(?:/Users/[A-Za-z]|/home/[a-z]|/private/(?:tmp|var)/|/var/[f]olders/|(?<![\w.])/[t]mp/"
        r"|(?<![\w.])~(?:[A-Za-z][A-Za-z0-9_-]*)?/(?!\.baml(?![\w-]))|[A-Za-z]:\\\\Users\\\\)"
    ),
}
# Login names that say nothing about a person (CI runners, containers).
GENERIC_ACCOUNTS = {"root", "runner", "ubuntu", "user", "admin", "build", "builder", "ci", "jenkins", "vagrant", "ec2-user"}


def login_name() -> dict[str, re.Pattern[str]]:
    name = getpass.getuser()
    if len(name) < 3 or name.lower() in GENERIC_ACCOUNTS:
        return {}
    return {"login name": re.compile(rf"(?<![A-Za-z0-9]){re.escape(name)}(?![A-Za-z0-9])", re.IGNORECASE)}


def repo_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=REPO, check=True, capture_output=True,
    ).stdout.decode()
    return sorted(f for f in out.split("\0") if f and (REPO / f).is_file())


def denylist() -> dict[str, re.Pattern[str]]:
    path = os.environ.get("SECRET_SCAN_DENYLIST")
    if not path:
        return {}
    terms = [t.strip() for t in Path(path).read_text().splitlines() if t.strip() and not t.startswith("#")]
    return {f"denylisted term {t!r}": re.compile(rf"(?<![A-Za-z0-9]){re.escape(t)}(?![A-Za-z0-9])", re.IGNORECASE) for t in terms}


def main() -> int:
    secrets = json.loads((REPO / SECRETS_FILE).read_text())
    patterns = {**PATTERNS, **login_name(), **denylist()}
    findings: list[str] = []
    files = repo_files()
    for rel in files:
        data = (REPO / rel).read_bytes()
        if rel != SECRETS_FILE:
            for name, value in secrets.items():
                if value.encode() in data:
                    findings.append(f"{rel}: fixture secret {name} outside {SECRETS_FILE}")
        # Binary files too: a telemetry recording, for example, stores the
        # process command line.
        text = data.decode("utf-8", errors="replace")
        for label, pattern in patterns.items():
            for match in pattern.finditer(text):
                if rel == SECRETS_FILE and any(match.group(0) in value for value in secrets.values()):
                    continue
                line = text.count("\n", 0, match.start()) + 1
                findings.append(f"{rel}:{line}: {label}: {match.group(0)!r}")
    print(f"secret scan: {len(files)} files, {len(findings)} finding(s)")
    for finding in findings:
        print("  " + finding)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
