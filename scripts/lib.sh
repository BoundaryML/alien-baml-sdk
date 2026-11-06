# Shared environment for scripts/*.sh. Source it; do not run it.
#
# BAML state never touches the developer's ~/.baml: the CLI gets a repo-local
# BAML_HOME, and every SDK run gets its own (see run_isolated).
#
# While BAML_COMMIT_PIN is set (below), the toolchain and every runtime
# package come from a local build of that BAML commit (scripts/build_baml.sh;
# README "Building BAML"). Without it, they are the published BAML $BAML_PIN:
# the CLI is installed into .baml_home/cli, the runtime packages come from
# npm, PyPI, the Go proxy and crates.io, and BAML_LOCAL_BUILD=1 takes all of
# them from a local build instead, to test BAML fixes that are not released yet.
#
# Knobs:
#   BAML_LOCAL_BUILD 1: use the local build in BAML_SOURCE. Default 1 while
#                    BAML_COMMIT_PIN is set (0 is then an error), otherwise 0.
#   BAML_SOURCE      local build only: the BAML checkout, built at its HEAD
#                    with no uncommitted changes (required). Its HEAD must be
#                    BAML_COMMIT_PIN when that is set.
#   BAML_UNPINNED    1: accept a BAML_SOURCE at another commit than
#                    BAML_COMMIT_PIN, to try one (a BAML pull request, say).
#                    The "==> BAML" line says so. Default 0.
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
BAML_PIN="0.25.2"     # what the toolchain reports and sdk/*/ pin
# Empty: the scripts use the published BAML $BAML_PIN. To build against one
# unreleased BAML commit instead (when the SDKs need a change no release has
# yet), set it to that commit, built locally: check
# it out in BAML_SOURCE, run scripts/build_baml.sh "$BAML_SOURCE", then
# BAML_UNPINNED=1 scripts/generate.sh, which records the new bridge's
# dependencies in sdk/rust/Cargo.lock (with the pin, Cargo runs --locked; the
# bench updates its own lockfile); then set the commit here and run
# scripts/verify.sh. Back to a release: set it to "", follow the steps
# above for BAML_PIN, and record crates.io's baml_bridge again in
# sdk/rust/Cargo.lock and bench/rust/Cargo.lock (`cargo update -p baml_bridge`).
BAML_COMMIT_PIN=""
[[ -z "$BAML_COMMIT_PIN" || "$BAML_COMMIT_PIN" =~ ^[0-9a-f]{40}$ ]] \
  || { echo "error: BAML_COMMIT_PIN must be a full 40-character commit hash, got \"$BAML_COMMIT_PIN\"" >&2; exit 1; }
export DO_NOT_TRACK=1

die() { echo "error: $*" >&2; exit 1; }
note() { echo "==> $*" >&2; }

if [[ -n "$BAML_COMMIT_PIN" ]]; then
  BAML_LOCAL_BUILD="${BAML_LOCAL_BUILD:-1}"
else
  BAML_LOCAL_BUILD="${BAML_LOCAL_BUILD:-0}"
fi
BAML_UNPINNED="${BAML_UNPINNED:-0}"
[[ "$BAML_LOCAL_BUILD" =~ ^[01]$ ]] || die "BAML_LOCAL_BUILD must be 0 or 1, got \"$BAML_LOCAL_BUILD\""
[[ "$BAML_UNPINNED" =~ ^[01]$ ]] || die "BAML_UNPINNED must be 0 or 1, got \"$BAML_UNPINNED\""
if [[ -n "$BAML_COMMIT_PIN" ]] && (( ! BAML_LOCAL_BUILD )); then
  die "BAML_LOCAL_BUILD=0 asks for the published BAML $BAML_PIN, which cannot build this repository: it needs BAML commit $BAML_COMMIT_PIN (BAML_COMMIT_PIN in scripts/lib.sh), built locally. Unset BAML_LOCAL_BUILD and set BAML_SOURCE (README \"Building BAML\")"
