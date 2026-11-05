#!/usr/bin/env bash
# B4 repro: two BAML-generated SDKs in one process (docs/gaps.md#b4).
#
#   examples/coexistence/run.sh        (after scripts/generate.sh)
#
# Builds the Alien SDK and a second, unrelated BAML project (other/), installs
# both into one TypeScript app and one Python environment the way a customer
# would, calls both, and asserts what happens today. The other project keeps
# BAML's default package name, baml_sdk; the Alien SDK names its own
# (package_name in baml/baml.toml). It fails if the observed
# behavior changes, so a BAML fix shows up here first. With a local BAML build
# (scripts/lib.sh), every install gets its runtime packages instead of the
# published ones.
source "$(dirname "$0")/../../scripts/lib.sh"

HERE="$REPO/examples/coexistence"
WORK="${COEXISTENCE_WORK:-$REPO/.cache/coexistence}"
rm -rf "$WORK"
mkdir -p "$WORK/packs"
# Telemetry off for both SDKs (the other one reads BAML_TELEMETRY), no Boundary
# credentials, and a BAML_HOME that must stay empty.
export ALIEN_TELEMETRY=off BAML_TELEMETRY=off BAML_HOME="$WORK/baml_home"
unset BOUNDARY_API_KEY BOUNDARY_API_URL BOUNDARY_PROJECT
mkdir -p "$BAML_HOME"

[[ -d "$REPO/sdk/typescript/dist" && -f "$REPO/sdk/python/alien_sdk/_operations.py" ]] || die "run scripts/generate.sh first"
ensure_baml_cli

note "generate the second BAML project"
baml_cli --project "$HERE/other/baml" generate 2>&1 | { grep -v '^warning: skipped `' || true; } >&2

FAILURES=0
# expect <label> <json-lines> <call> <ok|error> <substring>
expect() {
  local label="$1" lines="$2" call="$3" kind="$4" needle="$5" line
  line="$(grep "\"call\":\"$call\"" <<<"$lines" || true)"
  if grep -q "\"$kind\":" <<<"$line" && grep -qF -- "$needle" <<<"$line"; then
    echo "  OK   $label: $call -> $kind containing '$needle'"
  else
    echo "  FAIL $label: expected $call -> $kind containing '$needle', got: ${line:-<no line>}"
    FAILURES=$((FAILURES + 1))
  fi
}

ALIEN_URL="https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1"
GREETING="hello, Alien (from the other BAML project)"
SAME_NAME="not an AWS link: the other project"

# ---------------------------------------------------------------------------
note "typescript: build and pack both SDKs"
# Fresh: a previous run may have left node_modules linked to a deleted bridge
# copy, and pnpm keeps an existing node_modules as it is.
rm -rf "$HERE/other/typescript/node_modules"
(cd "$HERE/other/typescript" && pnpm install --frozen-lockfile >&2)
if (( BAML_LOCAL_BUILD )); then
  # As generate.sh does for the Alien SDK, but a copy rather than a link: in
  # the link: case below, each SDK must load its own copy of the bridge.
  bridge="$BAML_LOCAL_SDKS/typescript/bridge_typescript"
  mkdir -p "$WORK/other-bridge"
  cp -R "$bridge/package.json" "$bridge/dist" "$WORK/other-bridge/"
  ln -s "$bridge/node_modules" "$WORK/other-bridge/node_modules"
  ln -sfn "$WORK/other-bridge" "$HERE/other/typescript/node_modules/@boundaryml/baml-bridge"
fi
(cd "$HERE/other/typescript" && rm -rf dist && pnpm run build >&2)
(cd "$REPO/sdk/typescript" && pnpm pack --pack-destination "$WORK/packs" >&2)
(cd "$HERE/other/typescript" && pnpm pack --pack-destination "$WORK/packs" >&2)

