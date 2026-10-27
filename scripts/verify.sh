#!/usr/bin/env bash
# One entry point for every check.
#
#   scripts/verify.sh --phase N    (N = 1 or 2)
#
# Phase 2 first tests the OpenAPI importer and checks that the imported BAML
# is current and reproducible, and that the generated operation scenarios
# (conformance/README.md) are current. Then it runs baml check/test, generates and
# builds the four SDKs, runs every conformance scenario with phase <= N in all
# four languages, checks the shared setup fixture and the README quickstarts,
# runs the telemetry checks (docs/telemetry.md), the B4 coexistence repro and a
# secret scan. Exits non-zero on the first failure and prints a summary table.
# Logs: .verify/logs/<step>.log
source "$(dirname "$0")/lib.sh"

PHASE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --phase) PHASE="${2:-}"; shift 2 ;;
    *) die "unknown argument: $1 (usage: scripts/verify.sh --phase N)" ;;
  esac
done
# Later phases add steps; until they do, accepting them would pass vacuously.
[[ "$PHASE" =~ ^[12]$ ]] || die "usage: scripts/verify.sh --phase N (N = 1 or 2)"

OUT="$REPO/.verify"
rm -rf "$OUT"
mkdir -p "$OUT/logs" "$OUT/bin" "$OUT/conformance" "$OUT/home"
export CONFORMANCE_PHASE="$PHASE"

STEPS=()
summary() {
  echo
  echo "| Step | Result | Seconds |"
  echo "|---|---|---|"
  for row in "${STEPS[@]}"; do echo "| ${row//;/ | } |"; done
}

step() {
  local name="$1"; shift
  local start=$SECONDS
  printf '%-22s ' "$name"
  # A subshell, so `die` fails the step instead of the script. errexit does not
  # apply inside `if`, so every step function returns its failures explicitly.
  if ("$@") >"$OUT/logs/$name.log" 2>&1; then
    STEPS+=("$name;PASS;$((SECONDS - start))")
    echo "PASS ($((SECONDS - start))s)"
  else
    STEPS+=("$name;FAIL;$((SECONDS - start))")
    echo "FAIL ($((SECONDS - start))s)"
    echo "---- last lines of .verify/logs/$name.log ----"
    tail -n 80 "$OUT/logs/$name.log"
    summary
    exit 1
  fi
}

versions() {
  echo "verify.sh --phase $PHASE on $(uname -s) $(uname -m), $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "node   $(node --version)"
  echo "pnpm   $(pnpm --version)"
  echo "python $(python3 --version 2>&1 | cut -d' ' -f2) (scripts), $("$(cd "$REPO/sdk/python" && uv python find)" --version 2>&1 | cut -d' ' -f2) (sdk/python)"
  echo "uv     $(uv --version | cut -d' ' -f2)"
  echo "go     $(go version | cut -d' ' -f3)"
  echo "rustc  $(rustc --version | cut -d' ' -f2)"
  echo "cargo  $(cargo --version | cut -d' ' -f2)"
  echo "baml   $(BAML_HOME="$CLI_HOME" BAML_VERSION="$CLI_SELECTOR" BAML_TOOLCHAIN="$CLI_SELECTOR" baml --version 2>&1 | tr '\n' ' ')"
  echo "build  $BAML_BUILD"
}

# Fails on any error, and on any warning located in our own baml_src files.
# Standard-library warnings are tolerated.
baml_check() {
  local output status=0
  output="$(baml_cli --project "$REPO/baml" check 2>&1)" || status=$?
  echo "$output"
  [[ $status -eq 0 ]] || { echo "baml check exited $status"; return 1; }
  grep -q "Finished checked" <<<"$output" || { echo "baml check did not finish"; return 1; }
  local ours=0 file
  while IFS= read -r file; do
    if [[ -f "$REPO/baml/baml_src/$file" ]]; then
      echo "warning or error in our source: $file"; ours=1
    fi
  done < <(grep -Eo '^[^ :]+\.baml(:[0-9]+)' <<<"$output" | cut -d: -f1 | sort -u)
  return $ours
}