fi
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
  [[ -n "${BAML_SOURCE:-}" ]] || die "set BAML_SOURCE to a BAML checkout${BAML_COMMIT_PIN:+ at commit $BAML_COMMIT_PIN}, built with scripts/build_baml.sh (README \"Building BAML\")"
  BAML_COMMIT="$(git -C "$BAML_SOURCE" rev-parse HEAD)" || die "BAML_SOURCE=$BAML_SOURCE is not a BAML checkout"
  if [[ -n "$BAML_COMMIT_PIN" && "$BAML_COMMIT" != "$BAML_COMMIT_PIN" ]] && (( ! BAML_UNPINNED )); then
    die "BAML_SOURCE is at BAML commit $BAML_COMMIT, not at the pinned $BAML_COMMIT_PIN (BAML_COMMIT_PIN in scripts/lib.sh). Check that out and build it:
  git -C $BAML_SOURCE fetch origin
  git -C $BAML_SOURCE checkout $BAML_COMMIT_PIN
  $REPO/scripts/build_baml.sh $BAML_SOURCE
or set BAML_UNPINNED=1 to use $BAML_COMMIT."
  fi
  # A build embeds its commit, not its working tree. The TypeScript bridge
  # build rewrites two things git tracks: dist/ and the generated protobuf code.
  git -C "$BAML_SOURCE" diff --quiet HEAD -- baml_language \
    ':!baml_language/sdks/typescript/bridge_typescript/dist' \
    ':!baml_language/sdks/typescript/bridge_typescript/typescript_src/proto' \
    || die "BAML_SOURCE=$BAML_SOURCE has uncommitted changes under baml_language/: commit or discard them, then rebuild"
  if [[ -z "$BAML_COMMIT_PIN" ]]; then
    BAML_BUILD="$BAML_PIN, local build of BAML commit $BAML_COMMIT"
  elif [[ "$BAML_COMMIT" == "$BAML_COMMIT_PIN" ]]; then
    BAML_BUILD="$BAML_PIN, local build of the pinned BAML commit $BAML_COMMIT"
  else
    BAML_BUILD="$BAML_PIN, local build of BAML commit $BAML_COMMIT, UNPINNED (BAML_UNPINNED=1; the pin is $BAML_COMMIT_PIN)"
  fi
  BAML_LOCAL_SDKS="$BAML_SOURCE/baml_language/sdks"   # the runtime packages
  : "${BAML_CLI:=$BAML_SOURCE/baml_language/target/release/baml-cli}"
  : "${BAML_BRIDGE_PATH:=$BAML_SOURCE/baml_language/target/release/$BRIDGE_FILE}"
fi

# built_from_baml_commit <file>
# Every BAML compiler and bridge build embeds the commit it was built from, and
# the runtime refuses generated code from another commit. Fails early unless
# <file> comes from the HEAD of BAML_SOURCE.
built_from_baml_commit() {
  [[ -f "$1" ]] || die "$1 does not exist: build it (scripts/build_baml.sh)"
  LC_ALL=C grep -qaF "$BAML_COMMIT" "$1" \
    || die "$1 was not built from BAML commit $BAML_COMMIT (HEAD of BAML_SOURCE): rebuild it (scripts/build_baml.sh)"
}

# --- BAML CLI ---------------------------------------------------------------
CLI_HOME="$REPO/.baml_home/cli"
mkdir -p "$CLI_HOME"

if [[ -n "${BAML_CLI:-}" ]]; then
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
# $BAML_PIN: "baml toolchain 0.25.2 …" or "baml toolchain baml-cli 0.25.2 …"
# from wrapper 0.2, "baml-cli 0.25.2" from wrapper 0.3.
toolchain_is_pinned() {
  BAML_HOME="$CLI_HOME" BAML_VERSION="$1" BAML_TOOLCHAIN="$1" baml --version 2>&1 \
    | grep -Eq "^(baml toolchain (baml-cli )?|baml-cli )${BAML_PIN//./\\.}( |$)"
}

ensure_baml_cli() {
  command -v baml >/dev/null || die "baml is not on PATH (brew install boundaryml/baml/baml)"
  if (( BAML_LOCAL_BUILD )); then
    built_from_baml_commit "${BAML_CLI:-}"
  fi
  if [[ -n "${BAML_CLI:-}" ]]; then
    [[ -x "$BAML_CLI" ]] || die "BAML_CLI=$BAML_CLI is not an executable"
  elif ! toolchain_is_pinned "$BAML_PIN"; then
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
