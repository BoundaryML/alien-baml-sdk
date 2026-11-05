#!/usr/bin/env bash
# Builds what the scripts take from a BAML checkout: the CLI, the bridge
# library for Go and Rust, the TypeScript addon and the Python wheel.
#
#   scripts/build_baml.sh /path/to/baml
#
# The checkout must be at BAML_COMMIT_PIN (scripts/lib.sh), or at any commit
# with BAML_UNPINNED=1, with no uncommitted changes under baml_language/. It
# never checks out, fetches or resets anything: on a mismatch it prints the
# git commands to run and fails. Then export BAML_SOURCE=/path/to/baml for the
# other scripts (README "Building BAML").
[[ $# -eq 1 ]] || { echo "usage: scripts/build_baml.sh <path to BAML checkout>" >&2; exit 1; }
[[ -d "$1/baml_language" ]] || { echo "error: $1 is not a BAML checkout (no baml_language/)" >&2; exit 1; }
# lib.sh checks the checkout's commit and tree, as for every other script.
BAML_LOCAL_BUILD=1 BAML_SOURCE="$(cd "$1" && pwd)"
source "$(dirname "$0")/lib.sh"

cd "$BAML_SOURCE/baml_language"
note "building BAML commit $BAML_COMMIT in $BAML_SOURCE"
note "cargo build: baml_cli, bridge_cffi"
cargo build --release -p baml_cli -p bridge_cffi
note "typescript bridge"
(cd sdks/typescript/bridge_typescript && pnpm install --ignore-workspace --ignore-scripts --frozen-lockfile && pnpm build:release)
note "python wheel"
(cd sdks/python && rm -f dist/baml_bridge-*.whl && uvx --from 'maturin>=1.10,<2.0' maturin build --release --strip --out dist)

built_from_baml_commit "$BAML_SOURCE/baml_language/target/release/baml-cli"
built_from_baml_commit "$BAML_SOURCE/baml_language/target/release/$BRIDGE_FILE"
note "built BAML $BAML_BUILD; now export BAML_SOURCE=$BAML_SOURCE"
