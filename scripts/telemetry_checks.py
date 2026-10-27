#!/usr/bin/env python3
"""Telemetry checks T1-T6 (docs/telemetry.md), run by scripts/verify.sh.

Every run gets a fresh BAML_HOME and a local sink (telemetry/sink.mjs) as
BOUNDARY_API_URL that records every connection attempt. Programs:

- probe: the README quickstart of each language (calls only the setup helper,
  so its recordings can never hold a fixture secret);
- suite: the full conformance suite of each language (phase <= N).

Usage: telemetry_checks.py --phase N --out DIR --bridge-lib PATH --baml-cli-env JSON
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SECRETS: dict[str, str] = json.loads((REPO / "conformance" / "secrets.json").read_text())
# T3 scans recordings at these levels: (label, ALIEN_TELEMETRY value; None
# leaves it unset, the SDK default: medium).
T3_LEVELS = [("default", None), ("high", "high")]
# A fake Boundary API key: with it, a runtime that records uploads to the sink.
BOUNDARY_KEY = SECRETS["boundaryApiKey"]
TELEMETRY_VARS = ["ALIEN_TELEMETRY", "BAML_TELEMETRY", "BOUNDARY_API_KEY", "BOUNDARY_API_URL", "BOUNDARY_PROJECT", "BAML_HOME"]
LANGS = ["typescript", "python", "go", "rust"]


@dataclass
class Run:
    name: str
    exit_code: int
    files: int
    sink_connections: int
    sink_requests: list[dict]
    home: Path
    output: str
    # Files that appeared under the temporary HOME (fresh_home runs).
    home_files: int = 0
    stderr: str = ""


@dataclass
class Check:
    id: str
    lang: str
    description: str
    passed: bool
    detail: str
    runs: list[str] = field(default_factory=list)


def real_node() -> str:
    """The node binary itself. `node` on PATH may be a version manager's shim
    (mise, asdf) that writes its own state under $HOME on every run, which T5
    would count as files written under the temporary HOME."""
    proc = subprocess.run(["node", "-p", "process.execPath"], capture_output=True, text=True, check=True)
    return proc.stdout.strip()


class Env:
    def __init__(self, args: argparse.Namespace) -> None:
        self.phase = args.phase
        self.out = Path(args.out)
        self.bridge_lib = args.bridge_lib
        self.cli_env = json.loads(args.baml_cli_env)
        self.bin = self.out / "bin"
        self.node = real_node()

    def programs(self, lang: str) -> dict[str, tuple[list[str], Path]]:
        sdk = REPO / "sdk" / lang
        if lang == "typescript":
            return {
                "probe": ([self.node, "quickstart.mjs"], sdk),
                "suite": ([str(sdk / "node_modules" / ".bin" / "vitest"), "run"], sdk),
            }
        if lang == "python":
            python = str(sdk / ".venv" / "bin" / "python")
            return {
                "probe": ([python, "quickstart.py"], sdk),
                "suite": ([python, "-m", "pytest", "-q", "-p", "no:cacheprovider"], sdk),
            }
        if lang == "go":
            return {
                "probe": ([str(self.bin / "quickstart-go")], sdk),
                "suite": ([str(self.bin / "alien-go.test")], sdk / "alien"),
            }
        if lang == "rust":
            return {
                "probe": ([str(sdk / "target" / "debug" / "examples" / "quickstart")], sdk),
                "suite": ([str(self.bin / "conformance-rust")], sdk),
            }
        raise ValueError(lang)


class Sink:
    def __init__(self, node: str, log: Path) -> None:
        self.log = log
        self.proc = subprocess.Popen(
            [node, str(REPO / "telemetry" / "sink.mjs"), "--log", str(log)],
            stdout=subprocess.PIPE,
            text=True,
        )
        assert self.proc.stdout is not None
        line = self.proc.stdout.readline().strip()
        if not line.startswith("READY "):
            raise RuntimeError(f"sink did not start: {line!r}")
        self.url = line[len("READY "):]

    def stop(self) -> list[dict]:
        self.proc.terminate()
        self.proc.wait(timeout=10)
        return [json.loads(l) for l in self.log.read_text().splitlines() if l.strip()]


def count_files(path: Path) -> int:
    return sum(1 for p in path.rglob("*") if p.is_file()) if path.exists() else 0


def run(env: Env, lang: str, program: str, name: str, extra: dict[str, str], *, plant_login: str | None = None, fresh_home: bool = False) -> Run:
    """Runs one program with a fresh BAML_HOME and the local sink. With
    fresh_home, HOME is an empty directory too. plant_login ("BAML_HOME" or
    "HOME") saves a fake Boundary login for the sink's endpoint under that
    directory's BAML state first."""
    cmd, cwd = env.programs(lang)[program]
    root = env.out / "telemetry" / f"{name}-{lang}"
    shutil.rmtree(root, ignore_errors=True)
    # `baml query` reads <root>/.baml/btel, so BAML_HOME is <root>/.baml.
    home = root / ".baml"
    home.mkdir(parents=True)
    sink = Sink(env.node, root / "sink.jsonl")
    try:
        child_env = {k: v for k, v in os.environ.items() if k not in TELEMETRY_VARS and k != "BAML_BRIDGE_PATH"}
        child_env.update({"BAML_HOME": str(home), "BOUNDARY_API_URL": sink.url, "CONFORMANCE_PHASE": str(env.phase)})
        # The real node first: vitest's launcher runs `node` from PATH.
        child_env["PATH"] = os.pathsep.join([os.path.dirname(env.node), child_env.get("PATH", "")])
        child_env.pop("CONFORMANCE_OUT", None)
        if lang in ("go", "rust"):
            child_env.update({"BAML_BRIDGE_PATH": env.bridge_lib, "BAML_BRIDGE_DISABLE_DOWNLOAD": "true"})
        if fresh_home:
            # Point HOME somewhere empty too: a run that ignored BAML_HOME and
            # used ~/.baml would write to <root>/home/.baml instead.
            (root / "home").mkdir()
            child_env["HOME"] = str(root / "home")
        if plant_login:
            # A fake saved login for exactly this sink endpoint
            # (bcs_api: $BAML_HOME/login/cache/<sha256(endpoint)>.json).
            cache = (home if plant_login == "BAML_HOME" else root / "home" / ".baml") / "login" / "cache"
            cache.mkdir(parents=True)
            name_hash = hashlib.sha256(sink.url.encode()).hexdigest()
            login = {"refreshToken": SECRETS["boundaryLogin"], "caller": {"userId": "user_fixture", "email": "fixture@example.invalid"}}
            (cache / f"{name_hash}.json").write_text(json.dumps(login))
            os.chmod(cache / f"{name_hash}.json", 0o600)
        files_before = count_files(home)
        home_files_before = count_files(root / "home")
        for k, v in extra.items():
            if v is None:
                child_env.pop(k, None)
            else:
                child_env[k] = v
        proc = subprocess.run(cmd, cwd=cwd, env=child_env, capture_output=True, text=True, timeout=900)
        time.sleep(0.2)  # let the sink log the last connection
    finally:
        events = sink.stop()
    output = (proc.stdout + proc.stderr)[-4000:]
    (root / "output.txt").write_text(proc.stdout + proc.stderr)
    return Run(
        name=name,
        exit_code=proc.returncode,
        files=count_files(home) - files_before,
        sink_connections=sum(1 for e in events if e["kind"] == "connection"),
        sink_requests=[e for e in events if e["kind"] == "request"],
        home=home,
        output=output,
        home_files=count_files(root / "home") - home_files_before,
        stderr=proc.stderr,
    )


