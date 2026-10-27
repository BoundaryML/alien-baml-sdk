#!/usr/bin/env bash
# Regenerates the four SDKs from baml/ and builds every facade.
#
#   scripts/generate.sh
#
# Generated code lands in sdk/<language>/baml_sdk/ (gitignored; never edit it).
source "$(dirname "$0")/lib.sh"

ensure_baml_cli
command -v gofmt >/dev/null || die "go is not on PATH (baml generate runs gofmt)"

note "baml generate"
# Skip warnings only print with display_all_warnings. Each generator prints
# its warnings before its "Generated <language>" line. Standard-library items
# no host type can express are tolerated, and so are types of the internal
# `compat` namespace (its functions exchange JSON text). Any other skipped item
# of this project (`user.`) is a hole in a generated SDK: fail.
output="$(baml_cli --project "$REPO/baml" generate -F display_all_warnings 2>&1)" || { echo "$output" >&2; die "baml generate failed"; }
grep -v '^warning: skipped `' <<<"$output" >&2 || true
skipped="$(awk '
  /^warning: skipped `user\./ { pending[n++] = $0; next }
  /^ *Generated [a-z]+ / { for (i = 0; i < n; i++) print $2 ": " pending[i]; n = 0 }
' <<<"$output" | grep -Ev '^[a-z]+: warning: skipped `user\.compat\.[A-Za-z0-9_]+`: (references|unsupported union arm)' || true)"
if [[ -n "$skipped" ]]; then
  echo "$skipped" >&2
  die "baml generate skipped $(wc -l <<<"$skipped" | tr -d ' ') items of this project (see docs/gaps.md)"
fi

# A local BAML build replaces the runtime packages the manifests pin, without
# changing a manifest: TypeScript and Python get its bridges installed over
# the released ones; Go and Rust build its bridge sources, through the
# gitignored sdk/go/go.work and sdk/rust/.cargo/config.toml. A release build
# undoes that first: pnpm leaves an existing node_modules (and its link to the
# local bridge) as it is, and uv keeps an installed bridge of the same version.
# The local crate also replaces baml_bridge's entry in sdk/rust/Cargo.lock;
# a release build needs the committed one back.
if (( ! BAML_LOCAL_BUILD )); then
  rm -rf "$REPO/sdk/typescript/node_modules"
  rm -f "$REPO/sdk/go/go.work" "$REPO/sdk/go/go.work.sum" "$REPO/sdk/rust/.cargo/config.toml"
  grep -A2 '^name = "baml_bridge"$' "$REPO/sdk/rust/Cargo.lock" | grep -q '^source = "registry+' \
    || die "sdk/rust/Cargo.lock records a local baml_bridge, left by a local BAML build: restore it (git checkout sdk/rust/Cargo.lock)"
fi

note "typescript: pnpm install"
(cd "$REPO/sdk/typescript" && pnpm install --frozen-lockfile >&2)

note "python: uv sync"
if (( BAML_LOCAL_BUILD )); then
  (cd "$REPO/sdk/python" && uv sync --frozen >&2)
else
  (cd "$REPO/sdk/python" && uv sync --frozen --reinstall-package baml-bridge >&2)
fi

if (( BAML_LOCAL_BUILD )); then
  note "runtime packages: local build of BAML commit $BAML_COMMIT"
  for addon in "$BAML_LOCAL_SDKS"/typescript/bridge_typescript/dist/baml_node.*.node; do built_from_baml_commit "$addon"; done
  ln -sfn "$BAML_LOCAL_SDKS/typescript/bridge_typescript" "$REPO/sdk/typescript/node_modules/@boundaryml/baml-bridge"
  # From --find-links, not a file path: `uv run` then keeps it instead of syncing back to PyPI's.
  uv pip install --quiet --python "$REPO/sdk/python/.venv" --reinstall --no-deps --no-index \
    --find-links "$BAML_LOCAL_SDKS/python/dist" "baml-bridge==$BAML_PIN" >&2
  for ext in "$REPO"/sdk/python/.venv/lib/python*/site-packages/baml_bridge/baml_py*.so; do built_from_baml_commit "$ext"; done
  printf 'go 1.24\n\nuse .\n\nreplace github.com/boundaryml/baml-go => %s\n' "$BAML_LOCAL_SDKS/go/baml_go" >"$REPO/sdk/go/go.work"
  mkdir -p "$REPO/sdk/rust/.cargo"
  printf '[patch.crates-io]\nbaml_bridge = { path = "%s" }\n' "$BAML_LOCAL_SDKS/rust/bridge_rust" >"$REPO/sdk/rust/.cargo/config.toml"
fi

note "typescript: tsc"
(cd "$REPO/sdk/typescript" && rm -rf dist && pnpm run build >&2)

note "python: import"
# Importing starts the runtime: give it an empty BAML_HOME, telemetry off and
# no Boundary credentials, and require that it writes nothing.
home="$REPO/.cache/generate-home"
rm -rf "$home"
(cd "$REPO/sdk/python" && run_isolated_ts "$home" env -u BOUNDARY_API_KEY -u BOUNDARY_API_URL -u BOUNDARY_PROJECT ALIEN_TELEMETRY=off \
  uv run --frozen python -c "import baml_bridge, alien_sdk; assert baml_bridge.__version__ == '$BAML_PIN', baml_bridge.__version__" >&2)
[[ -z "$(find "$home" -type f)" ]] || die "importing alien_sdk wrote files under $home"

note "go: alien/operations.go"
# The facade's methods (c.Deployments.List), generated from baml_sdk/functions.go.
(cd "$REPO/sdk/go" && go run ./internal/genops >&2)

note "go: build + vet"
(cd "$REPO/sdk/go" && CGO_ENABLED=1 go build ./... >&2 && CGO_ENABLED=1 go vet ./alien/... ./internal/... >&2)

note "rust: cargo build"
if (( BAML_LOCAL_BUILD )); then
  # Not --locked: Cargo.lock records the local crate instead of crates.io's.
  (cd "$REPO/sdk/rust" && cargo build --all-targets >&2)
else
  (cd "$REPO/sdk/rust" && cargo build --locked --all-targets >&2)
fi

note "generated and built all four SDKs"
