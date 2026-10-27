# Input specifications

Copied unchanged from [alienplatform/alien](https://github.com/alienplatform/alien)
at commit `c5c3a30b3a71e819f55eb3dd8729b51dfd0db57f` (2026-10-26, release v3.3.43).
Copyright 2026 Alien Software, Inc., under the
[Functional Source License 1.1, Apache 2.0 Future License](https://github.com/alienplatform/alien/blob/c5c3a30b3a71e819f55eb3dd8729b51dfd0db57f/LICENSE.md)
(FSL-1.1-Apache-2.0). `tools/openapi2baml` reads them; Alien's specs stay the
source of truth. `scripts/import.sh --specs-from <checkout>` copies them, and
the importer's naming oracle, from a clean checkout of that repository.

| File | From | SHA-256 |
|---|---|---|
| `platform/openapi.json` | `client-sdks/platform/openapi.json` (OpenAPI 3.0.0, 282 operations) | `969ecd61…d968bc` |
| `platform/sdk-compatibility-overlay.json` | `client-sdks/platform/sdk-compatibility-overlay.json` | `62915a4b…e9cb6b` |
| `platform/speakeasy-overlay.yaml` | `client-sdks/platform/speakeasy-overlay.yaml` (reference only) | `5a2f8ee7…2b94` |
| `speakeasy-overlay.yaml` | `client-sdks/speakeasy-overlay.yaml` (reference only) | `e6eaaf4b…62ad` |
| `manager/openapi.json` | `client-sdks/manager/openapi.json` (OpenAPI 3.1.0, 53 operations; the TypeScript SDK's input) | `51edffc9…b1110c` |
| `manager/openapi-3.0.json` | `client-sdks/manager/openapi-3.0.json` (OpenAPI 3.0.3; the Rust SDK's input; reference only) | `f17efe15…45cfdb` |

The importer re-applies the overlays' intent in code
([tools/openapi2baml/DESIGN.md](../tools/openapi2baml/DESIGN.md)); the YAML
overlays are kept for reference.

## The two Manager specs

Both describe the same 53 operations (same methods, paths and operation IDs)
and the same 415 schemas. They spell nullability as their OpenAPI version
does (3.1 `type: [T, "null"]`, `oneOf: [{type: null}, …]` and `null` enum
members, versus 3.0 `nullable: true` and `allOf` wrappers), 3.1 adds
`propertyNames` on maps, and the 3.0 file leaves out 66 of the error
responses the 3.1 file declares (for example `404` on `GET /v1/releases/{id}`).
The importer reads the 3.1 file, the one the TypeScript SDK uses.