def closed_port() -> socket.socket:
    """A 127.0.0.1 socket that is bound but never listens: connecting to its
    port is refused, and nothing else can take the port while it is open."""
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    return sock


def added_stderr(r: Run, baseline: Run) -> list[str]:
    """stderr lines of r that the same program does not also print with
    telemetry off (baseline), log timestamps ignored."""
    def lines(text: str) -> list[str]:
        return [re.sub(r"^\d{4}-\d\d-\d\dT[0-9:.]+ ", "", line) for line in text.splitlines()]
    known = set(lines(baseline.stderr))
    return [line for line in lines(r.stderr) if line not in known]


def value_paths(value: object, needle: str, path: str = "") -> list[str]:
    """Paths (`[1].payload`) of the strings inside a decoded value that contain needle."""
    if isinstance(value, dict):
        return [p for k, v in value.items() for p in value_paths(v, needle, f"{path}.{k}")]
    if isinstance(value, list):
        return [p for i, v in enumerate(value) for p in value_paths(v, needle, f"{path}[{i}]")]
    return [path] if isinstance(value, str) and needle in value else []


def scan_recordings(env: Env, home: Path) -> dict[str, list[str]]:
    """Secrets found in a recording tree: plaintext grep over the files, plus a
    `baml query` dump of every table (blobs are compressed on disk). Each hit
    in a table names its span type and field, for example
    `spans: network_span network_event_values[1].payload`."""
    found: dict[str, list[str]] = {}
    for path in home.rglob("*"):
        if path.is_file():
            data = path.read_bytes()
            for key, value in SECRETS.items():
                if value.encode() in data:
                    found.setdefault(key, []).append(f"file {path.relative_to(home)}")
    root = home.parent
    evidence = query(env, root, "SELECT span_type, count(*) AS n FROM spans GROUP BY span_type")
    counts = {row[0]: row[1] for row in evidence["rows"]}
    states = sorted({r["state"] for r in evidence["outcome"]["recordings"]})
    if not counts.get("network_span"):
        # Nothing readable: an empty scan would be a false pass.
        raise NotEvaluable(f"recordings hold no readable network spans (recording state: {states}, spans: {counts})")
    if states != ["sealed"]:
        # The runtime was not shut down (docs/runtime.md#shutting-down), so
        # whatever it had not flushed yet is missing from the scan.
        raise NotEvaluable(f"recordings are not sealed (recording state: {states}, spans: {counts})")
    found["__evidence__"] = [f"spans {counts}, recordings {states}"]
    for table in ["spans", "span_announcements", "processes", "profiler"]:
        result = query(env, root, f"SELECT * FROM {table}", save=root / f"query-{table}.json")
        columns = [c["name"] for c in result["columns"]]
        for row in result["rows"]:
            cells = dict(zip(columns, row))
            for column, cell in cells.items():
                for key, value in SECRETS.items():
                    for path in value_paths(cell, value, column):
                        found.setdefault(key, []).append(f"{table}: {cells.get('span_type', '-')} {path}")
        text = json.dumps(result)
        for key, value in SECRETS.items():
            if value in text and not any(hit.startswith(f"{table}:") for hit in found.get(key, [])):
                # Only in the query's metadata (not a row): still a leak.
                found.setdefault(key, []).append(f"{table}: outside the rows")
    return found


