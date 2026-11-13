# Public API shape

This is the shape every operation takes in the four SDKs; [Decisions](#decisions)
lists the choices behind it. All 332 operations of Alien's specs are
imported: 279 Platform operations (every one but the 3 marked
`x-speakeasy-ignore`, which Alien's TypeScript SDK leaves out too) and 53
Manager operations. Each runs in all four languages against a local stub
(`conformance/`).

Everything below comes from two places: one rule set in the importer
(`tools/openapi2baml/DESIGN.md`) and one generic binding per facade. No facade
contains per-operation code.

## Calling an operation

Six representative operations:

| Operation | What it shows |
|---|---|
| `managers.get` | one required argument |
| `deployments.list` | only optional arguments |
| `deploymentGroups.createToken` | a path parameter and an optional body |
| `auth.whoami` | no arguments |
| `apiKeys.revoke` | no response body |
| `commands.createCommand` | a Manager operation |

**TypeScript.** Calls take the form `alien.<group>.<method>`, or
`alien.<method>` for an operation Alien's SDK has outside any group
(`alien.acceptWorkspaceInvitation(token)`; 39 Platform operations), and the client
options have the names of Alien's TypeScript SDK: `new Alien({ apiKey,
serverURL, workspace, timeoutMs })`, with `apiKey` and `workspace` defaulting to
`ALIEN_API_KEY` and `ALIEN_WORKSPACE`. `apiKey` can also be an async function,
called before every request. The Manager client is `new AlienManager({
serverURL, token, timeoutMs })` (`token` as in `@alienplatform/commands`; Alien's Manager
SDK calls it `bearer`). Calls are async and camelCase.
Required arguments are positional. Optional arguments go in a trailing options
object, which can be omitted. Request bodies are plain objects; their types
are in the exported `platform` and `manager` namespaces, which hold types only:
the SDK exports no runtime schema (Zod or another) to validate input with.
A union member this SDK does not know (see Unions below) has the tag
`"UNKNOWN"`, so a `switch` on the tag covers it like any other member, and
TypeScript checks that the switch is exhaustive.

```ts
import { Alien, AlienManager } from "@alien-baml/sdk"

const alien = new Alien()   // apiKey from ALIEN_API_KEY
const m = await alien.managers.get(id)
const page = await alien.deployments.list({ project: "my-project", limit: 10 })
const t = await alien.deploymentGroups.createToken(id)
const me = await alien.auth.whoami()
await alien.apiKeys.revoke(keyId)
for (const e of (await alien.events.list()).items) {
  switch (e.data.type) {
    case "DeploymentFailed": console.log(e.data.error.message); break
    case "UNKNOWN": console.log("new event type", e.data.raw.type); break
  }
}
const cmd = await new AlienManager({ serverURL: managerUrl, token }).commands.createCommand({
  deploymentId, command: "health-check", params: { mode: "inline", inlineBase64: "e30=" },
})
```

**Python.** Calls take the form `alien.<group>.<method>`, as in Alien's
TypeScript SDK. They are synchronous; `AsyncAlien` and `AsyncAlienManager`
have the same methods, `async`. The client takes the TypeScript SDK's options
in snake_case: `api_key` (default: the `ALIEN_API_KEY` environment variable),
`server_url`, `workspace` and `timeout` (in seconds, where `timeoutMs` is
milliseconds; decision 9). Optional arguments are keyword arguments. Groups,
methods, keyword arguments and fields are snake_case, as BAML's `language`
naming generates them: a multi-word filter reads
`deployment_group=` and a page's cursor `page.next_cursor`. The wire names do
not change, and each field keeps the BAML name as its pydantic alias, so
models also accept it (`CreateCommandRequest(deploymentId=…)`) and
`model_dump(by_alias=True)` writes it. Every method has the generated
operation's signature, without the client, so editors and type checkers see
it (mypy: `reveal_type(alien.deployments.list)` shows each keyword and the
return type), and an unknown keyword raises `TypeError` before any request,
the BAML name (`deploymentGroup=`) included. The methods are generated from
the generated package's stubs by `sdk/python/genops.py` into
`alien_sdk/_operations.py` (gitignored; `scripts/generate.sh` runs it, as
`sdk/go/internal/genops` writes Go's methods): one class per group, blocking
(`PlatformDeployments`, calling the generated `list_sync`) and async
(`AsyncPlatformDeployments`, awaiting the generated `list`), with the same
methods, so there is no per-operation code and no casing rule of the
facade's own. An optional argument the caller leaves out is `UNSET`, which
the generated function does not pass on, as before.
`alien_sdk/_gen.py` is the only module that imports the generated package,
`alien_sdk._generated` (`package_name` in `baml/baml.toml`): the wheel holds
the one package `alien_sdk`, so it never overwrites another BAML-generated
SDK's `baml_sdk` ([gaps B4](gaps.md#b4)).

```python
from alien_sdk import Alien, AlienManager, manager