note "typescript: one app installs both packages (one shared @boundaryml/baml-bridge)"
mkdir -p "$WORK/ts-installed"
cp "$HERE/typescript/app.mjs" "$REPO/sdk/typescript/pnpm-workspace.yaml" "$WORK/ts-installed/"
printf '{ "name": "ts-installed", "private": true, "type": "module" }\n' >"$WORK/ts-installed/package.json"
if (( BAML_LOCAL_BUILD )); then
  printf 'overrides:\n  "@boundaryml/baml-bridge": "file:%s"\n' "$BAML_LOCAL_SDKS/typescript/bridge_typescript" \
    >>"$WORK/ts-installed/pnpm-workspace.yaml"
fi
(cd "$WORK/ts-installed" && pnpm add "$WORK/packs/alien-baml-sdk-0.0.0.tgz" "$WORK/packs/other-baml-sdk-0.0.0.tgz" >&2)
bridges="$(find "$WORK/ts-installed/node_modules/.pnpm" -maxdepth 1 -name '@boundaryml+baml-bridge@*' | wc -l | tr -d ' ')"
echo "typescript, installed: $bridges copy of @boundaryml/baml-bridge"
[[ "$bridges" == 1 ]] || die "expected one deduplicated bridge, found $bridges"

out="$(cd "$WORK/ts-installed" && node app.mjs alien-first)"; echo "$out"
expect "ts installed, alien imported first" "$out" alien.cloudFormationLaunchUrl ok "$SAME_NAME"
expect "ts installed, alien imported first" "$out" other.greet ok "$GREETING"
out="$(cd "$WORK/ts-installed" && node app.mjs other-first)"; echo "$out"
expect "ts installed, other imported first" "$out" alien.cloudFormationLaunchUrl ok "$ALIEN_URL"
expect "ts installed, other imported first" "$out" other.greet error "Function not found: user.greet"

note "typescript workaround: each SDK resolves its own copy of the bridge (link: install)"
(cd "$HERE/typescript" && pnpm install --frozen-lockfile >&2)
out="$(cd "$HERE/typescript" && node app.mjs alien-first)"; echo "$out"
expect "ts separate bridges, alien first" "$out" alien.cloudFormationLaunchUrl ok "$ALIEN_URL"
expect "ts separate bridges, alien first" "$out" other.greet ok "$GREETING"
out="$(cd "$HERE/typescript" && node app.mjs other-first)"; echo "$out"
expect "ts separate bridges, other first" "$out" alien.cloudFormationLaunchUrl ok "$ALIEN_URL"
expect "ts separate bridges, other first" "$out" other.greet ok "$GREETING"

# ---------------------------------------------------------------------------
note "python: build the Alien SDK's wheel, as \`uv pip install sdk/python\` does"
# Its only package is alien_sdk: the generated SDK is alien_sdk._generated.
uv build --quiet --wheel --out-dir "$WORK/py/dist" "$REPO/sdk/python" >&2
wheel="$(echo "$WORK"/py/dist/alien_sdk-*.whl)"
[[ -f "$wheel" ]] || die "uv build wrote no alien_sdk wheel to $WORK/py/dist"
top="$(unzip -Z1 "$wheel" | cut -d/ -f1 | sort -u | tr '\n' ' ')"
echo "python, alien-sdk wheel: top-level $top"
[[ "$top" == "alien_sdk alien_sdk-0.0.0.dist-info " ]] || die "the alien-sdk wheel must hold only alien_sdk/ and its metadata, got: $top"

