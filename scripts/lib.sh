# Shared environment for scripts/*.sh. Source it; do not run it.
#
# BAML state never touches the developer's ~/.baml: the CLI gets a repo-local
# BAML_HOME, and every SDK run gets its own (see run_isolated).
#
# The toolchain and every runtime package are the published BAML $BAML_PIN:
# the CLI is installed into .baml_home/cli, the runtime packages come from
# npm, PyPI, the Go proxy and crates.io. BAML_LOCAL_BUILD=1 takes all of them
# from a local build of a BAML checkout instead, to test BAML fixes that are
# not released yet (README "Testing an unreleased BAML build").
#
# Knobs:
#   BAML_LOCAL_BUILD 1: use the local build in BAML_SOURCE. Default 0.
#   BAML_SOURCE      local build only: the BAML checkout, built at its HEAD
#                    with no uncommitted changes (required).
#   BAML_CLI         path to a baml-cli $BAML_PIN binary. Default: the local
#                    build's, or the release installed into .baml_home/cli.
#   BAML_BRIDGE_PATH local build: its libbridge_cffi for Go and Rust. Default:
#                    the local build's. A release always uses the published
#                    library, fetched into .cache/ (an inherited value is ignored).

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Moving to another BAML release: set BAML_PIN, pin the same version in every
# manifest that names a runtime package (sdk/typescript/package.json,
# sdk/python/pyproject.toml, sdk/go/go.mod, sdk/rust/Cargo.toml,
# examples/coexistence/other/typescript/package.json, bench/go/go.mod), update
# their lockfiles (and bench/rust/Cargo.lock), then run scripts/verify.sh.
BAML_PIN="0.23.0"     # what the toolchain reports and sdk/*/ pin
export DO_NOT_TRACK=1

die() { echo "error: $*" >&2; exit 1; }
note() { echo "==> $*" >&2; }

BAML_LOCAL_BUILD="${BAML_LOCAL_BUILD:-0}"
[[ "$BAML_LOCAL_BUILD" =~ ^[01]$ ]] || die "BAML_LOCAL_BUILD must be 0 or 1, got \"$BAML_LOCAL_BUILD\""
if (( ! BAML_LOCAL_BUILD )) && [[ -n "${BAML_SOURCE:-}" ]]; then
  die "BAML_SOURCE is set, but the scripts use the published BAML $BAML_PIN unless BAML_LOCAL_BUILD=1: set BAML_LOCAL_BUILD=1 to use the local build, or unset BAML_SOURCE"
fi

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) BRIDGE_TARGET="aarch64-apple-darwin"; BRIDGE_FILE="libbridge_cffi.dylib" ;;
  Darwin-x86_64) BRIDGE_TARGET="x86_64-apple-darwin"; BRIDGE_FILE="libbridge_cffi.dylib" ;;
  Linux-x86_64) BRIDGE_TARGET="x86_64-unknown-linux-gnu"; BRIDGE_FILE="libbridge_cffi.so" ;;
  Linux-aarch64) BRIDGE_TARGET="aarch64-unknown-linux-gnu"; BRIDGE_FILE="libbridge_cffi.so" ;;
  *) die "unsupported platform $(uname -s)-$(uname -m)" ;;
esac

# --- Local BAML build ---------------------------------------------------------
BAML_BUILD="$BAML_PIN (release)"
if (( BAML_LOCAL_BUILD )); then
  [[ -n "${BAML_SOURCE:-}" ]] || die "BAML_LOCAL_BUILD=1 needs BAML_SOURCE, the BAML checkout to use (README \"Testing an unreleased BAML build\")"
  BAML_COMMIT="$(git -C "$BAML_SOURCE" rev-parse HEAD)" || die "BAML_SOURCE=$BAML_SOURCE is not a BAML checkout"
  # A build embeds its commit, not its working tree. The TypeScript bridge
  # build rewrites two things git tracks: dist/ and the generated protobuf code.
  git -C "$BAML_SOURCE" diff --quiet HEAD -- baml_language \
    ':!baml_language/sdks/typescript/bridge_typescript/dist' \
    ':!baml_language/sdks/typescript/bridge_typescript/typescript_src/proto' \
    || die "BAML_SOURCE=$BAML_SOURCE has uncommitted changes under baml_language/: commit or discard them, then rebuild"
  BAML_BUILD="$BAML_PIN, local build of BAML commit $BAML_COMMIT"
  BAML_LOCAL_SDKS="$BAML_SOURCE/baml_language/sdks"   # the runtime packages
  : "${BAML_CLI:=$BAML_SOURCE/baml_language/target/release/baml-cli}"
  : "${BAML_BRIDGE_PATH:=$BAML_SOURCE/baml_language/target/release/$BRIDGE_FILE}"