alien = Alien()   # api_key from ALIEN_API_KEY
m = alien.managers.get(id)
page = alien.deployments.list(project="my-project", deployment_group="prod", limit=10)
page.next_cursor
t = alien.deployment_groups.create_token(id)
me = alien.auth.whoami()
alien.api_keys.revoke(key_id)
cmd = AlienManager(manager_url, token).commands.create_command(manager.CreateCommandRequest(...))
```

**Python: validating input.** `alien_sdk.platform` and `alien_sdk.manager`
hold every request and response type of each API: the generated pydantic
models, enums and type aliases, under their generated names, the same
classes the operations take and return (the generated client is left out).
They are modules (`from alien_sdk.platform import CreateSetupLinkRequest`),
one per API, so the 61 names both APIs define (`CreateCommandRequest`,
`CreateReleaseRequest`, …) never clash. They are written by `sdk/python/genops.py`
(gitignored). A customer can validate their own input, such as a dashboard
form, with them before calling the API:

```python
from alien_sdk.platform import CreateSetupLinkRequest

request = CreateSetupLinkRequest.model_validate(form)   # pydantic.ValidationError if it does not fit
alien.setup_links.create(request)
```

`model_validate` (and `model_validate_json`) accept both spellings of a
field: its pydantic alias, which is the BAML name
(`{"externalId": …, "maxDeployments": 5}`), and the Python name
(`{"external_id": …, "max_deployments": 5}`); the models set
`populate_by_name`. For most fields the BAML name is the wire name, so data
in the API's shape (JSON bodies, forms built from the API reference)
validates as it is, `model_dump(by_alias=True)` writes it, and a
`ValidationError` names the field by it (`externalId: Field required`). Use
the Python spelling for keyword arguments in Python code
(`CreateSetupLinkRequest(external_id=…)`). Do not send both for one field:
the alias wins. Unknown keys are ignored (`extra="ignore"`), so a misspelled
optional field is dropped without an error; values are converted as
pydantic's default (lax) mode does (`"5"` → `5`).
`sdk/python/tests/test_types.py` checks each of these.

Three kinds of field do not follow the API's JSON in pydantic
([B27](gaps.md#b27)):

- A field with `@json.rename`. Its alias is the BAML name (`onOther` for the
  wire name `on-other`, `vpcId` for `vpc_id`), or it has none (`test_`, wire
  name `test`), so `model_validate` reads the wire name only where it is also
  the Python name (`vpc_id`), and `model_dump(by_alias=True)` writes the
  alias.
- `additionalProperties`, which holds the keys a class does not list:
  pydantic requires them under an `additionalProperties` key and ignores
  them beside the listed fields, where the API sends them.
- A union's `Other` member: pydantic accepts it only as
  `{"<tag>": "UNKNOWN", "raw": {…}}`, not as the member the server sent.

The SDK's own requests and responses are not affected: it reads and writes
them through BAML, which follows the `@json` attributes.

**Go.** Calls take the form `c.<Group>.<Method>(ctx, …)`, grouped and named
as in Alien's TypeScript SDK with Go's initialisms (`c.APIKeys.Revoke`,
`c.Projects.GetTemplateURLs`); an operation without a group is a method of
the client (`c.AcceptWorkspaceInvitation(ctx, token)`). Required arguments are
positional. Optional ones are a trailing `*<OperationID>Params`
(`*Manager<OperationID>Params` for the Manager; `nil` sets none; a `nil` field
is left out, so optional strings and integers are pointers, set with
`alien.Ptr`), or the request body itself when it is the only optional
argument. Failures are `*alien.AlienError` (B5). The methods are generated
from the generated package by `sdk/go/internal/genops` into
`alien/operations.go`, so there is no per-operation code. Request and
response types are in the generated package, `alienapi` (`package_name` in
`baml/baml.toml`), named `alienapi.Platform<Name>` and
`alienapi.Manager<Name>`. The Manager client's generated client is
`m.Generated`, because `m.Manager` is the Manager API's `manager` group
(`m.Manager.ManagerInfo(ctx)`). Printing `c` never shows the API key, but
`%v`, `%+v` and `%#v` of `c.Platform`, the generated client, do: so
`log.Printf("%+v", c.Platform)` logs the key. `json.Marshal` of `c`,
`c.Platform`, `m` or `m.Generated` writes the key or the token
([B22](gaps.md#b22)).

```go
c, err := alien.New(alien.Options{APIKey: key})
m, err := c.Managers.Get(ctx, id)
page, err := c.Deployments.List(ctx, &alien.ListDeploymentsParams{Project: alien.Ptr("my-project"), Limit: alien.Ptr[int64](10)})
t, err := c.DeploymentGroups.CreateToken(ctx, id, nil)
me, err := c.Auth.Whoami(ctx)
err = c.APIKeys.Revoke(ctx, keyID)
cmd, err := alien.NewManager(alien.ManagerOptions{ServerURL: managerURL, Token: token}).Commands.CreateCommand(ctx, req)
```

**Rust.** Calls take the form `alien.<group>().<method>(…)`, grouped as in
Alien's TypeScript SDK; an operation without a group is a method of the client
(`alien.accept_workspace_invitation(token)`). The method takes the required
arguments (each `impl Into<T>`, so `&str` works for a `String`) and returns a
builder with one setter per optional argument, also `impl Into<T>`;
`.send().await` makes the call and returns `Result<T, alien_sdk::Error>`.
`.baml_options(BamlOptions::new().timeout_ms(10_000))` sets per-call options.
An optional parameter Alien adds is one more setter, so existing calls keep
compiling (v3.3.43 added `channel`, `effectScope` and `effect` to
`releases.listDeployments`, which positional arguments would have broken).
Names are BAML's `language` naming: snake_case modules, functions, arguments
and fields (`deployment_groups().create_token(id)`, `page.next_cursor`),
PascalCase types and enum variants; the wire names do not change. The
builders are generated from the generated crate by `sdk/rust/genops` into
`src/operations.rs` (as `sdk/go/internal/genops` writes Go's methods), so
there is no per-operation code: one builder per operation, 279 Platform and 53
Manager, in `alien_sdk::operations::{platform, manager}`, named by joining the
generated module and function (`DeploymentsList`). Request and response types
are in `alien_sdk::platform` and `alien_sdk::manager`, which re-export the
generated types and nothing else. This is the shape of Alien's Rust client
(Progenitor), grouped like the other three SDKs instead of flat
(`client.list_deployments()`). `Options` holds Alien's TypeScript SDK options
in snake_case (`api_key`, `server_url`, `workspace`, `timeout`). `api_key:
None` reads `ALIEN_API_KEY`; with no key, `Alien::new` fails with
`INVALID_REQUEST`. The generated clients, whose `Debug` shows the key and the
token ([B22](gaps.md#b22)), are private to the facade: `Debug` of `Alien`,
`AlienManager`, every group and every builder shows neither
(`tests/client_options.rs`).

```rust
use alien_sdk::{Alien, AlienManager, Options};

let alien = Alien::new(Options { api_key: Some(api_key), ..Default::default() })?;
let m = alien.managers().get(id).send().await?;
let page = alien.deployments().list().project("my-project").limit(10).send().await?;
let t = alien.deployment_groups().create_token(id).send().await?;
let me = alien.auth().whoami().send().await?;
alien.api_keys().revoke(key_id).send().await?;
let cmd = AlienManager::new(manager_url, Some(token)).commands().create_command(req).send().await?;
```

**Rust: request bodies.** A request body is a generated struct, passed
whole (`alien.setup_links().create(body)`). Every generated struct has a
`new` that takes its required fields, in declaration order, and sets every
optional field to `None`:
`CreateSetupLinkRequest::new(external_id: String, name: String, project: String)`.
Set optional fields with struct update syntax over it:

```rust
use alien_sdk::platform::CreateSetupLinkRequest;

let body = CreateSetupLinkRequest {
    max_deployments: Some(5),
    ..CreateSetupLinkRequest::new("cust-1".into(), "Customer".into(), "web".into())
};
let link = alien.setup_links().create(body).send().await?;
```

When Alien adds an optional field (v3.3.43 added `returnUrl` and `refreshUrl`
to `CreateSetupLinkRequest`), this code keeps compiling and the new field is
`None`. A struct literal that names every field stops compiling: the structs
are not `#[non_exhaustive]`, so the compiler accepts such a literal until a
field is added. A required field Alien adds is one more argument of `new`, so
both forms stop compiling. There is no `Default`, which would have to fill
required fields with `""` and `0`.

`tests/shape.rs` compiles the calls and the request body above.

## Rules

| Topic | Rule |
|---|---|
| Groups and methods | As in Alien's TypeScript SDK: `x-speakeasy-group`, else camelCase of the first tag; `x-speakeasy-name-override`, else camelCase of the operation ID. An operation with neither group nor tag (39 Platform operations) is a method of the client itself, as in that SDK. One config rename (`POST /v1/deployment-groups/{id}/tokens` → `createToken`). Checked against names extracted from the SDK's sources: 279 of 279 Platform and 53 of 53 Manager operations match (`tests/naming_oracle.rs`), and the TypeScript, Python and Rust facade tests call each of them by that name (Rust: `every_operation_has_a_builder` in `tests/conformance.rs`). Operations marked `x-speakeasy-ignore` are left out, as Speakeasy does (3: `listPendingAccessRequests`, `reportMaterializedAccessRequests`, `reportAccessApprovals`) |
| Generated async and blocking functions | BAML #5519: in TypeScript, Python and Rust the plain generated name is async (`platform.deployments.list`, `platform::deployments::list`) and its blocking twin is `<name>_sync`; Rust's per-call-options variants are `<name>_with_options` and `<name>_sync_with_options`. Go has one blocking function per operation, taking a `context.Context`. The facades pick one per client: TypeScript and Rust bind the async functions, Python's `Alien` and `AlienManager` the `_sync` twins and `AsyncAlien` and `AsyncAlienManager` the async ones, all under the plain name (`alien.deployments.list`), so the facades' public API does not depend on the suffix |
| Python and Rust names | BAML's `language` naming (BAML #5481): groups, methods, arguments and fields in snake_case (`deploymentGroups.createToken` → `deployment_groups.create_token`, `nextCursor` → `next_cursor`), types and enum variants in PascalCase. An acronym is one word (`revokeAPIKey` → `revoke_api_key`), and so is a known word such as `OAuth`, `GraphQL` or `GitHub` (BAML #5524: `getGcpOAuthProvider` → `get_gcp_oauth_provider`). A Python keyword gets a trailing underscore (`deployments.import` → `deployments.import_`), a Rust keyword is a raw identifier (`deployments().r#move(id)`). The facades take these names from the generated code and add no casing rules. Wire names do not change |
| Operations newer than the SDK | None at the vendored commit: Alien's TypeScript SDK has every Platform and Manager operation of the spec. One that is newer would be named by the same rule |
| Arguments | Required arguments are positional: path parameters, then other required parameters, then a required body. Optional parameters and an optional body come after them as optional arguments. An operation can be called with no arguments exactly when Alien's SDK makes its request optional (all 332 operations checked) |
| Workspace | The `workspace` query parameter is a client setting, sent on every operation that accepts it |
| Types | Component schemas keep their names. Operation-level schemas use Alien's names: `<OperationId>Response`, plus `<OperationId>Request` for a body that is the only argument, else `<OperationId>RequestBody`. Other inline schemas are named `<Parent><Field>`. Identical classes are merged |
| Optional fields | Unset optionals are omitted from request bodies. A request cannot send an explicit `null` (decision 5; [B13](gaps.md#b13) lists the fields where the spec gives `null` a meaning) |
| Extra keys | An object with both `properties` and `additionalProperties` keeps the unlisted keys in an `additionalProperties` map, as Speakeasy names it (34 schemas) |
| Field and argument names | BAML names as in Alien's TypeScript SDK (decisions 11 and 12), which TypeScript and Go use as they are and Python and Rust snake_case (Python and Rust names, above): a wire name with separators is camelCased (`custom_domains` → `customDomains`, `vpc_id` → `vpcId`, `on-other` → `onOther`, `_internal` → `internal`; 21 Platform and 14 Manager wire names), any other is kept. A BAML reserved word then gets a trailing underscore (`test` → `test_`, B14). Requests and responses keep the wire names on the wire, through `@json.rename`; `docs/importer-coverage.md` lists every renamed field. Arguments follow the same rule: the Manager's path parameters `command_id`, `lease_id` and `vault_name` are `commandId`, `leaseId` and `vaultName`, and the Platform header `If-Match` is `ifMatch` (`alien.dynamicContainers.put(id, name, body, { ifMatch: etag })`). `tests/naming_oracle.rs` checks every field and argument name against the SDK's sources |
| Enums | Open: a string enum becomes `string`, so values a newer server adds still decode. Its values are listed in the doc comment. A single-value enum or constant stays a literal only where it tags a union member, so the union decodes unambiguously; anywhere else it becomes its base type (`string`, `int`, `bool`), with its value in the doc comment |
| Unions | Open: a union whose members share a tag field (`type`, `platform`, …) has one more member, last, named `<Owner><Field>Other` (`EventDataOther`). Its tag is always `"UNKNOWN"` and `raw` holds the member as the server sent it (the server's tag is `raw.type`), as in Speakeasy's forward-compatible unions. So a member a newer server adds decodes instead of failing the call, and a switch on the tag handles it like any other member: TypeScript `case "UNKNOWN":` (exhaustiveness-checked with a `never` default), Python `match` on the class or the tag, Go a type switch (`case alienapi.PlatformEventDataOther:`), Rust a `match` on the union's `EventDataOther` variant. A known tag that does not decode still fails with `MALFORMED_RESPONSE`. A union tagged by two fields has the same one catch-all (decision 13): `ExternalBinding` is tagged by `type`, then by `service` where several bindings share a `type`, and a new `type`, or a known `type` with a new `service` (`{"type": "storage", "service": "minio", …}`), decodes as `ExternalBindingOther { type: "UNKNOWN", raw }`, with the server's `type` and `service` in `raw`: TypeScript `if (b.type === "UNKNOWN") b.raw.service`, Python `case platform.ExternalBindingOther(raw=raw):`, Go `case alienapi.PlatformExternalBindingOther:`, Rust `platform::ExternalBinding::ExternalBindingOther(o) => o.raw["service"]`. A known pair that does not decode (`storage`/`s3` with `forcePathStyle: "yes"`) still fails with `MALFORMED_RESPONSE`. Unions without such tags stay closed |
| Responses | 2xx JSON → the type; JSON or empty → `T?`; no content → `null`. Unknown fields are ignored. A required field the spec leaves untyped (`json`) may be missing, and is then `null`. A number too large for a 64-bit float (`1e400`), even in an ignored field, fails the body with `MALFORMED_RESPONSE` |
| Errors | One `AlienError` (code, message, retryable, HTTP status, context, hint, request ID, `internal`, `source`) in every language ([errors.md](errors.md)). Platform error bodies keep Alien's code, retryable flag, hint, request ID, context, `internal` flag and `source` chain, and each language's cause chain follows `source`. Manager error bodies keep code and message, the same four fields when the body has them, and are never retryable. Empty, `.` and `..` path parameters and malformed base URLs fail with `INVALID_REQUEST` before any request |
| Manager authentication | The token is sent on every Manager operation when the client has one, as `@alienplatform/commands` does |

## What differs from Alien's SDKs

| | Alien's TypeScript SDK | This SDK |
|---|---|---|
| Required parameters without a body | Request object: `managers.get({ id })` | Positional: `managers.get(id)` |
| Optional body | Second positional argument: `createToken(id, body?)` | Options object: `createToken(id, { body })` |
| Request bodies | Plain objects; unset keys can be left out | TypeScript: plain objects or generated classes; unset keys can be left out. Python and Go default unset fields. Rust: `T::new` takes the required fields and leaves the optional ones `None`, and struct update syntax over it sets the others, so an optional field Alien adds does not break code that builds a body that way ([Rust: request bodies](#calling-an-operation)) |
| Explicit `null` in a request | Possible (Alien's Rust SDK cannot) | Not possible: unset fields are left out (decision 5) |
| Enums | Closed: an unknown `Manager.status` fails decoding | Open (decision 1) |
| Responses the server leaves fields out of | Fills in the spec's default (`maxDeployments: 100`) | Leaves them unset (`null`) |
| An error body on an operation whose spec declares no error response (or not that status) | Not read: `AlienDefaultError`, no code | Read: `AlienError` with the server's code |
| Unions | Closed: an event type newer than the SDK fails `events.list`, and a binding `service` newer than the SDK fails `deployments.get` | Open: they decode as `EventDataOther` and `ExternalBindingOther` (decisions 1 and 13) |
| Manager authentication | Token on every request (`@alienplatform/commands`) | The same |
| TypeScript client options | `serverURL`; `apiKey` (a string or an async function; `ALIEN_API_KEY`); `workspace` (`ALIEN_WORKSPACE`). Manager (`@alienplatform/manager-api`): `serverURL`; `bearer` (a string or an async function; `ALIEN_MANAGER_BEARER`) | The same for the Platform client. Without an API key, `new Alien()` throws `AlienError` `INVALID_REQUEST`; Alien's SDK sends requests without one. Manager: `serverURL` and `token` (a string, no environment default), as in `@alienplatform/commands` |
| Timeout | `timeoutMs` per HTTP request, on the client and per call; a per-call `timeoutMs` replaces the client's; a timeout throws `RequestTimeoutError` | `timeoutMs` per HTTP request on the client (decision 9). A per-call `{ $baml: { timeoutMs } }` limits the whole call as well, and the shorter one ends it, so it can shorten the client's limit but not lengthen it. A client timeout fails with `AlienError` `COMMUNICATION_ERROR` (retryable); a per-call one throws `BamlAbortError` |
| Other TypeScript client options | `retryConfig`, `httpClient`, `userAgent`, `serverIdx`, `debugLogger` (`ALIEN_DEBUG`) | Not supported. Per call: `{ $baml: { timeoutMs, signal } }` |
| TypeScript errors | `APIError` for an API error body (`code`, `retryable`, `hint`, `requestId`, `context` as JSON, `statusCode`, `body`, `headers`), other classes for connection, timeout and validation failures | One `AlienError` for all of them: `code`, `message`, `retryable`, `statusCode` (`null` without a response), `hint`, `requestId`, `internal`, `source` (the server's chain, each level an `AlienError`; also the error's `cause`), and `context` as strings (the server's context is JSON text in `context.details`). No `body` or `headers` ([errors.md](errors.md)) |
| Client options in Python, Go and Rust | Alien has no Python or Go SDK; its Rust client (Progenitor) takes a base URL, `Client::new(baseurl)` | The TypeScript SDK's options in each language's casing: Python `Alien(api_key, *, server_url, workspace, timeout)`, Go `alien.Options{APIKey, ServerURL, Workspace, Timeout}`, Rust `Options { api_key, server_url, workspace, timeout }`. The API key defaults to `ALIEN_API_KEY`; without one, creating the client fails with `INVALID_REQUEST`. Go also reads `ALIEN_WORKSPACE`; Python and Rust do not. Only TypeScript takes `apiKey` as a function. Manager: a server URL, `token` and the timeout (Python `AlienManager(server_url, token, *, timeout)`, Go `alien.NewManager(alien.ManagerOptions{ServerURL, Token, Timeout})`, Rust `AlienManager::new(server_url, token).with_timeout(..)`); an empty token sends no `Authorization` header |

## Rust incumbent differences

Alien's Rust clients (`alien-platform-api` and `alien-manager-api`, generated
by Progenitor) run the same scenarios as the TypeScript SDK
(`conformance/incumbents/README.md`). Where they differ from it, this SDK
follows the TypeScript SDK. The table lists where they differ from this SDK.

| | Alien's Rust clients | This SDK |
|---|---|---|
| Operations | Default features: the 74 Platform operations Alien's own code calls; `full-api`: all 282. Manager: all 53 | All 332 in every language |
| Calls | Builders, flat: `client.list_deployments().project(p).limit(10).send().await?.into_inner()`; `workspace` is an argument of each call; authentication is a `reqwest::Client` with a default `Authorization` header | Builders, grouped as in the other SDKs: `alien.deployments().list().project(p).limit(10).send().await?`; `workspace` and the API key are client options |
| An optional request body left out | Sent anyway, as the body type's default: `{}`, or `INVALID_REQUEST` before sending when that type has required fields (98 operations) | Not sent |
| String constraints (`pattern`, `minLength`, `maxLength`, `format: uuid`) | Checked on arguments (refused before sending) and on responses (`INVALID_RESPONSE_PAYLOAD`) | Not checked: the server decides |
| Enums | Closed, apart from the three response fields Alien's filter opens (`DeploymentResource.status` and two lists of package types): an unknown value in `Manager.targets` fails the response | Open (decision 1) |
| Unions | Closed: an external binding with a new `service` or `type` fails `deployments.get` | Open (decisions 1 and 13) |
| Fields Alien's compatibility overlay re-adds (`operations.listPlugins` `installations`, `operations.publishPlugin` `enabled`, `permissionDiff`) | Dropped: the client is generated without the overlay | Kept, as in the TypeScript SDK |
| Keys a type does not list | Kept by the Platform client, dropped by the Manager client | Kept under `additionalProperties` |
| An error on a status the operation does not declare | `UNEXPECTED_RESPONSE`, body not read; retryable at 5xx only (Manager: also 408, 425 and 429) | The server's code; `PLATFORM_HTTP_ERROR` (retryable on 408, 429, 5xx) or `MANAGER_HTTP_ERROR` without an Alien error body |
| A body that is not JSON, or a malformed 200 | `INVALID_RESPONSE_PAYLOAD`, without the HTTP status | `PLATFORM_HTTP_ERROR` or `MALFORMED_RESPONSE`, with the status |
| Manager error bodies | Read when they are a whole Alien error (`code`, `message`, `retryable`, `internal`); a 5xx is retryable | Read when they have `code` and `message`; `hint`, `requestId`, `internal` and `source` are kept when present; never retryable |
| serde on the generated types | `Serialize` and `Deserialize` in the wire format: `serde_json::from_value::<CreateSetupLinkRequest>(form)` works | The same: `alien_sdk` turns on BAML's `serde` feature, so every type in `alien_sdk::platform` and `alien_sdk::manager` implements both, following the `@json` attributes, and reads and writes the JSON the SDK sends and receives (`tests/serde.rs`); `alien_sdk::serde_json` is the `serde_json` they use. Two request classes differ: serde writes `expiresAt` (`CreateDeploymentTokenRequest`) and `statusCode` (`ReportLogExportDeliveryRequestBody`) as `null` when unset, where the SDK leaves them out ([B26](gaps.md#b26)) |

## Decisions

The choices behind the shape above, with the alternative where there was one.

1. **Enums.** Decided: open every enum (implemented). Alien's TypeScript
   SDK opens one (`DeploymentResource.status`) and its Rust filter opens
   three (that one and two lists of package types), so both reject an unknown
   `Manager.status`. The conformance scenario
   `platform.getManager.success_unknown_fields` requires accepting one.
   Alternative: open only what Alien's Rust filter opens, and change that
   scenario. Unions are open too (`open_unions`): a member newer than the SDK
   decodes as the union's `Other` member instead of failing the call.
2. **Naming.** Decided: Alien's names in every language: the TypeScript
   SDK's groups and methods (implemented and verified against it) and its
   client options (`apiKey`, `serverURL`, `workspace`, `ALIEN_API_KEY`), in
   each language's casing.
3. **Arguments.** Decided: required arguments positional and optional ones
   named (implemented). List calls then read exactly like Alien's SDK
   (`deployments.list({ project, limit })`), and no params classes are
   generated, except in Go, which has no named arguments: its optional
   arguments are one `<OperationID>Params` struct per operation, and Rust,
   whose optional arguments are a builder's setters
   (`deployments().list().project(p).limit(10).send()`), as in Alien's Rust
   client. Alternative: Speakeasy's request objects (`managers.get({ id })`),
   generated as `<OperationId>Params` classes.
4. **Manager.** Decided (implemented):
   - Send the token on every operation, as `@alienplatform/commands` does,
     although the spec declares security on only 36 of 53 operations (also a
     question for Alien in [gaps.md](gaps.md)).
   - Map errors as `@alienplatform/commands` does: code and message kept,
     `details` in context, never retryable. A body in Alien's full error
     shape also keeps its hint, request ID, `internal` flag and `source`
     chain ([errors.md](errors.md#the-servers-fields-and-the-source-chain)).
5. **Explicit `null` in requests ([B13](gaps.md#b13)).** Decided: request
   bodies are encoded by omission. An unset optional field is left out, as
   both of Alien's SDKs do (Speakeasy omits `undefined`, Progenitor skips
   `None`), and BAML keeps one value for an unset optional field, so a request
   cannot send an explicit `null`. This affects `pinRelease.releaseId`
   ("Set to null to resume its followed channel"), where Alien's CLI itself
   sends `{}`, and the other fields that give `null` a meaning, such as
   `updateProject.rootDirectory` and `updateProject.domainId` ([B13](gaps.md#b13)
   lists them; a question for Alien in [gaps.md](gaps.md#questions-for-alien)).
6. **Rust and `json` ([B19](gaps.md#b19)).** Decided: BAML maps `json` to
   `serde_json::Value` in Rust, as Progenitor does (BAML PR #5287, in
   BAML 0.23.0). Rust has every imported operation.
7. **TypeScript request bodies ([B15](gaps.md#b15)).** Decided: an omitted
   nullable key counts as `null`, and generated classes declare nullable
   fields optional (BAML PRs #5291 and #5377, in BAML 0.23.0). Request
   bodies are plain objects or classes, with unset keys left out.
8. **Python and Rust casing (B7).** Decided: BAML's `language` naming
   (BAML #5481), so Python and Rust names are snake_case everywhere
   (`deployment_groups.create_token`, `page.next_cursor`,
   `deployments.list(deployment_group="prod")`). The wire
   names do not change. Earlier the generators supported only
   `preserve-case` for both, so Python fields and keywords and all Rust names
   were camelCase.
9. **Timeouts.** Decided and implemented: a client-level timeout, as
   Speakeasy's `timeoutMs`, for the Platform and the Manager client. It is a
   `timeout_ms` field of the generated `PlatformClient` and `ManagerClient`,
   which `internal.send` applies to each HTTP request, from connecting to
   reading the whole response; the facades pass it through, or convert to it:

   | Language | Platform | Manager |
   |---|---|---|
   | TypeScript | `new Alien({ timeoutMs: 30_000 })` | `new AlienManager({ serverURL, token, timeoutMs: 30_000 })` |
   | Python | `Alien(timeout=30.0)` | `AlienManager(url, token, timeout=30.0)` |
   | Go | `alien.New(alien.Options{Timeout: 30 * time.Second})` | `alien.NewManager(alien.ManagerOptions{ServerURL: url, Token: token, Timeout: 30 * time.Second})` |
   | Rust | `Alien::new(Options { timeout: Some(Duration::from_secs(30)), ..Default::default() })` | `AlienManager::new(url, token).with_timeout(Duration::from_secs(30))` |

   Go takes a `time.Duration`, as `net/http`'s `Client.Timeout` and
   `context.WithTimeout` do, rather than a millisecond count; Rust takes a
   `std::time::Duration`. Both are rounded up to whole milliseconds. Unset is
   BAML's default of 5 minutes; zero or negative (Go: negative; Rust:
   `Duration::ZERO`) is no limit, as in Speakeasy. Python takes seconds as a
   float, as httpx does, also rounded up to whole milliseconds: unset is
   `DEFAULT_TIMEOUT` (the same 5 minutes) and `None` is no limit, as in
   httpx. Zero or negative raises `ValueError`: in Python 0 reads as "time
   out at once", not "no limit". A request that runs out fails with
   `COMMUNICATION_ERROR`, retryable, as other transport failures do (Alien's
   Rust client uses the same code for a timeout): `HTTP request GET
   /v1/managers/m1 failed: timed out after 300 ms (timeout)`.

   Per-call timeouts keep working as before: `$baml` in TypeScript options,
   `_baml=` in Python, the `ctx` deadline in Go and `.baml_options(..)` on a
   Rust builder.
   They limit the whole call, and the shorter of the two limits ends it. So a
   per-call timeout can shorten the client's for one call but not lengthen it
   (Speakeasy's per-call `timeoutMs` replaces the client's), and it ends the
   call with the binding's cancellation error, not `AlienError`: TypeScript
   `BamlAbortError`, Python `baml_bridge.BamlCancelledError`, Go
   `context.DeadlineExceeded`, Rust `Error::Runtime`.
10. **Python async.** Decided: `AsyncAlien` and `AsyncAlienManager` bind the
    generated async functions (`await AsyncAlien().managers.get(id)`; `Alien`
    binds their blocking `_sync` twins), with the same groups, methods,
    options and errors as `Alien` and `AlienManager`. They run through the
    same Platform scenarios as `Alien`.
11. **Field names with separators.** Decided: fields use the names of Alien's
    TypeScript SDK, which camelCases wire names with separators (21 in the
    Platform API, 14 in the Manager API; `billing.getEntitlements`: `features.customDomains` for
    `custom_domains`; stack settings networks: `vpcId`, `privateSubnetIds`).
    The importer maps them as it maps `test` → `test_`, with `@json.rename`, so
    the JSON on the wire does not change.
12. **Argument names with separators.** Decided: argument names follow the
    field-name rule, as in Alien's TypeScript SDK: `command_id` →
    `commandId`, `lease_id` → `leaseId`, `vault_name` → `vaultName` (Manager
    path parameters) and `If-Match` → `ifMatch` (`dynamicContainers.put`).
    TypeScript `{ ifMatch: etag }`; Python and Rust then snake_case the BAML
    name (`if_match=etag`, `.if_match(etag)`), and Go's
    `&alien.PutDynamicContainerParams{IfMatch: …}` is unchanged. The request
    is unchanged.
13. **Unions tagged by two fields.** Decided: one catch-all, shaped like
    every other open union's. `ExternalBinding` (the external bindings in a
    deployment's stack settings) has no single tag: `type` (`storage`,
    `queue`, …) and, where several bindings share a `type`, `service` (`s3`,
    `gcs`, …) tell its 31 members apart. A new `type`, or a known `type` with
    a new `service`, decodes as `ExternalBindingOther { type: "UNKNOWN", raw }`
    instead of failing the whole response, so code switches on `type` as for
    every other open union. The cost: a new storage service reports `type`
    `"UNKNOWN"`, although the server sent `"storage"`; that stays in
    `raw.type`. Alternatives: `service` also set to `"UNKNOWN"`, or one
    catch-all per shared `type` (`ExternalBindingStorageOther { type:
    "storage", service: "UNKNOWN", raw }`, 7 classes instead of 1).

    ```ts
    for (const b of Object.values(deployment.stackSettings.externalBindings ?? {})) {
      if (b.type === "UNKNOWN") console.log(b.raw.type, b.raw.service)  // "storage" "minio"
    }
    ```
