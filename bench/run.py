#!/usr/bin/env python3
"""Runs every measurement against a local stub and writes metrics/raw/results.json.

  ALIEN_REF=/path/to/alien bench/run.sh [--quick] [--runs N] [--procs N] [--builds N] [--quiet-wait MINUTES]

bench/run.sh builds everything (bench/build.sh), runs this driver, then
bench/render.py writes metrics/results.md. To re-measure without rebuilding:
`python3 bench/run.py && python3 bench/render.py`.

Every bench program (bench/typescript, bench/python, bench/go, bench/rust,
bench/incumbent-*, bench/baseline) takes a mode, prints one line, then waits
for stdin EOF so this driver can read its RSS:

  import                              load the SDK, build a client; prints "ready"
  get <base_url>                      ... and one managers.get; prints the id
  warm-get <base_url> <batches> <n>   ns per managers.get call, one per batch (JSON)
  page <base_url> <size> <warmup> <n> ms per managers.list(limit=size) call (JSON)
  warm-helper | warm-noop <batches> <n>   ns per call of the setup helper or a
                                      no-op BAML function (BAML SDKs only)

Every process gets an empty BAML_HOME and telemetry off, except the
`<language>+telemetry` programs: the same SDK with telemetry as an application
gets it by default (ALIEN_TELEMETRY unset, no API key, so it records into
BAML_HOME and sends nothing), for warm `managers.get` and the 2,000-item page.

`--builds N` also times N clean builds of the Rust bench program against
Alien's Progenitor client (default features and `full-api`), dev and release,
and counts the lines each generates.

Results are provisional unless the 1-minute load average stays low for the
whole run (bench/render.py, QUIET_LOAD): other work on the machine skews them.
`--quiet-wait MINUTES` waits before the measurements and before the builds until
it is below QUIET_LOAD, polling every minute, and runs them anyway after that
many minutes.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CACHE = REPO / ".cache" / "bench"
INCUMBENTS = REPO / ".cache" / "incumbents"
OURS = ["typescript", "python", "go", "rust"]
# The same programs with telemetry at its default (records into BAML_HOME).
TELEMETRY = [f"{name}+telemetry" for name in OURS]
QUIET_LOAD = 2.0  # as bench/render.py
SERVER: subprocess.Popen | None = None  # the stub, for error messages
INCUMBENT = ["incumbent-typescript", "incumbent-rust"]


def load1() -> float:
    return round(os.getloadavg()[0], 2)


def programs(build: dict) -> dict[str, dict]:
    py = str(REPO / "sdk" / "python" / ".venv" / "bin" / "python")
    node = shutil.which("node") or sys.exit("node not on PATH")
    native = {"BAML_BRIDGE_PATH": build["bridge_lib"], "BAML_BRIDGE_DISABLE_DOWNLOAD": "true"}
    progs = {
        "typescript": {"cmd": [node, str(REPO / "bench" / "typescript" / "bench.mjs")], "env": {}},
        "python": {"cmd": [py, str(REPO / "bench" / "python" / "bench.py")], "env": {"PYTHONPATH": str(REPO / "sdk" / "python")}},
        "go": {"cmd": [str(CACHE / "bench-go")], "env": native},
        "rust": {"cmd": [str(CACHE / "bench-rust")], "env": native},
        "incumbent-typescript": {"cmd": [node, str(REPO / "bench" / "incumbent-typescript" / "bench.mjs")], "env": {}},
        "incumbent-rust": {"cmd": [str(CACHE / "bench-incumbent-rust")], "env": {}},
        "baseline-node": {"cmd": [node, str(REPO / "bench" / "baseline" / "node.mjs")], "env": {}},
        "baseline-python": {"cmd": [py, str(REPO / "bench" / "baseline" / "python.py")], "env": {}},
    }
    for name in OURS:
        progs[f"{name}+telemetry"] = {**progs[name], "telemetry": True}
    return progs


def clean_env(extra: dict[str, str], home: Path, telemetry: bool = False) -> dict[str, str]:
    """An empty BAML_HOME, no inherited BAML or Boundary settings (so no API key), and
    telemetry off, or at its default (`telemetry`: neither level variable set)."""
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("BAML_", "BOUNDARY_")) and k != "ALIEN_TELEMETRY"}
    shutil.rmtree(home, ignore_errors=True)
    home.mkdir(parents=True)
    env["BAML_HOME"] = str(home)
    if not telemetry:
        env.update({"BAML_TELEMETRY": "off", "ALIEN_TELEMETRY": "off"})
    env.update(extra)
    return env


def rss_kb(pid: int) -> int:
    return int(subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], check=True, capture_output=True, text=True).stdout.strip())


def run_once(program: dict, argv: list[str], home: Path) -> tuple[float, int, str, int]:
    """Spawns a program; returns ms from spawn to its first stdout line, RSS (KB) right
    after that line, the line, and how many files it left in BAML_HOME. Every program
    then waits for stdin EOF."""
    telemetry = program.get("telemetry", False)
    env = clean_env(program["env"], home, telemetry)
    start = time.perf_counter()
    proc = subprocess.Popen(program["cmd"] + argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True)
    assert proc.stdout is not None and proc.stdin is not None and proc.stderr is not None
    line = proc.stdout.readline()
    elapsed = (time.perf_counter() - start) * 1000
    if not line.strip():
        proc.kill()
        stub = "" if SERVER is None or SERVER.poll() is None else f" (the stub server exited with {SERVER.returncode})"
        raise RuntimeError(f"{program['cmd']} {argv}: no result{stub}; stderr: {proc.stderr.read()[-2000:]}")
    rss = rss_kb(proc.pid)
    proc.stdin.close()
    if proc.wait(timeout=600) != 0:
        raise RuntimeError(f"{program['cmd']} {argv} exited {proc.returncode}: {proc.stderr.read()[-2000:]}")
    files = sum(1 for p in home.rglob("*") if p.is_file())
    if files and not telemetry:
        raise RuntimeError(f"{program['cmd']} {argv} wrote {files} files under BAML_HOME (telemetry must be off)")
    if telemetry and not files:
        raise RuntimeError(f"{program['cmd']} {argv} recorded nothing under BAML_HOME (telemetry should be on)")
    return elapsed, rss, line.strip(), files


# The BAML SDKs open a new TCP connection for every request (the runtime builds a
# new HTTP client per request), and every closed connection keeps a local port in
# TIME_WAIT for about 30 s on macOS, which has 16,384 of them. Several processes
# making thousands of calls each would run out, so the driver paces them: it
# starts a process only while the connections opened in the last WINDOW seconds,
# plus the ones it will open, stay within BUDGET.
WINDOW, BUDGET = 40.0, 9000
RECENT: list[tuple[float, int]] = []  # (end time, connections opened)


def pace(connections: int) -> None:
    while True:
        now = time.monotonic()
        RECENT[:] = [(t, c) for t, c in RECENT if now - t < WINDOW]
        if sum(c for _, c in RECENT) + connections <= BUDGET:
            return
        time.sleep(1)


def start_fixed_server() -> tuple[subprocess.Popen, str]:
    # A private copy, so that tooling which stops stray bench servers by command
    # line (`pkill -f bench/fixed-server.mjs`) cannot stop this one mid-run.
    script = CACHE / f"stub-server-{os.getpid()}.mjs"
    shutil.copy2(REPO / "bench" / "fixed-server.mjs", script)
    proc = subprocess.Popen(["node", str(script)], stdout=subprocess.PIPE, text=True)
    assert proc.stdout is not None
    line = proc.stdout.readline().strip()
    if not line.startswith("READY http://127.0.0.1:"):
        raise RuntimeError(f"fixed server: {line!r}")
    return proc, line[len("READY "):]


def du_kb(path: Path) -> int:
    return int(subprocess.run(["du", "-sk", str(path)], check=True, capture_output=True, text=True).stdout.split()[0])


def breakdown(root: Path, depth_scoped: bool = True) -> dict[str, int]:
    out: dict[str, int] = {}
    for child in sorted(root.iterdir()):
        if child.name.startswith(".") and child.name != ".pnpm":
            continue
        if depth_scoped and child.name.startswith("@") and child.is_dir():
            for sub in sorted(child.iterdir()):
                out[f"{child.name}/{sub.name}"] = du_kb(sub)
        else:
            out[child.name] = du_kb(child)
    return dict(sorted(out.items(), key=lambda kv: -kv[1]))


def npm_install_size(tarballs: list[Path], workdir: Path) -> dict:
    shutil.rmtree(workdir, ignore_errors=True)
    workdir.mkdir(parents=True)
    (workdir / "package.json").write_text('{"name": "size-probe", "private": true, "type": "module"}\n')
    subprocess.run(["npm", "install", "--no-audit", "--no-fund", "--omit=dev", "--ignore-scripts", *map(str, tarballs)],
                   cwd=workdir, check=True, capture_output=True)
    nm = workdir / "node_modules"
    return {"total_kb": du_kb(nm), "packages_kb": breakdown(nm)}


def typescript_size(build: dict, packs: Path) -> dict:
    subprocess.run(["pnpm", "pack", "--pack-destination", str(packs)], cwd=REPO / "sdk" / "typescript", check=True, capture_output=True)
    tarballs = [next(packs.glob("alien-baml-sdk-*.tgz"))]
    if build["baml_local_build"]:
        # The local runtime package as npm would install it: package.json and dist/
        # (its `files`), packed from a copy so nothing is written into BAML_SOURCE.
        src = Path(build["baml_local_sdks"]) / "typescript" / "bridge_typescript"
        copy = CACHE / "bridge-pack"
        shutil.rmtree(copy, ignore_errors=True)
        copy.mkdir(parents=True)
        shutil.copy2(src / "package.json", copy / "package.json")
        shutil.copytree(src / "dist", copy / "dist")
        subprocess.run(["npm", "pack", "--ignore-scripts", "--pack-destination", str(packs)], cwd=copy, check=True, capture_output=True)
        tarballs.append(next(packs.glob("boundaryml-baml-bridge-*.tgz")))
    return npm_install_size(tarballs, CACHE / "size-ts")


def python_size(build: dict, workdir: Path) -> dict:
    shutil.rmtree(workdir, ignore_errors=True)
    pkg = workdir / "alien-pkg"
    pkg.mkdir(parents=True)
    for name in ["alien_sdk", "baml_sdk"]:
        shutil.copytree(REPO / "sdk" / "python" / name, pkg / name, ignore=shutil.ignore_patterns("__pycache__"))
    dependencies = tomllib.loads((REPO / "sdk" / "python" / "pyproject.toml").read_text())["project"]["dependencies"]
    (pkg / "pyproject.toml").write_text(
        '[build-system]\nrequires = ["setuptools>=68"]\nbuild-backend = "setuptools.build_meta"\n\n'
        f'[project]\nname = "alien-sdk"\nversion = "0.0.0"\ndependencies = {json.dumps(dependencies)}\n\n'
        '[tool.setuptools.packages.find]\nwhere = ["."]\n')
    venv = workdir / "venv"
    subprocess.run(["uv", "venv", "--quiet", "--python", str(REPO / "sdk" / "python" / ".venv" / "bin" / "python"), str(venv)], check=True)
    if build["baml_local_build"]:
        wheel = next((Path(build["baml_local_sdks"]) / "python" / "dist").glob("baml_bridge-*.whl"))
        subprocess.run(["uv", "pip", "install", "--quiet", "--python", str(venv), str(wheel)], check=True)
    subprocess.run(["uv", "pip", "install", "--quiet", "--python", str(venv), str(pkg)], check=True)
    site = next(venv.glob("lib/python*/site-packages"))
    return {"total_kb": du_kb(site), "packages_kb": breakdown(site, depth_scoped=False)}


def wait_quiet(block: str, minutes: float, log) -> dict:
    """Waits until the 1-minute load average is below QUIET_LOAD, polling every minute,
    for at most `minutes`; returns the load the block started at."""
    start = time.monotonic()
    while load1() >= QUIET_LOAD and time.monotonic() - start < minutes * 60:
        log(f"{block}: waiting for a load average below {QUIET_LOAD}")
        time.sleep(60)
    return {"block": block, "waited_s": round(time.monotonic() - start), "load1": load1()}


# Build timing: the Rust bench program and Alien's Progenitor client in a program of the same
# shape (bench/incumbent-rust), default features and every operation: (directory, cargo
# arguments, the packages "crate only" rebuilds: the generated SDK and the program).
BUILDS = {
    "rust": (REPO / "bench" / "rust", [], ["baml_sdk", "alien_sdk", "alien_bench"]),
    "progenitor-default": (REPO / "bench" / "incumbent-rust", [], ["alien-platform-api", "incumbent_bench"]),
    "progenitor-full-api": (REPO / "bench" / "incumbent-rust", ["--features", "full-api"], ["alien-platform-api", "incumbent_bench"]),
}


def lines(paths) -> int:
    return sum(p.read_bytes().count(b"\n") for p in paths)


def rust_builds(runs: int, build: dict, log) -> dict:
    """Seconds for a clean build of each program in a fresh target directory (every
    dependency compiled; downloads fetched first) and for rebuilding only the SDK crate
    and the program, dev and release; and the lines of Rust each SDK generates."""
    target = CACHE / "build-timing"
    env = {**os.environ, "CARGO_TARGET_DIR": str(target)}
    out: dict = {"runs": runs, "programs": {}, "lines": {}}
    for name, (directory, extra, crates) in BUILDS.items():
        patch = []
        if name == "rust" and build["baml_local_build"]:  # as bench/build.sh
            patch = ["--config", f'patch.crates-io.baml_bridge.path="{build["baml_local_sdks"]}/rust/bridge_rust"']
        extra = [*patch, *extra]
        subprocess.run(["cargo", "fetch", "--quiet", *patch], cwd=directory, env=env, check=True)
        out["programs"][name] = {}
        for profile in ("dev", "release"):
            release = ["--release"] if profile == "release" else []
            make = ["cargo", "build", "--quiet", "--offline", *extra, *release]
            clean = ["cargo", "clean", "--quiet", "--offline", *[a for c in crates for a in ("-p", c)], *release]
            t: dict = {"clean_s": [], "crate_only_s": [], "load1": []}
            for _ in range(runs):
                shutil.rmtree(target, ignore_errors=True)
                t["load1"].append(load1())
                for key, before in (("clean_s", None), ("crate_only_s", clean)):
                    if before:
                        subprocess.run(before, cwd=directory, env=env, check=True)
                    start = time.perf_counter()
                    subprocess.run(make, cwd=directory, env=env, check=True)
                    t[key].append(round(time.perf_counter() - start, 1))
            out["programs"][name][profile] = t
            log(f"build {name} {profile}: clean {statistics.median(t['clean_s'])} s, crate only {statistics.median(t['crate_only_s'])} s")
            if name.startswith("progenitor") and profile == "release":
                # What Progenitor generated (Alien's build.rs counts the same lines).
                generated = list((target / "release" / "build").glob("alien-platform-api-*/out/codegen.rs"))
                if len(generated) != 1:
                    raise RuntimeError(f"{name}: expected one codegen.rs, found {generated}")
                out["lines"][name] = len(generated[0].read_text().splitlines())
    src = REPO / "sdk" / "rust" / "baml_sdk" / "src"
    out["lines"]["rust"] = {"total": lines(src.rglob("*.rs")),
                            "api": lines(p for d in ("platform", "manager") for p in (src / d).rglob("*.rs"))}
    shutil.rmtree(target, ignore_errors=True)
    return out


def machine() -> dict:
    def sh(*cmd: str) -> str:
        return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip()
    return {
        "cores": sh("sysctl", "-n", "hw.ncpu"),
        "memory_gb": int(sh("sysctl", "-n", "hw.memsize")) // 2**30,
        "os": f"macOS {platform.mac_ver()[0]}",
        "node": sh("node", "--version"),
        "python": sh(str(REPO / "sdk" / "python" / ".venv" / "bin" / "python"), "--version"),
        "go": sh("go", "version").replace("go version ", ""),
        "rustc": sh("rustc", "--version"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--runs", type=int, default=15, help="cold-start samples per program and mode")
    parser.add_argument("--procs", type=int, default=3, help="processes per warm-call and page measurement")
    parser.add_argument("--quick", action="store_true", help="a fast smoke run (numbers not meaningful)")
    parser.add_argument("--skip-sizes", action="store_true", help="keep the sizes from the previous results.json")
    parser.add_argument("--builds", type=int, default=3, help="clean Rust builds per program and profile")
    parser.add_argument("--skip-builds", action="store_true", help="keep the build times from the previous results.json")
    parser.add_argument("--quiet-wait", type=float, default=0, metavar="MINUTES",
                        help=f"before the measurements and the builds, wait up to this long for a load average below {QUIET_LOAD}")
    args = parser.parse_args()
    if args.quick:
        args.runs, args.procs, args.builds = 3, 1, 1
    calls = {"warm_get": (5, 200), "warm_helper": (5, 2000), "warm_noop": (5, 2000)} if args.quick else \
            {"warm_get": (5, 400), "warm_helper": (10, 2000), "warm_noop": (10, 2000)}
    pages = {20: (20, 50 if args.quick else 200), 2000: (2, 3 if args.quick else 10)}  # size: (warm-up, timed)

    build_file = CACHE / "build.json"
    if not build_file.is_file():
        sys.exit("run bench/build.sh first")
    build = json.loads(build_file.read_text())
    progs = programs(build)
    for name, prog in progs.items():
        exe = Path(prog["cmd"][-1] if prog["cmd"][0].endswith(("node", "python")) else prog["cmd"][0])
        if not exe.exists():
            sys.exit(f"{name}: {exe} missing; run bench/build.sh")
    home = CACHE / "baml_home"
    raw: dict = {
        "date": time.strftime("%Y-%m-%d %H:%M %Z"), "machine": machine(),
        "build": {k: build[k] for k in ("baml_build", "baml_local_build", "alien_commit")},  # no local paths
        "settings": {"runs": args.runs, "procs": args.procs, "calls": calls, "pages": {str(k): v for k, v in pages.items()}, "quick": args.quick,
                     "builds": args.builds, "quiet_wait_min": args.quiet_wait},
        "load": {"start": [round(x, 2) for x in os.getloadavg()]},
        "quiet": [],
    }

    def log(msg: str) -> None:
        print(f"[load {load1():5.2f}] {msg}", flush=True)

    def block(name: str, wait: bool = False) -> None:
        """Records the load a section starts at; `wait`: first waits for a quiet machine."""
        raw["quiet"].append(wait_quiet(name, args.quiet_wait if wait else 0, log))
        log(f"block: {name}")

    global SERVER
    server, base = start_fixed_server()
    SERVER = server
    try:
        # Cold start and RSS: round-robin over every (program, mode), so a burst of
        # load on the machine spreads over all of them; one unrecorded round first.
        cold_cases: list[tuple[str, str, list[str]]] = []
        for name in progs:
            if name in TELEMETRY:  # measured warm only
                continue
            if name.startswith("baseline"):
                cold_cases.append((name, "start", ["start"]))
                continue
            cold_cases += [(name, "import", ["import"]), (name, "get", ["get", base])]
            if name == "incumbent-typescript":
                cold_cases.append((name, "get-standalone", ["get-standalone", base]))
        cold: dict = {f"{n}/{m}": {"ms": [], "rss_kb": [], "load1": []} for n, m, _ in cold_cases}
        block("cold start", wait=True)  # the measurements below run back to back
        for run in range(args.runs + 1):
            for name, mode, argv in cold_cases:
                load = load1()
                ms, rss, _, _ = run_once(progs[name], argv, home)
                if run:
                    c = cold[f"{name}/{mode}"]
                    c["ms"].append(round(ms, 2)); c["rss_kb"].append(rss); c["load1"].append(load)
            log(f"cold start round {run}/{args.runs}")
        raw["cold"] = cold
        for key, c in cold.items():
            log(f"cold {key}: median {statistics.median(c['ms']):.1f} ms, RSS {statistics.median(c['rss_kb']) / 1024:.1f} MB")

        def per_process(section: str, names: list[str], argv_for, connections: int = 0) -> dict:
            """`connections`: new TCP connections one process of a program that does not
            keep connections alive opens (the BAML SDKs and Python urllib)."""
            block(section)
            out: dict = {}
            for name in names:
                out[name] = {"values": [], "load1": [], "rss_kb": [], "files": []}
            for _ in range(args.procs):
                for name in names:  # interleaved, as above
                    opens = connections if name in OURS + TELEMETRY or name == "baseline-python" else 0
                    pace(opens)
                    load = load1()
                    _, rss, line, files = run_once(progs[name], argv_for(name), home)
                    RECENT.append((time.monotonic(), opens))
                    out[name]["values"] += json.loads(line); out[name]["load1"].append(load); out[name]["rss_kb"].append(rss)
                    out[name]["files"].append(files)
            for name, o in out.items():
                log(f"{section} {name}: median {statistics.median(o['values']):.3f}")
            return out

        b, n = calls["warm_get"]
        raw["warm_get_ns"] = per_process("warm-get ns/call", OURS + INCUMBENT + TELEMETRY, lambda _: ["warm-get", base, str(b), str(n)], 200 + b * n)
        b, n = calls["warm_helper"]
        raw["warm_helper_ns"] = per_process("warm-helper ns/call", OURS, lambda _: ["warm-helper", str(b), str(n)])
        b, n = calls["warm_noop"]
        raw["warm_noop_ns"] = per_process("warm-noop ns/call", OURS, lambda _: ["warm-noop", str(b), str(n)])
        raw["page_ms"], raw["page_bytes"] = {}, {}
        for size, (warmup, timed) in pages.items():
            names = OURS + INCUMBENT + ["baseline-node", "baseline-python"] + (TELEMETRY if size == 2000 else [])
            raw["page_ms"][str(size)] = per_process(f"page {size} ms/call", names, lambda _: ["page", base, str(size), str(warmup), str(timed)], warmup + timed)
            raw["page_bytes"][str(size)] = len(subprocess.run(["curl", "-s", f"{base}/v1/managers?limit={size}"], check=True, capture_output=True).stdout)
    finally:
        server.terminate()
        (CACHE / f"stub-server-{os.getpid()}.mjs").unlink(missing_ok=True)
    raw["load"]["end"] = [round(x, 2) for x in os.getloadavg()]

    out_file = REPO / "metrics" / "raw" / "results.json"
    if args.skip_builds:
        raw["builds"] = json.loads(out_file.read_text())["builds"]
    else:
        block("rust builds", wait=True)
        raw["builds"] = rust_builds(args.builds, build, log)
    if args.skip_sizes:
        raw["sizes"] = json.loads(out_file.read_text())["sizes"]
    else:
        log("sizes")
        packs = CACHE / "packs"
        shutil.rmtree(packs, ignore_errors=True)
        packs.mkdir(parents=True)
        lib = Path(build["bridge_lib"])
        sizes: dict = {
            "typescript": typescript_size(build, packs),
            "incumbent-typescript": npm_install_size([next(INCUMBENTS.glob("alienplatform-platform-api-*.tgz"))], CACHE / "size-ts-incumbent"),
            "python": python_size(build, CACHE / "size-py"),
        }
        for lang, exe in [("go", "bench-go"), ("rust", "bench-rust")]:
            sizes[lang] = {"total_kb": du_kb(CACHE / exe) + du_kb(lib), "packages_kb": {"bench program": du_kb(CACHE / exe), lib.name: du_kb(lib)}}
        sizes["incumbent-rust"] = {"total_kb": du_kb(CACHE / "bench-incumbent-rust"), "packages_kb": {"bench program": du_kb(CACHE / "bench-incumbent-rust")}}
        sizes["generated_source_kb"] = {lang: du_kb(REPO / "sdk" / lang / "baml_sdk") for lang in OURS}
        raw["sizes"] = sizes
    out_file.parent.mkdir(parents=True, exist_ok=True)
    out_file.write_text(json.dumps(raw, indent=1) + "\n")
    log(f"wrote {out_file.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