baml_test() {
  local output status=0
  output="$(baml_cli --project "$REPO/baml" test 2>&1)" || status=$?
  echo "$output"
  [[ $status -eq 0 ]] || { echo "baml test exited $status"; return 1; }
  grep -Eq "Finished [0-9]+ passed, 0 failed" <<<"$output" || return 1
  # Telemetry is off for the CLI too: no recordings next to the project.
  [[ ! -e "$REPO/baml/.baml" ]] || { echo "baml test wrote $REPO/baml/.baml"; return 1; }
}

# The shared setup fixture is spelled out in three places; they must agree.
fixture_consistency() {
  python3 -I - "$REPO" <<'PY'
import json, sys
from pathlib import Path
repo = Path(sys.argv[1])
fx = json.loads((repo / "conformance/fixtures/setup_launch_url.json").read_text())
sc = json.loads((repo / "conformance/scenarios/setup.cloudformationLaunchUrl.shared_fixture.json").read_text())
assert sc["call"]["args"]["input"] == fx["input"], "scenario input differs from the fixture"
assert sc["expect_result"]["ok"] == fx["expected"], "scenario expectation differs from the fixture"
baml = (repo / "baml/baml_src/ns_setup/setup_tests.baml").read_text()
assert json.dumps(fx["expected"], ensure_ascii=False) in baml, "BAML shared-fixture test differs from the fixture"
print("fixture, scenario and BAML test agree")
PY
}

# Runs one language suite with an isolated, initially empty BAML_HOME and
# ALIEN_TELEMETRY=off, then requires the home to be empty. The telemetry
# checks run the suites again with telemetry on (T3, T4).
suite() {
  local lang="$1"; shift
  local home="$OUT/home/$lang"
  if [[ "$lang" == typescript ]]; then
    run_isolated_ts "$home" env ALIEN_TELEMETRY=off CONFORMANCE_OUT="$OUT/conformance" "$@" || return 1
  else
    run_isolated "$home" env ALIEN_TELEMETRY=off CONFORMANCE_OUT="$OUT/conformance" CGO_ENABLED=1 "$@" || return 1
  fi
  local count
  count="$(find "$home" -type f | wc -l | tr -d ' ')"
  echo "files under BAML_HOME after the run: $count"
  [[ "$count" == 0 ]]
}

suite_typescript() { suite typescript pnpm -C "$REPO/sdk/typescript" test; }
suite_python() { suite python uv run --frozen --directory "$REPO/sdk/python" pytest; }
suite_go() { (cd "$REPO/sdk/go" && suite go go test -count=1 ./...); }
# Cargo finds sdk/rust/.cargo/config.toml (local BAML build) from the working directory.
suite_rust() { (cd "$REPO/sdk/rust" && suite rust cargo test); }

shared_fixture() {
  local expected="$OUT/shared_fixture.expected" lang
  python3 -I -c 'import json,sys; sys.stdout.write(json.load(open(sys.argv[1]))["expected"])' \
    "$REPO/conformance/fixtures/setup_launch_url.json" >"$expected" || return 1
  for lang in typescript python go rust; do
    cmp "$expected" "$OUT/conformance/$lang/setup.cloudformationLaunchUrl.shared_fixture.txt" || return 1
    echo "$lang: byte-identical ($(wc -c <"$expected" | tr -d ' ') bytes, sha256 $(shasum -a 256 "$OUT/conformance/$lang/setup.cloudformationLaunchUrl.shared_fixture.txt" | cut -d' ' -f1))"
  done
}

quickstarts() {
  python3 -I "$REPO/scripts/run_quickstarts.py" --out "$OUT" --bridge-lib "$BRIDGE_LIB" --local-build "$BAML_LOCAL_BUILD"
}

