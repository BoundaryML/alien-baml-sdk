# Shared environment for scripts/*.sh. Source it; do not run it.
#
# BAML state never touches the developer's ~/.baml: the CLI gets a repo-local
# BAML_HOME, and every SDK run gets its own (see run_isolated).
#
# The toolchain and every runtime package are the published BAML $BAML_PIN:
# the CLI is installed into .baml_home/cli, the runtime packages come from
# npm, PyPI, the Go proxy and crates.io, and the runtime library Go and Rust
# load is fetched into .cache/.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Moving to another BAML release: set BAML_PIN, pin the same version in every
# manifest that names a runtime package (sdk/typescript/package.json,
# sdk/python/pyproject.toml (as BAML_PIN_PYPI), sdk/go/go.mod, sdk/rust/Cargo.toml,
# examples/coexistence/other/typescript/package.json, bench/go/go.mod) and in
# the minimumReleaseAgeExclude entries of sdk/typescript/pnpm-workspace.yaml
# and examples/coexistence/other/typescript/pnpm-workspace.yaml, update their
# lockfiles, then run scripts/verify.sh. The two Rust lockfiles come last: the
# generated crate sdk/rust/alien_api pins baml_bridge too, so run
# scripts/generate.sh once (it regenerates the crate, then stops at Rust), then
# `cargo update -p baml_bridge --precise <version>` in sdk/rust and bench/rust.
BAML_PIN="0.25.3-nightly.20261109.a"     # what the toolchain reports and sdk/*/ pin
# PyPI spells a nightly as a dev release (PEP 440); for a release it is BAML_PIN.
BAML_PIN_PYPI="0.25.3.dev2026110900"
export DO_NOT_TRACK=1

die() { echo "error: $*" >&2; exit 1; }
note() { echo "==> $*" >&2; }

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) BRIDGE_TARGET="aarch64-apple-darwin"; BRIDGE_FILE="libbridge_cffi.dylib" ;;
  Darwin-x86_64) BRIDGE_TARGET="x86_64-apple-darwin"; BRIDGE_FILE="libbridge_cffi.dylib" ;;
  Linux-x86_64) BRIDGE_TARGET="x86_64-unknown-linux-gnu"; BRIDGE_FILE="libbridge_cffi.so" ;;
  Linux-aarch64) BRIDGE_TARGET="aarch64-unknown-linux-gnu"; BRIDGE_FILE="libbridge_cffi.so" ;;
  *) die "unsupported platform $(uname -s)-$(uname -m)" ;;
esac

# --- BAML CLI ---------------------------------------------------------------
CLI_HOME="$REPO/.baml_home/cli"
mkdir -p "$CLI_HOME"

# The CLI's own runtime (`baml test`) records telemetry into <project>/.baml/
# unless BAML_TELEMETRY=off; the ALIEN_TELEMETRY rename applies only to the
# generated SDKs. Diagnostics use one format, uncolored, whoever runs the
# scripts (the CLI picks another one inside coding agents): verify.sh and
# generate.sh parse them. The wrapper on PATH (0.2.x) reads BAML_VERSION;
# newer wrappers read BAML_TOOLCHAIN.
baml_cli() {
  BAML_HOME="$CLI_HOME" BAML_VERSION="$BAML_PIN" BAML_TOOLCHAIN="$BAML_PIN" BAML_CLI_ALLOW_DIRECT=1 \
    BAML_TELEMETRY=off baml --diagnostic-format agent --color never "$@" --agent-skill-check off
}

# True when `baml --version` reports toolchain $BAML_PIN:
# "baml toolchain 0.25.3-nightly.20261109.a …" or "baml toolchain baml-cli 0.25.3-nightly.20261109.a …"
# from wrapper 0.2, "baml-cli 0.25.3-nightly.20261109.a" from wrapper 0.3.
toolchain_is_pinned() {
  BAML_HOME="$CLI_HOME" BAML_VERSION="$BAML_PIN" BAML_TOOLCHAIN="$BAML_PIN" baml --version 2>&1 \
    | grep -Eq "^(baml toolchain (baml-cli )?|baml-cli )${BAML_PIN//./\\.}( |$)"
}

ensure_baml_cli() {
  command -v baml >/dev/null || die "baml is not on PATH (brew install boundaryml/baml/baml)"
  if ! toolchain_is_pinned; then
    note "installing BAML toolchain $BAML_PIN into $CLI_HOME"
    BAML_HOME="$CLI_HOME" baml toolchain install "$BAML_PIN" >&2
  fi
  toolchain_is_pinned \
    || die "expected BAML toolchain $BAML_PIN, got: $(BAML_HOME="$CLI_HOME" BAML_VERSION="$BAML_PIN" BAML_TOOLCHAIN="$BAML_PIN" baml --version 2>&1 | tr '\n' ' ')"
  note "BAML $BAML_PIN"
}

# --- Native runtime library for Go and Rust ---------------------------------
# The published library is cached as the runtime packages cache it on first
# use, $BAML_HOME/bridges/<version>/: the library under <target>/, and the
# release manifest, which Go's loader checks a cached library against.
BRIDGE_HOME="$REPO/.cache/bridge-home"
BRIDGE_MANIFEST_URL="https://pkg.boundaryml.com/manifest/v1/version/$BAML_PIN.json"

ensure_bridge_lib() {
  BRIDGE_LIB="$BRIDGE_HOME/bridges/$BAML_PIN/$BRIDGE_TARGET/$BRIDGE_FILE"
  if [[ ! -f "$BRIDGE_LIB" ]]; then
    note "fetching the published BAML $BAML_PIN runtime library into $BRIDGE_HOME"
    BAML_HOME="$BRIDGE_HOME" cargo run --quiet --manifest-path "$REPO/sdk/rust/Cargo.toml" --example provision_bridge >&2
    [[ -f "$BRIDGE_LIB" ]] || die "provisioning did not produce $BRIDGE_LIB"
  fi
  local manifest="$BRIDGE_HOME/bridges/$BAML_PIN/manifest.json"
  if [[ ! -f "$manifest" ]]; then
    note "fetching the BAML $BAML_PIN release manifest into $BRIDGE_HOME"
    curl -fsS --retry 2 -o "$manifest.tmp" "$BRIDGE_MANIFEST_URL" || die "cannot fetch $BRIDGE_MANIFEST_URL"
    mv "$manifest.tmp" "$manifest"
  fi
  export BRIDGE_LIB
}

# run_isolated <baml_home> <command...>
# Runs a command with a fresh BAML_HOME. Go and Rust get the pinned runtime
# library and may not download; TypeScript and Python bundle theirs.
run_isolated() {
  local home="$1"; shift
  mkdir -p "$home"
  BAML_HOME="$home" BAML_BRIDGE_PATH="$BRIDGE_LIB" BAML_BRIDGE_DISABLE_DOWNLOAD=true "$@"
}

# The TypeScript bridge also reads BAML_BRIDGE_PATH, as a path to its .node
# addon, so TypeScript runs must not inherit the Go/Rust value.
run_isolated_ts() {
  local home="$1"; shift
  mkdir -p "$home"
  env -u BAML_BRIDGE_PATH BAML_HOME="$home" "$@"
}
