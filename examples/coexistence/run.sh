#!/usr/bin/env bash
# B4 repro: two BAML-generated SDKs in one process (docs/gaps.md#b4).
#
#   examples/coexistence/run.sh        (after scripts/generate.sh)
#
# Builds the Alien SDK and a second, unrelated BAML project (other/), installs
# both into one TypeScript app and one Python environment the way a customer
# would, calls both, and asserts what happens today. It fails if the observed
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

[[ -d "$REPO/sdk/typescript/dist" && -d "$REPO/sdk/python/baml_sdk" ]] || die "run scripts/generate.sh first"
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
note "python: package both SDKs"
package() {  # package <dir> <name> <source package dirs...>
  local dir="$1" name="$2"; shift 2
  mkdir -p "$dir"
  for src in "$@"; do cp -R "$src" "$dir/"; done
  find "$dir" -name __pycache__ -prune -exec rm -rf {} +
  cat >"$dir/pyproject.toml" <<EOF
[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[project]
name = "$name"
version = "0.0.0"
dependencies = ["baml-bridge==$BAML_PIN", "pydantic>=2.6", "typing-extensions>=4.14.0"]

[tool.setuptools.packages.find]
where = ["."]
EOF
}
package "$WORK/py/alien-pkg" alien-sdk "$REPO/sdk/python/alien_sdk" "$REPO/sdk/python/baml_sdk"
package "$WORK/py/other-pkg" other-baml-sdk "$HERE/other/python/baml_sdk"

pyenv() {  # pyenv <venv> <first package> <second package>
  uv venv --quiet "$1" >&2
  if (( BAML_LOCAL_BUILD )); then  # first, so both packages keep it (they install its dependencies)
    uv pip install --quiet --python "$1" --no-deps --no-index --find-links "$BAML_LOCAL_SDKS/python/dist" "baml-bridge==$BAML_PIN" >&2
  fi
  uv pip install --quiet --python "$1" "$2" >&2
  uv pip install --quiet --python "$1" "$3" 2>&1 | tee "$1.install.log" >&2
}

note "python: install the Alien SDK, then the other SDK"
pyenv "$WORK/py/venv-alien-then-other" "$WORK/py/alien-pkg" "$WORK/py/other-pkg"
out="$("$WORK/py/venv-alien-then-other/bin/python" -I "$HERE/python/app.py")"; echo "$out"
expect "python, other installed last" "$out" alien.cloudformation_launch_url ok "$SAME_NAME"
expect "python, other installed last" "$out" other.greet ok "$GREETING"

note "python: install the other SDK, then the Alien SDK"
pyenv "$WORK/py/venv-other-then-alien" "$WORK/py/other-pkg" "$WORK/py/alien-pkg"
out="$("$WORK/py/venv-other-then-alien/bin/python" -I "$HERE/python/app.py")"; echo "$out"
expect "python, alien installed last" "$out" alien.cloudformation_launch_url ok "$ALIEN_URL"
expect "python, alien installed last" "$out" other.greet error "AttributeError"

note "python: vendoring the other SDK under another import name"
# Renaming the package directory avoids the module clash. Both SDKs still
# share the one process-global runtime.
mkdir -p "$WORK/py/vendored"
cp -R "$HERE/other/python/baml_sdk" "$WORK/py/vendored/other_baml_sdk"
out="$(cd "$WORK/py/vendored" && PYTHONPATH="$WORK/py/vendored" "$WORK/py/venv-other-then-alien/bin/python" "$HERE/python/app.py" vendored)"; echo "$out"
expect "python, vendored as other_baml_sdk" "$out" vendored.greet ok "$GREETING"
expect "python, vendored as other_baml_sdk" "$out" alien.cloudformation_launch_url ok "$SAME_NAME"

stray="$(find "$BAML_HOME" -type f | wc -l | tr -d ' ')"
echo "files under BAML_HOME after the runs: $stray"
[[ "$stray" == 0 ]] || die "the SDKs wrote under $BAML_HOME"

echo
if [[ $FAILURES -gt 0 ]]; then
  echo "coexistence: $FAILURES expectation(s) no longer hold; update docs/gaps.md#b4"
  exit 1
fi
echo "coexistence: every expectation in docs/gaps.md#b4 holds"
