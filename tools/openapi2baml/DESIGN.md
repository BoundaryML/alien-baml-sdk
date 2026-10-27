# openapi2baml: design

`openapi2baml` turns Alien's OpenAPI specifications into BAML: one class or type
alias per schema and one function per operation, in `baml/baml_src/ns_platform/`
and `baml/baml_src/ns_manager/`. `baml generate` then turns that into four SDKs.
The specifications stay the source of truth; the output is never edited by hand.

It is a standalone Rust crate with no dependency on the BAML repository.

```sh
scripts/import.sh           # rewrites ns_platform/, ns_manager/ and docs/importer-coverage.md
scripts/import.sh --check   # fails if they differ from a fresh run (verify.sh --phase 2 runs it)
```

## Inputs

A config file names, per service: the spec, overlays to merge, the operations to
import (`"all"` or an explicit list), the output directory and the client class.
`config.json` imports every operation of both specs (`scripts/import.sh` uses
it); `stage-a.json` imports only the 78 operations the SDK first covered (76
Platform, 2 Manager) and is kept for comparisons. Specs are
vendored under `specs/` (see `specs/README.md`).

| Service | Spec | Error body | Auth |
|---|---|---|---|
| `platform` | `specs/platform/openapi.json` (3.0) + `sdk-compatibility-overlay.json` | `APIError` | `Authorization: Bearer <api_key>` on every operation (global security) |
| `manager` | `specs/manager/openapi.json` (3.1) | `ErrorResponse {code, message, details?}` | `Authorization: Bearer <token>` on every operation when the client has a token, as `@alienplatform/commands` does (the spec declares `bearer` security on 36 of 53) |

## Output

Per service, every file starts with a do-not-edit banner naming the spec, the
overlays merged into it, and their SHA-256:

```
baml/baml_src/ns_platform/
  client.baml            PlatformClient
  types.baml             every class and type alias, sorted by name
  operations.baml        operations without a group
  ns_<group>/operations.baml   one namespace per group
```

The runtime helpers the generated functions call (send a request, map errors,
encode and decode bodies, build query strings) are hand-written in
`baml_src/ns_internal/http.baml`. A generated function is one call to one of
them: the response type, the method and the path, then the query parameters,
header parameters and body the operation has, as named arguments:

```baml
function list(client: root.platform.PlatformClient, search: string? = null, limit: int? = null) -> root.platform.Manager[] throws root.errors.AlienError {
    root.internal.platform_json<root.platform.Manager[]>(
        client, "GET", "/v1/managers",
        query = { "workspace": client.workspace, "search": search, "limit": limit },
    )
}

function put(client: root.platform.PlatformClient, id: string, name: string, body: root.platform.PutDynamicContainerRequest, ifMatch: string? = null) -> root.platform.DynamicContainerResponse throws root.errors.AlienError {
    root.internal.platform_json<root.platform.DynamicContainerResponse>(
        client, "PUT", `/v1/deployments/${root.internal.path_segment(id)}/dynamic-containers/${root.internal.path_segment(name)}`,
        query = { "workspace": client.workspace },
        headers = { "if-match": ifMatch },
        body = root.internal.json_body(body),
    )
}
```

