#!/usr/bin/env bash
# Imports Alien's OpenAPI specs (specs/) into BAML with tools/openapi2baml:
# baml/baml_src/ns_platform/, baml/baml_src/ns_manager/ and
# docs/importer-coverage.md. Never edit those by hand.
#
#   scripts/import.sh                     rewrite them
#   scripts/import.sh --check             fail if they differ from the importer's output
#   scripts/import.sh --specs-from <dir>  first re-vendor specs/ and the importer's naming
#                                         oracle from a clean checkout of alienplatform/alien
#                                         at <dir>, then rewrite them
source "$(dirname "$0")/lib.sh"

usage="usage: scripts/import.sh [--check | --specs-from <alien checkout>]"
if [[ "${1:-}" == "--specs-from" ]]; then
  [[ $# -eq 2 ]] || die "$usage"
  from="$(cd "$2" && pwd)" || die "$2 is not a directory"
  commit="$(git -C "$from" rev-parse HEAD)" || die "$from is not a git checkout"
  # The specs and the TypeScript SDK the oracle is read from must be one commit.
  git -C "$from" diff --quiet HEAD -- client-sdks \
    && [[ -z "$(git -C "$from" ls-files --others --exclude-standard -- client-sdks)" ]] \
    || die "$from has uncommitted changes under client-sdks/"
  # specs/README.md lists these files.
  for file in platform/openapi.json platform/sdk-compatibility-overlay.json platform/speakeasy-overlay.yaml \
    speakeasy-overlay.yaml manager/openapi.json manager/openapi-3.0.json; do
    cp "$from/client-sdks/$file" "$REPO/specs/$file"
  done
  python3 -I "$REPO/tools/openapi2baml/scripts/extract_naming_oracle.py" --alien-ref "$from" \
    --out "$REPO/tools/openapi2baml/tests/fixtures/naming-oracle.json" >/dev/null
  note "vendored specs/ and the naming oracle from alienplatform/alien $commit"
  note "now update specs/README.md (commit, operation counts, SHA-256) and the counts in docs/"
  set --
fi
[[ $# -eq 0 || "$*" == "--check" ]] || die "$usage"
cargo run --quiet --locked --manifest-path "$REPO/tools/openapi2baml/Cargo.toml" -- \
  --root "$REPO" --config tools/openapi2baml/config.json "$@"
