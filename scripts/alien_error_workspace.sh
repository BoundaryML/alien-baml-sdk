#!/usr/bin/env bash
# Checks the Rust SDK's `alien-error` conversion inside a workspace shaped
# like Alien's own, where `alien-error` is a path dependency.
#
#   scripts/alien_error_workspace.sh      (after scripts/generate.sh)
#
# Cargo tells packages apart by name, version and source, so a path
# `alien-error` and the crates.io one are two packages and their
# `alien_error::AlienError` are two types: `alien_sdk` converts to the
# crates.io one, the workspace's code names the path one, and `?` and
# `.into_sdk_error().context(..)` do not type-check. One entry in the
# workspace's manifest makes them one package again (docs/errors.md):
#
#   [patch.crates-io]
#   alien-error = { path = "crates/alien-error" }
#
# The script builds a throwaway workspace with a path copy of `alien-error`
# (the source Cargo downloaded for the SDK's own build; nothing of Alien's is
# checked in) and one member that uses `alien_sdk` as Alien's code would. It
# runs `cargo check` twice and fails unless the first run, without the patch,
# ends in the type errors above, and the second, with it, succeeds.
#
# The workspace is rebuilt on every run under .cache/ (or
# $ALIEN_ERROR_WORKSPACE); its target directory is kept, so only the first run
# compiles the SDK's dependencies.
source "$(dirname "$0")/lib.sh"

SDK="$REPO/sdk/rust"
WORK="${ALIEN_ERROR_WORKSPACE:-$REPO/.cache/alien-error-workspace}"
[[ -f "$SDK/alien_api/Cargo.toml" ]] || die "run scripts/generate.sh first"

# Where Cargo unpacked the `alien-error` that sdk/rust/Cargo.lock names.
read -r version source < <(cargo metadata --locked --format-version 1 --manifest-path "$SDK/Cargo.toml" | python3 -I -c '
import json, os, sys
packages = [p for p in json.load(sys.stdin)["packages"] if p["name"] == "alien-error"]
assert len(packages) == 1, f"expected one alien-error package, found {len(packages)}"
print(packages[0]["version"], os.path.dirname(packages[0]["manifest_path"]))') \
  || die "cannot find the alien-error package of sdk/rust (is the alien-error feature still on by default?)"
[[ -f "$source/src/lib.rs" ]] || die "no alien-error source at $source"
note "alien-error $version, copied from $source"

mkdir -p "$WORK"
find "$WORK" -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} +
mkdir -p "$WORK/crates" "$WORK/app/src"
cp -R "$source" "$WORK/crates/alien-error"
# The published manifest names a test target the workspace has no use for;
# the lockfile is the crate's own.
rm -rf "$WORK/crates/alien-error/Cargo.lock" "$WORK/crates/alien-error/tests"
python3 -I - "$WORK/crates/alien-error/Cargo.toml" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text, count = re.subn(r"\n\[\[test\]\]\n(?:[^\[\n][^\n]*\n)+", "\n", text)
assert count == 1, "the published alien-error manifest has no [[test]] target to remove"
open(path, "w", encoding="utf-8").write(text)
PY

manifest() {  # the workspace's manifest; "$1" is appended to it
  cat >"$WORK/Cargo.toml" <<EOF
# A workspace shaped like Alien's: \`alien-error\` is a member-less path
# dependency that every crate takes from the workspace.
[workspace]
resolver = "2"
members = ["app"]

[workspace.dependencies]
alien-error = { path = "crates/alien-error", version = "$version" }
$1
EOF
}

cat >"$WORK/app/Cargo.toml" <<EOF
[package]
name = "app"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
alien-error = { workspace = true }
alien_sdk = { path = "$SDK" }
serde = { version = "1", features = ["derive"] }
EOF

cat >"$WORK/app/src/lib.rs" <<'EOF'
//! Code as Alien's own crates write it: `alien_error` is the workspace's
//! crate, errors are `alien_error::Result`, and a caller's error is chained
//! on with `.context(..)`.
use alien_error::{AlienErrorData, Context};
use alien_sdk::{Alien, SdkResultExt};

#[derive(Debug, Clone, serde::Serialize)]
pub struct LookupFailed;

impl AlienErrorData for LookupFailed {
    fn code(&self) -> &'static str {
        "MANAGER_LOOKUP_FAILED"
    }
    fn retryable(&self) -> bool {
        false
    }
    fn internal(&self) -> bool {
        false
    }
    fn message(&self) -> String {
        "cannot look the manager up".to_string()
    }
}

/// `?` converts `alien_sdk::Error` to the workspace's `alien_error::AlienError`.
pub async fn with_question_mark(alien: &Alien, id: &str) -> alien_error::Result<String> {
    Ok(alien.managers().get(id).send().await?.name)
}

/// `into_sdk_error()` converts in place, and `.context(..)` chains onto it.
pub async fn with_context(alien: &Alien, id: &str) -> alien_error::Result<String, LookupFailed> {
    let manager = alien.managers().get(id).send().await.into_sdk_error().context(LookupFailed)?;
    Ok(manager.name)
}

/// The SDK's re-export and the workspace's crate are the same crate.
pub fn same_type(error: alien_sdk::alien_error::AlienError) -> alien_error::AlienError {
    error
}
EOF

check() {  # cargo check of the workspace; prints the compiler's output
  # The SDK's lockfile first, so the workspace resolves the versions the SDK
  # is tested with (and needs no network for them).
  cp "$SDK/Cargo.lock" "$WORK/Cargo.lock"
  (cd "$WORK" && env -u CARGO_TARGET_DIR -u RUSTFLAGS cargo check --color never 2>&1)
}
packages() {  # how many `alien-error` packages the workspace resolves
  (cd "$WORK" && cargo metadata --format-version 1 | python3 -I -c '
import json, sys
print(sum(p["name"] == "alien-error" for p in json.load(sys.stdin)["packages"]))')
}

note "without [patch.crates-io]: the conversion must not type-check"
manifest ""
if output="$(check)"; then
  echo "$output"
  die "the workspace compiled without the patch: Cargo no longer treats the path alien-error as another package"
fi
echo "$output" | grep -E "^(error|warning: unused)" | sort | uniq -c
[[ "$(packages)" == 2 ]] || die "expected two alien-error packages without the patch, found $(packages)"
# `?`: no `From<alien_sdk::Error>` for the workspace's AlienError.
grep -q "error\[E0277\]" <<<"$output" || { echo "$output"; die "expected error E0277 (\`?\` cannot convert the SDK's error)"; }
grep -q "alien_sdk::Error" <<<"$output" || { echo "$output"; die "the E0277 error does not name alien_sdk::Error"; }
# `.context(..)` and `same_type`: the converted error is the other package's type.
grep -Eq "error\[E0(599|308)\]" <<<"$output" || { echo "$output"; die "expected error E0599 or E0308 (two different AlienError types)"; }
grep -Eq "^error: could not compile \`app\`" <<<"$output" || { echo "$output"; die "the failure is not in the app crate"; }
echo "without the patch: cargo check fails as expected"

note "with [patch.crates-io]: the conversion must type-check"
manifest '
[patch.crates-io]
alien-error = { path = "crates/alien-error" }'
output="$(check)" || { echo "$output"; die "the workspace does not compile with the patch"; }
echo "$output" | tail -n 3
[[ "$(packages)" == 1 ]] || die "expected one alien-error package with the patch, found $(packages)"
echo "with the patch: cargo check succeeds, one alien-error $version package"
