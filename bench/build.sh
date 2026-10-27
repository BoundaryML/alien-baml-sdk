#!/usr/bin/env bash
# Builds every benchmark program: the four SDKs (scripts/generate.sh), the Go
# and Rust bench programs, and Alien's current SDKs (the incumbents; the
# Speakeasy Manager SDK only for conformance/incumbents).
#
#   ALIEN_REF=/path/to/alien bench/build.sh
#
# ALIEN_REF is a checkout of github.com/alienplatform/alien. Its HEAD commit is
# exported with `git archive` into .cache/incumbents/alien (nothing is written
# inside ALIEN_REF) and the incumbents are built there. They are rebuilt only
# when that commit changes. The SDKs use the published BAML release, or a local
# BAML build with BAML_LOCAL_BUILD=1 and BAML_SOURCE (README "Testing an
# unreleased BAML build"). Writes .cache/bench/build.json for bench/run.py.
source "$(dirname "$0")/../scripts/lib.sh"

[[ -n "${ALIEN_REF:-}" && -d "$ALIEN_REF/client-sdks/platform" ]] || die "set ALIEN_REF to a checkout of the Alien repository"
CACHE="$REPO/.cache/bench"
INC="$REPO/.cache/incumbents"
mkdir -p "$CACHE" "$INC"

note "SDKs (scripts/generate.sh)"
"$REPO/scripts/generate.sh"
ensure_bridge_lib

note "go bench"
if (( BAML_LOCAL_BUILD )); then
  # The bench module's own go.work: the local runtime package, as in sdk/go.
  printf 'go 1.24\n\nuse %s\n\nreplace github.com/boundaryml/baml-go => %s\n' \
    "$REPO/bench/go" "$BAML_LOCAL_SDKS/go/baml_go" >"$CACHE/go.work"
  GOWORK_FILE="$CACHE/go.work"
else
  GOWORK_FILE=off
fi
(cd "$REPO/bench/go" && GOWORK="$GOWORK_FILE" CGO_ENABLED=1 go build -o "$CACHE/bench-go" .)

note "rust bench (release)"
RUST_PATCH=()
if (( BAML_LOCAL_BUILD )); then
  RUST_PATCH=(--config "patch.crates-io.baml_bridge.path=\"$BAML_LOCAL_SDKS/rust/bridge_rust\"")
fi
(cd "$REPO/bench/rust" && CARGO_TARGET_DIR="$CACHE/rust-target" cargo build --release --quiet "${RUST_PATCH[@]}")
cp "$CACHE/rust-target/release/alien_bench" "$CACHE/bench-rust"

ALIEN_COMMIT="$(git -C "$ALIEN_REF" rev-parse HEAD)"
if [[ "$(cat "$INC/alien.commit" 2>/dev/null)" != "$ALIEN_COMMIT" ]]; then
  note "incumbents: exporting Alien $ALIEN_COMMIT"
  rm -rf "$INC/alien" "$INC/platform-api" "$INC/manager-api" "$INC/alien.commit" "$INC"/alienplatform-platform-api-*.tgz
  mkdir -p "$INC/alien"
  git -C "$ALIEN_REF" archive --format=tar "$ALIEN_COMMIT" | tar -x -C "$INC/alien"

  note "incumbent: @alienplatform/platform-api (Speakeasy TypeScript)"
  cp -R "$INC/alien/client-sdks/platform/typescript" "$INC/platform-api"
  rm -f "$INC/platform-api"/*.tgz
  # The package-lock in the Alien repository is stale; pin what Alien's
  # pnpm-lock.yaml resolves for this package instead.
  (cd "$INC/platform-api" && rm -rf node_modules package-lock.json \
    && npm install --no-audit --no-fund --save-exact zod@4.3.2 react@19.3.0 react-dom@19.3.0 >&2 \
    && npm install --no-audit --no-fund --save-dev --save-exact @tanstack/react-query@5.104.1 typescript@5.8.3 @types/react@18.3.31 >&2 \
    && NODE_OPTIONS=--max-old-space-size=16384 npx tsc >&2 \
    && npm pack --pack-destination "$INC" >&2)

  # For conformance/incumbents/speakeasy-typescript/run.mjs only (no benchmark uses it).
  note "incumbent: @alienplatform/manager-api (Speakeasy TypeScript)"
  cp -R "$INC/alien/client-sdks/manager/typescript" "$INC/manager-api"
  (cd "$INC/manager-api" && rm -rf node_modules package-lock.json \
    && npm install --no-audit --no-fund --save-exact zod@4.3.2 >&2 \
    && npm install --no-audit --no-fund --save-dev --save-exact typescript@5.9.3 >&2 \
    && NODE_OPTIONS=--max-old-space-size=16384 npx tsc >&2)
  echo "$ALIEN_COMMIT" >"$INC/alien.commit"
else
  note "incumbents: Alien $ALIEN_COMMIT already built"
fi

note "incumbent: alien-platform-api (Progenitor Rust, default features)"
(cd "$REPO/bench/incumbent-rust" && CARGO_TARGET_DIR="$CACHE/incumbent-rust-target" cargo build --release --quiet)
cp "$CACHE/incumbent-rust-target/release/incumbent_bench" "$CACHE/bench-incumbent-rust"

python3 -I - "$CACHE/build.json" <<PY
import json, sys
json.dump({
    "baml_build": "$BAML_BUILD",
    "baml_local_build": bool($BAML_LOCAL_BUILD),
    "bridge_lib": "$BRIDGE_LIB",
    "baml_local_sdks": "${BAML_LOCAL_SDKS:-}",
    "alien_commit": "$ALIEN_COMMIT",
}, open(sys.argv[1], "w"), indent=1)
PY
note "built every benchmark program"