- The helper is named after the service and what the response holds:
  `platform_json<T>` (JSON decoded as `T`), `platform_optional_json<T>` (`T?`:
  `null` for an empty body), `platform_empty` (no content), and
  `platform_decoded<T>` with a `decode` function for an open union ("Open
  unions" below); `manager_…` for the Manager.
- The path is a string, or a backtick string that interpolates each path
  parameter through `path_segment` (percent-encoded; "Parameters" below).
- `query` and `headers` are map literals in the order they are sent; a
  parameter whose value is `null` is left out (`query_string`,
  `request_headers`). A query value is an `internal.QueryValue`: a string, an
  integer, a number or a string list.
- `json_body` encodes the body; `null` sends none.
- A call without named arguments is one line
  (`root.internal.manager_json<root.manager.HealthResponse>(client, "GET", "/health")`).
- BAML resolves a name only in its own namespace and has no imports, so the
  service's types are written `root.platform.<Name>`, except in operations
  without a group, which are in the service's namespace (`PlatformClient`).

**Deterministic.** The importer reads specs with key order preserved, sorts
every emitted list (types by name, operations by group then method), and writes
no timestamps. Two runs over the same inputs produce byte-identical files;
`--check` compares the files on disk with a fresh run, and `verify.sh --phase 2`
also imports twice into empty trees and compares them byte for byte.

## Schemas → BAML types

| OpenAPI | BAML | Notes |
|---|---|---|
| `$ref: '#/components/schemas/X'` | `X` | Component names sanitized to valid identifiers. Scalar components (IDs, open enums) are inlined where used unless they are recursive |
| object with `properties` | `class` | Component name, or a name derived from where it appears (below) |
| object with only `additionalProperties: S` | `map<string, S>` | `true`/`{}` values become `map<string, json>` |
| object with both | `class` plus `additionalProperties: map<string, S>` | Keys the schema does not list are kept in that field (Speakeasy's name for it), by generated `FromJson`/`ToJson`. A property named `additionalProperties` fails generation |
| free-form object, `{}`, untyped schema | `json` | |
| `allOf` of one schema (with or without a description sibling) | that schema | Platform wraps refs this way |
| `allOf` of several object schemas | one merged `class` | Properties and `required` united; a conflicting property fails generation |
| `allOf` with a `oneOf`/`anyOf` part | union of merged classes | `allOf: [oneOf: [A, B], C]` is `A ∧ C \| B ∧ C`, also when the part is a `$ref` to the union (the Manager's `ExternalBinding`) |
| `oneOf` / `anyOf` | union `A \| B \| …` | `null` members make the union optional; members flattened |
| `oneOf` with `discriminator` | union of the mapped classes | Each mapped class's discriminator field becomes a required literal of its mapping value, so decoding picks the right class by shape |
| string `enum` | `string` (open) | Policy `open_enums: "all"`: a value a newer server adds still decodes. Values listed in the doc comment. See decision 1 in `docs/api-shape.md` |
| single-value `enum` / `const` | literal (`"value"`, `true`, `1`) on a union member's class; elsewhere `string` / `int` / `float` / `bool` | A union member's literal field is its tag and keeps the union unambiguous. Anywhere else (other classes, union members that are values themselves, type aliases) the literal is widened, as open enums are, and its value is listed in the doc comment. Only under `open_enums: "all"`; the other policies keep it a literal |
| integer or boolean `enum` with several values | `int` / `bool` | |
| `integer` (any format) | `int` | BAML ints are 63-bit |
| `number` | `float` | |
| `string` (any `format`, incl. `date-time`, `uri`, `email`) | `string` | `date-time` is not parsed: the SDK passes the server's text through |
| `boolean` | `bool` | |
| `array` | `T[]` | No `items` → `json[]` |
| `nullable: true`, `type: [T, "null"]`, `null` enum member | `T?` | |
| property not in `required` | `T?` | Absent and `null` both decode to null: BAML has one unset value (B13) |

**Inline schema names.** An inline object gets the name of where it appears:
`<Parent><Property>` for a property, `<Parent>Item` for array items, `<Parent>Value`
for map values. Operation-level schemas get the names Alien's TypeScript SDK
gives them: `<OperationId>Response`, and `<OperationId>Request` for a body
that is the only argument, else `<OperationId>RequestBody`. The one real member
of a nullable union (`anyOf: [X, null]`) keeps its parent's name. Other union
members get `<Parent>` plus their tag value (the discriminator property, else
any single-value property, also inside `allOf` parts), or the key of an
externally tagged variant (`{"Aws": {…}}`), else `<Parent><N>`. A derived name
that collides with another type fails generation.

**Identical classes are merged.** Alien's Platform spec inlines the same shapes
under many paths (Alien's Rust filter deduplicates them for the same reason).
Classes with identical fields (names, wire names, types, required flags) merge
until nothing changes, so shapes nested in identical shapes merge too. The
surviving name is a pinned class's (a spec component, or an operation's own
request body or response, which never merge away), else any class's; among
those, the one whose schema is under `/components/`, then the shallowest, then
the first by JSON pointer. The key is the schema's own, so neither the order
of lowering nor the other operations imported decide it: importing more
operations renames a merged class only when they bring an identical schema that
sorts first. Before merging, classes and aliases that no imported operation
reaches (such as the members of a union typed `json`) are dropped. With every
operation imported, 3,793 Platform duplicates merge away and 1,149 classes
remain (with `stage-a.json`: 1,669 and 650).

**Field names** follow Alien's TypeScript SDK. A wire name with separators
(anything but letters and digits) is camelCased, as Speakeasy does:
`custom_domains` → `customDomains`, `on-other` → `onOther`, `_internal` →
`internal`. Any other wire name is kept (`createdAt`). Then a BAML reserved
word (`test`, `in`, `for`, … — the list is in `src/naming.rs`, established by
compiling each candidate word with BAML 0.22.2; BAML 0.23.0 rejects the same
words) gets a trailing underscore (`test` →
`test_`, decision B14). A class with a field whose name differs from its wire
name implements `baml.FromJson` and `baml.ToJson` mapping each field to its
wire name, following `baml_std/openai/ns_internal/chat.baml:454`, so the JSON
on the wire is unchanged. `@alias` is never used for wire names. Every renamed
field is listed in `docs/importer-coverage.md`, and a renamed field that would
collide with an existing field fails generation. `tests/naming_oracle.rs`
checks every field name against the names in the SDK's sources
(`field_names` in the oracle): 21 Platform and 14 Manager wire names are
camelCased there.

**Request bodies omit unset optional fields.** A class reachable from a
request body that has optional fields implements `baml.ToJson`, writing each
set field under its wire name and leaving unset optional fields out (as both
of Alien's SDKs do). Other classes use BAML's default encoding, which
honors nested overrides. Values inside `json` fields are written as given,
`null`s included. A request therefore cannot send an explicit `null` for an
optional field (decision 5 in `docs/api-shape.md`, B13).

**Open unions** (`open_unions: true`). A union whose members are all classes
with a common string-literal field holding a different value in each (the
tag: `type`, `platform`, `mode`, …) gets one more member, last:
`<Owner><Field>Other` (`EventDataOther` for `Event.data`). Its tag is the
literal `"UNKNOWN"` and `raw` holds the member as the server sent it, the
shape of Speakeasy's forward-compatible unions. A member a newer server adds
then decodes as the `Other` class instead of failing the whole response
(`MALFORMED_RESPONSE` for every `events.list` when Alien adds one event type),
and since every member's tag is a literal, a `switch` on the tag stays
exhaustive in TypeScript (`case "UNKNOWN":`), with no helper.

**Unions tagged by several fields.** When no one field tells the members
apart, the tag is a sequence of fields: the first field of the first member
that every member has as a string literal, then, while some members agree on
every field so far, the first field of the first of them that all of them
have as a string literal. Alien's `ExternalBinding` (31 members) is tagged by
`type`, then `service` for the 6 `type`s several members share (`storage`:
`s3`, `blob`, `gcs`, `local-storage`); `ai` and `container_apps_environment`
have no `service`. Such a union gets the same single `Other` member, with the
first field as its tag: `ExternalBindingOther { type: "UNKNOWN", raw }`.
Decoding reads `type`, then `service` where the `type` is shared; a member is
picked only when every tag field it has matches. So a new `type`, or a known
`type` with a new `service` (`{"type": "storage", "service": "minio"}`),
decodes as `Other`, and a known pair that does not decode still fails. A
union whose fields cannot tell every member apart this way (a member without
`service` sharing its `type` with members that have one) stays closed.

BAML picks a union's member by shape, and the `Other` member's literal tag
fits no value a server sends. So the importer decodes open unions itself, by
the tag: every class with a field holding one gets a generated
`baml.FromJson`, and every operation returning one passes a `decode` function
to `platform_decoded`/`manager_decoded`. Each `match`es on the tag, decodes a
known tag as its own class with `baml.json.to<Member>` (so a known member
that does not decode still fails) and any other tag as the `Other` class; a
union tagged by several fields nests one `match` per field. The class's
other fields decode with `baml.json.to<T>` as usual:

```baml
class APIKeyDeploymentSetupConfigItemsItem {
    item: string,
    source: SetupItemStatusSourceProjectRelease | SetupItemStatusSourceBuiltIn | SetupItemStatusSourceOther,
    …
    implements baml.FromJson {
        function from_json(j: baml.json.json) -> Self throws baml.json.DecodeError {
            match (j) {
                let object: map<string, baml.json.json> => APIKeyDeploymentSetupConfigItemsItem {
                    item: baml.json.to<string>(baml.json.field(object, "item")),
                    source: match (baml.json.field(object, "source")) {
                        let member: map<string, baml.json.json> => match (baml.json.to<string>(baml.json.field(member, "type"))) {
                            "project-release" => baml.json.to<SetupItemStatusSourceProjectRelease>(member),
                            "built-in" => baml.json.to<SetupItemStatusSourceBuiltIn>(member),
                            _ => SetupItemStatusSourceOther { type: "UNKNOWN", raw: member },
                        },
                        _ => throw baml.json.DecodeError { message: "expected a JSON object", path: "" },
                    },
                    …
                },
                _ => throw baml.json.DecodeError { message: "APIKeyDeploymentSetupConfigItemsItem must be a JSON object", path: "" },
            }
        }
    }
}
```

An optional union adds a `null => null` arm to the same `match`; a list of
them decodes each item with `items.map((item) -> { … })`, a map each value.
Each union is decoded where it is used rather than by a named function per
union: BAML has no private functions (docs/gaps.md#b6), so such a function
would be exported by all four generated SDKs. Encoding needs nothing
new: the `Other` class's `baml.ToJson` writes `raw` back. The Go and Rust
generators name a union after its members (`…ReceiverOr…Sender`), so adding a
member, `Other` or one Alien adds, renames that type. The importer refuses a
spec where a member's tag is `"UNKNOWN"`. Request types get the member too:
they share classes with responses, and it lets a caller send a member this
SDK does not know yet. Not opened: unions without such tags, and a known
member with a new value in another literal field (`DeploymentGroupScope.role`)
still fails.

**Documentation.** Schema and property descriptions become doc comments; a
property without one takes the description of a documentation-only `allOf`
part or of its only non-null union member. Two notes are appended where the
BAML type no longer says it: the known values of an enum decoded as `string`
(also for list items and map values) or of a widened literal, and the default
the server applies.
Parameter descriptions are listed in the function's doc comment.

## Operations → BAML functions

**Naming follows Alien's TypeScript SDKs** (which re-apply their Speakeasy
overlays), so a call reads the same in every language:

| Incumbent TypeScript | Generated BAML | TypeScript | Python |
|---|---|---|---|
| `alien.managers.get({ id })` | `platform.managers.get(client, id)` | `alien.managers.get(id)` | `alien.managers.get(id)` |
| `alien.deployments.list({ project, limit })` | `platform.deployments.list(client, project? = null, …, limit? = null)` | `alien.deployments.list({ project, limit })` | `alien.deployments.list(project=…, limit=…)` |
| `alienManager.commands.createCommand(body)` | `manager.commands.createCommand(client, body)` | `new AlienManager(o).commands.createCommand(body)` | `AlienManager(url, token).commands.create_command(body)` |

Go calls generated methods (`c.Managers.Get(ctx, id)`, written into
`alien/operations.go` by `sdk/go/internal/genops`) and Rust the generated
functions (`platform::managers::get_async(client, id)`); see
`docs/api-shape.md`.

- Platform: group = `x-speakeasy-group`, else camelCase of the first tag (41
  operations, as Speakeasy does); method = `x-speakeasy-name-override`, else
  camelCase of the operation ID; plus the overlay's rename (`POST
  /v1/deployment-groups/{id}/tokens` → `createToken`, `method_renames` in the
  config). Operations marked `x-speakeasy-ignore` are not imported.
- Manager: group = first tag, method = camelCase(operation ID).
- `tests/fixtures/naming-oracle.json`, extracted from the incumbent SDK sources
  (`scripts/extract_naming_oracle.py`), checks every generated name
  (`tests/naming_oracle.rs`: 279 of 279 Platform and 53 of 53 Manager match).
- BAML accepts `delete`, `get`, `list`, `new` as function names; the generators
  handle host keywords (TypeScript exports `delete` through an alias, Python
  renames `import` to `import_`).

**Arguments.** Required parameters are positional, in this order: path
parameters as the path uses them, other required parameters in spec order,
then a required body. Optional parameters follow in spec order, then an
optional body, each as a BAML default argument (`limit: int? = null`), which
every binding lets the caller omit: an options object in TypeScript, keyword
arguments in Python, `With…` call options in Go, `OptionalArg` in Rust. A
function is callable with the client alone exactly when Alien's TypeScript SDK
makes its request argument optional (`tests/naming_oracle.rs` checks all 332
operations). Argument names follow the field-name rule, as Alien's TypeScript
SDK names its request keys: a wire name with separators is camelCased
(`command_id` → `commandId`, `If-Match` → `ifMatch`; 4 wire names in 10
operations), any other is kept. Then a BAML reserved word gets a trailing
underscore, and two arguments with the same name fail generation. The request still uses the
wire name. `tests/naming_oracle.rs` checks every argument name against the
SDK's request types (`param_names` in the oracle).

`workspace` is a global (Platform overlay): it is never a function argument;
operations that accept it send `client.workspace` when set.

**Parameters.** Path parameters are percent-encoded (RFC 3986 unreserved set).
`""`, `.` and `..` are refused with `INVALID_REQUEST`: they would reach a
different endpoint. Query parameters use form style with `explode` (the spec's
default): `workspace` first, then the operation's parameters in spec order;
arrays repeat the key, unset parameters are omitted, and a whole `number` is
written as JavaScript writes it (`limit=2`, not `2.0`). Header parameters are
sent when set, under their lowercased names. A query string on the client's
base URL (`https://h/v1?token=x`) stays at the end of the URL, before the
operation's parameters. Literal parameter types, closed enums included, are
widened to their base type (`"a"[]` → `string[]`): a parameter is only
serialized. The helpers serialize
only what Alien's specs use, and anything else fails generation: path
parameters must be scalars; query parameters strings, integers, numbers or
string lists; header parameters strings, one per operation at most; and every
parameter must use its location's default `style` and `explode`, without
`allowReserved`.

**Responses.**

| 2xx responses | Return type | Decoding |
|---|---|---|
| JSON with one schema | `T` | `baml.json.decode<T>` (honors `FromJson`) |
| JSON with several schemas | `A \| B` | first member that decodes |
| JSON and an empty one (202 JSON / 204) | `T?` | `null` for an empty body |
| no content | `null` | body read and discarded |

**Errors.** Every function throws `root.errors.AlienError`:

| Failure | Platform | Manager |
|---|---|---|
| non-2xx with the service's error body | server's `code`, `message`, `retryable` (absent = false) | server's `code`, `message`; not retryable; `details` in context |
| non-2xx with any other body | `PLATFORM_HTTP_ERROR`, retryable on 408, 429, 5xx | `MANAGER_HTTP_ERROR`, not retryable (as `@alienplatform/commands`) |
| connection, timeout or body-read failure | `COMMUNICATION_ERROR`, retryable (with the HTTP status when the body could not be read) | same |
| 2xx body that does not decode | `MALFORMED_RESPONSE` | same |

## Overlays

The importer does not run Speakeasy; it re-applies what each overlay is for.

| Overlay | Intent | How |
|---|---|---|
| `client-sdks/speakeasy-overlay.yaml` | `x-speakeasy-max-method-params: 2` on body operations | Not applied: required arguments are positional and optional ones are named, in every operation (see Arguments) |
| `platform/speakeasy-overlay.yaml` | `workspace` as a hidden global parameter; `createToken` rename; one description | `client.workspace`; the rename is applied by operation path and method; the description is not used |
| `platform/sdk-compatibility-overlay.json` | Re-adds schemas, properties and operations removed from the server API so published SDK types and methods keep existing | Each `update` action is merged at its target (`$['…']['…']` paths, schemas and paths alike), adding only what is absent; a value that differs from the spec's fails the import. Types are generated only when an imported operation references them; `config.json` imports every operation |

## What the importer refuses

It fails, naming the JSON pointer, on: an operation without `operationId`; a
duplicate generated name (type, function, field, argument, group); an
unresolvable `$ref`; conflicting properties in an `allOf` merge, or a merged
part with `additionalProperties`; an overlay value that conflicts with the
spec; a parameter without a schema, with a serialization other than its
location's default, or of a type it cannot serialize; more than one header
parameter; a path template that is malformed or does not match the path
parameters; a request or response content type other than `application/json`;
a required request body whose type admits `null` (`json_body` would send no
body); a response that may be empty and holds an open union (none of these in
Alien's specs today).
Constructs it accepts but cannot type precisely (for example an untyped schema
with `not`, `if` or `patternProperties`, or an `allOf` part that is not an
object) become `json` and are counted in `docs/importer-coverage.md`.

## Decisions and other policies

`docs/api-shape.md` ("Decisions") records the decisions this design follows,
all made: open enums and unions, incumbent-style naming, the argument rule,
Manager authentication and errors, and explicit `null` in requests.

`open_enums` also supports `"rust-filter"` (only the two fields Alien's Rust
filter opens decode as `string`; the import fails if either is no longer an
enum, as that filter does) and `"none"`. Both keep every other enum a closed
union of literals, single-value ones included. The SDK uses neither, and with
BAML 0.23.0 their output does not compile: a class with its own
`baml.FromJson` decodes such a field with
`baml.json.to<"aws" | "gcp" | …>(…)`, which BAML reads as comparisons
(`operator | cannot be applied to bool and "gcp"`). The Rust generator would
also skip every type with a non-identifier literal such as
`"deployment-group"` ([B20](../../docs/gaps.md#b20)).