note "python: package the other SDK (top-level baml_sdk, BAML's default)"
other="$WORK/py/other-pkg"
mkdir -p "$other"
cp -R "$HERE/other/python/baml_sdk" "$other/"
find "$other" -name __pycache__ -prune -exec rm -rf {} +
cat >"$other/pyproject.toml" <<EOF
[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[project]
name = "other-baml-sdk"
version = "0.0.0"
dependencies = ["baml-bridge==$BAML_PIN", "pydantic>=2.6", "typing-extensions>=4.14.0"]

[tool.setuptools.packages.find]
where = ["."]
EOF
# One wheel for both environments: a second build from the same directory
# would also package the build/ the first one leaves there.
uv build --quiet --wheel --out-dir "$WORK/py/dist" "$other" >&2
other="$(echo "$WORK"/py/dist/other_baml_sdk-*.whl)"
[[ -f "$other" ]] || die "uv build wrote no other_baml_sdk wheel to $WORK/py/dist"

pyenv() {  # pyenv <venv> <first package> <second package>
  uv venv --quiet "$1" >&2
  if (( BAML_LOCAL_BUILD )); then  # first, so both packages keep it (they install its dependencies)
    uv pip install --quiet --python "$1" --no-deps --no-index --find-links "$BAML_LOCAL_SDKS/python/dist" "baml-bridge==$BAML_PIN" >&2
  fi
  uv pip install --quiet --python "$1" "$2" >&2
  uv pip install --quiet --python "$1" "$3" >&2
}
app() { "$1/bin/python" -I "$HERE/python/app.py" "$2"; }  # app <venv> <mode>

# Either install order leaves both packages whole. Both still share the one
# process-global runtime: the SDK imported last owns it.
python_both() {  # python_both <venv> <label>
  local venv="$1" label="$2" out
  out="$(app "$venv" installed)"; echo "$out"
  expect "python, $label" "$out" installed.alien-sdk ok "intact, top-level [alien_sdk]"
  expect "python, $label" "$out" installed.other-baml-sdk ok "intact, top-level [baml_sdk]"
  out="$(app "$venv" alien-first)"; echo "$out"
  # The other SDK's type map, also runtime state, does not know the Alien SDK's argument class.
  expect "python, $label, alien imported first" "$out" alien.cloudformation_launch_url error "expected setup.CloudFormationLaunch, found CloudFormationLaunch"
  expect "python, $label, alien imported first" "$out" other.greet ok "$GREETING"
  out="$(app "$venv" other-first)"; echo "$out"
  expect "python, $label, other imported first" "$out" alien.cloudformation_launch_url ok "$ALIEN_URL"
  expect "python, $label, other imported first" "$out" other.greet error "Function not found: user.greet"
}

note "python: install the Alien SDK, then the other SDK"
pyenv "$WORK/py/venv-alien-then-other" "$wheel" "$other"
python_both "$WORK/py/venv-alien-then-other" "other installed last"

note "python: install the other SDK, then the Alien SDK"
pyenv "$WORK/py/venv-other-then-alien" "$other" "$wheel"
python_both "$WORK/py/venv-other-then-alien" "alien installed last"

note "python: uninstalling either SDK leaves the other whole"
uv pip uninstall --quiet --python "$WORK/py/venv-alien-then-other" other-baml-sdk >&2
out="$(app "$WORK/py/venv-alien-then-other" installed; app "$WORK/py/venv-alien-then-other" alien-first)"; echo "$out"
expect "python, other uninstalled" "$out" installed.alien-sdk ok "intact"
expect "python, other uninstalled" "$out" alien.cloudformation_launch_url ok "$ALIEN_URL"
uv pip uninstall --quiet --python "$WORK/py/venv-other-then-alien" alien-sdk >&2
out="$(app "$WORK/py/venv-other-then-alien" installed; app "$WORK/py/venv-other-then-alien" other-first)"; echo "$out"
expect "python, alien uninstalled" "$out" installed.other-baml-sdk ok "intact"
expect "python, alien uninstalled" "$out" other.greet ok "$GREETING"

stray="$(find "$BAML_HOME" -type f | wc -l | tr -d ' ')"
echo "files under BAML_HOME after the runs: $stray"
[[ "$stray" == 0 ]] || die "the SDKs wrote under $BAML_HOME"

echo
if [[ $FAILURES -gt 0 ]]; then
  echo "coexistence: $FAILURES expectation(s) no longer hold; update docs/gaps.md#b4"
  exit 1
fi
echo "coexistence: every expectation in docs/gaps.md#b4 holds"