class NotEvaluable(Exception):
    pass


def query(env: Env, root: Path, sql: str, save: Path | None = None) -> dict:
    """`baml query --local` over <root>/.baml/btel. An unsealed recording (a
    process that exited without flushing) makes the CLI exit 1 with a valid
    JSON result whose outcome says so; that outcome is reported, not hidden."""
    proc = subprocess.run(
        ["baml", "query", "--local", "--from", str(root), "--format", "json", "--max-rows", "1000000", sql, "--agent-skill-check", "off"],
        env={**os.environ, **env.cli_env},
        capture_output=True,
        text=True,
        timeout=300,
    )
    if save is not None:
        save.write_text(proc.stdout)
    try:
        result = json.loads(proc.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f"baml query failed (exit {proc.returncode}): {proc.stdout[-1000:]} {proc.stderr[-1000:]}")
    if proc.returncode != 0 and not result.get("outcome", {}).get("unsealed"):
        raise RuntimeError(f"baml query failed (exit {proc.returncode}): {json.dumps(result.get('outcome'))[:1000]}")
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", type=int, required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--bridge-lib", required=True)
    parser.add_argument("--baml-cli-env", required=True)
    env = Env(parser.parse_args())
    checks: list[Check] = []

    def ok(r: Run) -> bool:
        return r.exit_code == 0

    def fail_detail(r: Run) -> str:
        return f"{r.name} exited {r.exit_code}: {r.output[-800:]}" if r.exit_code else ""

    # T6 probes against an unreachable endpoint each take ~5 s to exit (the
    # runtime retries it at shutdown), so the four languages run at once.
    with closed_port() as sock, ThreadPoolExecutor(len(LANGS)) as pool:
        unreachable_url = f"http://127.0.0.1:{sock.getsockname()[1]}"
        unreachable = dict(zip(LANGS, pool.map(lambda lang: run(
            env, lang, "probe", "t6-unreachable",
            {"ALIEN_TELEMETRY": None, "BOUNDARY_API_KEY": BOUNDARY_KEY, "BOUNDARY_API_URL": unreachable_url}), LANGS)))

    for lang in LANGS:
        # T1: ALIEN_TELEMETRY is the switch; BAML_TELEMETRY alone does nothing.
        on = run(env, lang, "probe", "t1-alien-high", {"ALIEN_TELEMETRY": "high", "BOUNDARY_API_KEY": "local"})
        off = run(env, lang, "probe", "t1-alien-off-baml-high", {"ALIEN_TELEMETRY": "off", "BAML_TELEMETRY": "high", "BOUNDARY_API_KEY": "local"})
        med = run(env, lang, "probe", "t1-alien-medium-baml-off", {"ALIEN_TELEMETRY": "medium", "BAML_TELEMETRY": "off", "BOUNDARY_API_KEY": "local"})
        passed = all(map(ok, [on, off, med])) and on.files > 0 and off.files == 0 and med.files > 0
        checks.append(Check("T1", lang, "ALIEN_TELEMETRY controls the level; BAML_TELEMETRY has no effect", passed,
                            f"ALIEN=high: {on.files} files; ALIEN=off+BAML=high: {off.files} files; ALIEN=medium+BAML=off: {med.files} files "
                            + fail_detail(on) + fail_detail(off) + fail_detail(med)))

        # T2: off means nothing, across the full scenario run, even with a
        # Boundary API key that the control run shows would reach the sink.
        control = run(env, lang, "probe", "t2-control-default-key", {"ALIEN_TELEMETRY": None, "BOUNDARY_API_KEY": BOUNDARY_KEY})
        t2 = run(env, lang, "suite", "t2-off", {"ALIEN_TELEMETRY": "off", "BOUNDARY_API_KEY": BOUNDARY_KEY})
        checks.append(Check("T2", lang, "ALIEN_TELEMETRY=off with an API key: no files, no sink connections (full suite)",
                            ok(control) and control.sink_connections > 0 and ok(t2) and t2.files == 0 and t2.sink_connections == 0,
                            f"{t2.files} files, {t2.sink_connections} sink connections; "
                            f"control (default level, same key): {control.sink_connections} sink connections " + fail_detail(control) + fail_detail(t2)))

        # T3: no secrets recorded (local recordings only), at each level.
        for label, level in T3_LEVELS:
            t3 = run(env, lang, "suite", f"t3-{label}", {"ALIEN_TELEMETRY": level, "BOUNDARY_API_KEY": "local"})
            try:
                found = scan_recordings(env, t3.home) if ok(t3) else {}
                evidence = "; ".join(found.pop("__evidence__", []))
                evaluable = ok(t3)
            except NotEvaluable as reason:
                found, evidence, evaluable = {}, f"NOT EVALUABLE: {reason}", False
            leaked = sorted(found)
            checks.append(Check("T3", lang, f"ALIEN_TELEMETRY {label}: no fixture secret in any recording (full suite)",
                                evaluable and t3.files > 0 and t3.sink_connections == 0 and not leaked,
                                (f"{label}: {t3.files} files, {t3.sink_connections} sink connections; {evidence}; secrets found: "
                                 + (", ".join(f"{k} ({'; '.join(sorted(set(found[k])))})" for k in leaked) or "none")
                                 + " " + fail_detail(t3))))

        # T4: the default an end user gets: ALIEN_TELEMETRY unset, no Boundary
        # key and no saved login. The runtime records locally, only under
        # BAML_HOME/btel, and connects nowhere (BOUNDARY_API_URL is the sink).
        t4 = run(env, lang, "suite", "t4-default", {"ALIEN_TELEMETRY": None})
        outside = sorted(str(p.relative_to(t4.home)) for p in t4.home.rglob("*") if p.is_file() and p.relative_to(t4.home).parts[0] != "btel")
        checks.append(Check("T4", lang, "ALIEN_TELEMETRY unset, no API key: local recordings under BAML_HOME/btel, no connection (full suite)",
                            ok(t4) and t4.files > 0 and not outside and t4.sink_connections == 0,
                            f"{t4.files} files, outside btel/: {outside or 'none'}, {t4.sink_connections} sink connections " + fail_detail(t4)))

        # T5: isolation. No saved login is read except from the isolated
        # BAML_HOME: one saved under the temporary HOME's ~/.baml is ignored,
        # one planted in BAML_HOME is used.
        plain = run(env, lang, "probe", "t5-login-in-home", {"ALIEN_TELEMETRY": "medium"}, fresh_home=True, plant_login="HOME")
        planted = run(env, lang, "probe", "t5-login-in-baml-home", {"ALIEN_TELEMETRY": "medium"}, fresh_home=True, plant_login="BAML_HOME")
        stray = plain.home_files + planted.home_files
        passed = ok(plain) and ok(planted) and plain.files > 0 and plain.sink_connections == 0 and stray == 0 and planted.sink_connections > 0
        checks.append(Check("T5", lang, "only the isolated BAML_HOME is used; no saved login is read from elsewhere", passed,
                            f"login saved under HOME: {plain.files} files in BAML_HOME, {plain.sink_connections} sink connections; "
                            f"login planted in BAML_HOME: {planted.sink_connections} sink connections "
                            f"({', '.join(sorted({r['method'] + ' ' + r['path'] for r in planted.sink_requests}))}); "
                            f"files written under the temporary HOME: {stray} " + fail_detail(plain) + fail_detail(planted)))

        # T6: telemetry never prints (baml/baml.toml: on_initial_telemetry_failure
        # = "ignore"). With telemetry at its default level and no key (local
        # files), or a key and an endpoint that is unreachable (a closed
        # 127.0.0.1 port) or rejects every request (the sink, T2 control), the
        # program succeeds and prints nothing on stderr that it does not also
        # print with telemetry off (T1).
        local = run(env, lang, "probe", "t6-default-no-key", {"ALIEN_TELEMETRY": None})
        down = unreachable[lang]
        added = {"no key": added_stderr(local, off), "unreachable": added_stderr(down, off), "rejecting": added_stderr(control, off)}
        passed = (all(map(ok, [local, down, control, off])) and local.files > 0 and local.sink_connections == 0
                  and down.sink_connections == 0 and control.sink_connections > 0 and not any(added.values()))
        checks.append(Check("T6", lang, "telemetry at the default level: exit 0, no telemetry output on stderr", passed,
                            f"no key ({local.files} files, {local.sink_connections} sink connections): "
                            f"stderr added: {' / '.join(added['no key']) or 'nothing'}; "
                            f"unreachable ({unreachable_url}): {down.sink_connections} sink connections, "
                            f"stderr added: {' / '.join(added['unreachable']) or 'nothing'}; "
                            f"rejecting (sink, HTTP 503; {control.sink_connections} sink connections): "
                            f"stderr added: {' / '.join(added['rejecting']) or 'nothing'} " + fail_detail(local) + fail_detail(down)))

    results = [c.__dict__ for c in checks]
    (env.out / "telemetry" / "results.json").write_text(json.dumps(results, indent=2))
    print("| Check | Language | Result | Detail |")
    print("|---|---|---|---|")
    exit_code = 0
    for c in checks:
        print(f"| {c.id} | {c.lang} | {'PASS' if c.passed else 'FAIL'} | {c.detail.strip()} |")
        if not c.passed:
            exit_code = 1
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