fi

# built_from_baml_commit <file>
# Every BAML compiler and bridge build embeds the commit it was built from, and
# the runtime refuses generated code from another commit. Fails early unless
# <file> comes from the HEAD of BAML_SOURCE.
built_from_baml_commit() {
  [[ -f "$1" ]] || die "$1 does not exist: build it (README \"Testing an unreleased BAML build\")"
  LC_ALL=C grep -qaF "$BAML_COMMIT" "$1" \
    || die "$1 was not built from BAML commit $BAML_COMMIT (HEAD of BAML_SOURCE): rebuild it"
}

# --- BAML CLI ---------------------------------------------------------------
CLI_HOME="$REPO/.baml_home/cli"
mkdir -p "$CLI_HOME"

if [[ -n "${BAML_CLI:-}" ]]; then
  [[ -x "$BAML_CLI" ]] || die "BAML_CLI=$BAML_CLI is not an executable"
  # The wrapper on PATH (0.2.x) reads BAML_VERSION; newer wrappers read BAML_TOOLCHAIN.
  CLI_SELECTOR="$BAML_CLI"
else
  CLI_SELECTOR="$BAML_PIN"
fi

# The CLI's own runtime (`baml test`) records telemetry into <project>/.baml/
# unless BAML_TELEMETRY=off; the ALIEN_TELEMETRY rename applies only to the
# generated SDKs. Diagnostics use one format, uncolored, whoever runs the
# scripts (the CLI picks another one inside coding agents): verify.sh and
# generate.sh parse them.
baml_cli() {
  BAML_HOME="$CLI_HOME" BAML_VERSION="$CLI_SELECTOR" BAML_TOOLCHAIN="$CLI_SELECTOR" BAML_CLI_ALLOW_DIRECT=1 \
    BAML_TELEMETRY=off baml --diagnostic-format agent --color never "$@" --agent-skill-check off
}

# True when `baml --version` (with the given selector) reports toolchain
# $BAML_PIN, as "baml toolchain 0.23.0 …" or "baml toolchain baml-cli 0.23.0 …".
toolchain_is_pinned() {
  BAML_HOME="$CLI_HOME" BAML_VERSION="$1" BAML_TOOLCHAIN="$1" baml --version 2>&1 \
    | grep -Eq "^baml toolchain (baml-cli )?${BAML_PIN//./\\.}( |$)"
}

ensure_baml_cli() {
  command -v baml >/dev/null || die "baml is not on PATH (brew install boundaryml/baml/baml)"
  if (( BAML_LOCAL_BUILD )); then
    built_from_baml_commit "${BAML_CLI:-}"
  elif [[ -z "${BAML_CLI:-}" ]] && ! toolchain_is_pinned "$BAML_PIN"; then
    note "installing BAML toolchain $BAML_PIN into $CLI_HOME"
    BAML_HOME="$CLI_HOME" baml toolchain install "$BAML_PIN" >&2
  fi
  toolchain_is_pinned "$CLI_SELECTOR" \
    || die "expected BAML toolchain $BAML_PIN, got: $(BAML_HOME="$CLI_HOME" BAML_VERSION="$CLI_SELECTOR" baml --version 2>&1 | tr '\n' ' ')"
  note "BAML $BAML_BUILD"
}

# --- Native runtime library for Go and Rust ---------------------------------
# A release's library is cached as the runtime packages cache it on first use,
# $BAML_HOME/bridges/<version>/: the library under <target>/, and the release
# manifest, which Go's loader checks a cached library against.
BRIDGE_HOME="$REPO/.cache/bridge-home"
BRIDGE_MANIFEST_URL="https://pkg.boundaryml.com/manifest/v1/version/$BAML_PIN.json"

ensure_bridge_lib() {
  if (( BAML_LOCAL_BUILD )); then
    built_from_baml_commit "${BAML_BRIDGE_PATH:-}"
    BRIDGE_LIB="$BAML_BRIDGE_PATH"
  else
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