# Binaries the telemetry checks run directly (no rebuilds inside the checks).
telemetry_binaries() {
  (cd "$REPO/sdk/go" && CGO_ENABLED=1 go build -o "$OUT/bin/quickstart-go" ./cmd/quickstart \
    && CGO_ENABLED=1 go test -c -o "$OUT/bin/alien-go.test" ./alien) || return 1
  local exe
  exe="$(cd "$REPO/sdk/rust" && cargo test --no-run --test conformance --message-format=json 2>/dev/null \
    | python3 -I -c 'import json,sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("target", {}).get("name") == "conformance" and m.get("executable"):
        print(m["executable"])')" || return 1
  [[ -x "$exe" ]] || { echo "rust conformance test binary not found"; return 1; }
  cp "$exe" "$OUT/bin/conformance-rust" || return 1
  (cd "$REPO/sdk/rust" && cargo build --quiet --example quickstart)
}

telemetry() {
  local cli_env
  cli_env="$(python3 -I -c 'import json,sys; print(json.dumps(dict(zip(["BAML_HOME","BAML_VERSION","BAML_TOOLCHAIN","BAML_CLI_ALLOW_DIRECT","DO_NOT_TRACK","BAML_TELEMETRY"], sys.argv[1:]))))' \
    "$CLI_HOME" "$CLI_SELECTOR" "$CLI_SELECTOR" 1 1 off)"
  python3 -I "$REPO/scripts/telemetry_checks.py" --phase "$PHASE" --out "$OUT" --bridge-lib "$BRIDGE_LIB" --baml-cli-env "$cli_env"
}

secret_scan() { python3 -I "$REPO/scripts/secret_scan.py"; }

importer_test() { (cd "$REPO/tools/openapi2baml" && cargo test --locked); }

# Two imports into empty trees must be byte-identical.
import_deterministic() {
  local run
  for run in a b; do
    mkdir -p "$OUT/import/$run/tools/openapi2baml" || return 1
    cp -R "$REPO/specs" "$OUT/import/$run/" || return 1
    cp "$REPO/tools/openapi2baml/config.json" "$OUT/import/$run/tools/openapi2baml/" || return 1
    cargo run --quiet --locked --manifest-path "$REPO/tools/openapi2baml/Cargo.toml" -- \
      --root "$OUT/import/$run" --config tools/openapi2baml/config.json || return 1
  done
  diff -r "$OUT/import/a" "$OUT/import/b" || return 1
  echo "two imports: $(find "$OUT/import/a/baml" "$OUT/import/a/docs" -type f | wc -l | tr -d ' ') generated files, byte-identical"
}

# B4 characterization: fails if two BAML SDKs in one process start behaving
# differently from docs/gaps.md#b4 (for example because BAML fixed it).
coexistence() { COEXISTENCE_WORK="$OUT/coexistence" "$REPO/examples/coexistence/run.sh"; }

ensure_baml_cli  # may install the pinned toolchain into .baml_home/cli
versions | tee "$OUT/versions.txt"
echo
step toolchain ensure_baml_cli
if (( PHASE >= 2 )); then
  step importer-test importer_test
  step import-current "$REPO/scripts/import.sh" --check
  step import-deterministic import_deterministic
  step scenarios-current python3 -I "$REPO/scripts/operation_scenarios.py" --check
fi
step baml-check baml_check
step baml-test baml_test
step fixture-consistency fixture_consistency
step generate "$REPO/scripts/generate.sh"
step bridge-library ensure_bridge_lib
ensure_bridge_lib  # again in this shell: the step ran in a subshell
step suite-typescript suite_typescript
step suite-python suite_python
step suite-go suite_go
step suite-rust suite_rust
step shared-fixture shared_fixture
step quickstarts quickstarts
step telemetry-binaries telemetry_binaries
step telemetry telemetry
step coexistence-b4 coexistence
step secret-scan secret_scan
summary
echo
echo "telemetry checks (.verify/logs/telemetry.log):"
grep '^|' "$OUT/logs/telemetry.log"
