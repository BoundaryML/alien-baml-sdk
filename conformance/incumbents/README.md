# Incumbent baselines

Adapters that run the conformance scenarios through Alien's current SDKs, so
"same behavior" is measured rather than assumed. Results go to
`conformance/baseline/<implementation>/`.

| Adapter | SDK | Scenarios | Status |
|---|---|---|---|
| `speakeasy-typescript/run.mjs` | `@alienplatform/platform-api` and `@alienplatform/manager-api` 3.3.43 (Speakeasy) | `platform.*`, `manager.*` and every `ops.*` scenario | Every operation the SDKs have |
| `progenitor-rust/run.mjs` | `alien-platform-api` and `alien-manager-api` 3.3.43 (Progenitor, `InterfaceStyle::Builder`), twice: default features and `full-api` | The same | Default: the 74 Platform operations Alien's own Rust code calls, and all 53 Manager operations. `full-api`: every operation |
| `@alienplatform/commands`, `alien-commands-client` | | | Not built |

Build the incumbents first, from a checkout of Alien at the commit the specs
are vendored from (`specs/README.md`): `ALIEN_REF=/path/to/alien
bench/build.sh`. Then run `node
conformance/incumbents/speakeasy-typescript/run.mjs --phase 2`
(`INCUMBENTS_DIR` points it at SDKs built elsewhere) and `node
conformance/incumbents/progenitor-rust/run.mjs --phase 2` (`--variant default`
or `--variant full-api` for one of the two; both by default, about a minute
each after the first build). Output is templatized (fixture secrets, the stub
address and run-time instants), so two runs produce identical files. Both
adapters share how a scenario is judged and written (`judge.mjs`).

**How a scenario is judged.** The adapter calls the operation as Alien's SDK
names it. The Speakeasy adapter takes its call shape from
`tools/openapi2baml/tests/fixtures/naming-oracle.json`, with date-time fields
as `Date`s as that SDK types them. The Progenitor adapter finds the client
method by the operation's method and path, and sets each argument through its
builder method (`client.list_deployments().project(p).limit(10).send()`); see
"The Progenitor adapter" below.
- *Request parity:* method, path, query, the headers the scenario names, and
  the JSON body must match what the BAML SDKs send. Equivalent, so not a
  difference: a body field or query parameter the SDK fills with its spec
  default (the BAML SDKs leave unset fields out: decision 5 in
  `docs/api-shape.md`), a date-time written with milliseconds, query
  parameters in another order, and `content-type` sent with no body.
- *Outcome parity:* the decoded response must match what the BAML SDKs
  return, and an error must have the same code, retryable flag and status.
  A response field the SDK fills with its spec default (the BAML SDKs leave
  it unset) and a date-time written differently count as equal. To judge the
  outcome whatever the request was, the stub always receives the request the
  scenario expects.
- An error body becomes `{code, retryable, http_status}` plus the context the
  SDK reports; any other error `{code: "(<ErrorClass>)", http_status}`.

## The Progenitor adapter

`progenitor-rust/` is a small Rust program (`src/main.rs`) that reads one call
per line, makes it through the clients, and prints what each returned;
`run.mjs` drives it.
- **Building.** For each variant, `run.mjs` builds the clients from
  `.cache/incumbents/alien` (Cargo target in `.cache/incumbents/progenitor`),
  reads the code Progenitor generated (`codegen.rs`), and writes the
  operation table the program includes: one row per client method, setting
  each parameter through its builder method with the type that method takes
  (`a.take::<types::ListDeploymentsName>("name")`). The table is generated, so
  there is no per-operation code, and the compiler checks every row.
- **Calling.** The scenario's arguments are converted to those types with
  serde, as a Rust caller would build them. A value the types refuse (a
  `pattern`, a UUID) is recorded as refused, and no request is sent. The
  client gets the token as a default `Authorization` header and the
  timeouts of `Client::new` (15 s), as Alien's CLI builds it, or the
  scenario's timeout. `workspace` is an ordinary parameter of these clients:
  it is set from the scenario's client, as the BAML SDKs send it.
- **Recording.** The client sends to a local proxy that records each
  request and forwards the request the scenario expects to the stub, as the
  Speakeasy adapter's HTTP client does.
- **Outcomes.** A response is serialized again (wire names) and compared
  with the BAML result under its BAML names (`custom_domains` is
  `customDomains`; keys a class keeps under `additionalProperties`). An
  error is reduced as Alien's own code reads it: `convert_sdk_error`, which
  `into_sdk_error()` calls, gives the code, retryable flag and status; the
  context counts only when it came from an error body. The 4 Platform
  operations whose spec declares no error schema have no `into_sdk_error()`,
  so their errors are the client's own (`(UnexpectedResponse)`).

## Findings ([baseline/speakeasy-typescript/SUMMARY.md](../baseline/speakeasy-typescript/SUMMARY.md))

856 scenarios ran through all 332 operations (279 Platform, 53 Manager): at
this commit the SDKs have every operation of the specs. 775 behave the same.

- **Requests: 331 of 332 operations agree.** The other one is the Manager's
  `stackImport`: the SDK drops keys of the free-form `importData` its schema
  does not list.
- **Responses: all 332 operations decode the same.**
- **Error bodies: 273 of 332 operations agree.** For the other 59 (44
  Manager, 15 Platform) the spec declares no error response for the status
  (404 in these scenarios), so the SDK does not read the body: it throws
  `AlienDefaultError` or `AlienManagerDefaultError` without a code. The BAML
  SDKs read Alien's error body on every operation.
- **No code where the body is not an API error.** For a text or
  foreign JSON body (400, 408, 429, 500, 502), a 200 that is not JSON, a 200
  that fails the schema, a dropped connection and a timeout, the SDK throws
  `AlienDefaultError`, `ResponseValidationError`, `ConnectionError` or
  `RequestTimeoutError`, with no code and no retryable flag. The BAML SDKs
  report `PLATFORM_HTTP_ERROR` (retryable on 408, 429 and 5xx),
  `MANAGER_HTTP_ERROR`, `MALFORMED_RESPONSE` and `COMMUNICATION_ERROR`
  (retryable).
- **Closed enums.** The SDK rejects a Manager whose `status` or `targets`
  holds a value it does not know (`ResponseValidationError`); it accepts
  unknown fields. Alien's Rust client opens these enums, and the BAML SDKs
  follow it.
- **Closed unions.** An external binding with a new `service` for a known
  `type` (`storage`/`minio`), or a new `type`, fails the whole
  `deployments.get` (`ResponseValidationError`); the BAML SDKs decode it as
  `ExternalBindingOther` (docs/api-shape.md, decision 13).
- **`null` for a nullable response** (`deployments.getUpdateOperation`): the
  SDK rejects it; the BAML SDKs return `null`.
- **Request bodies.** The SDK fills spec defaults (`maxDeployments: 100`,
  `limit=20`), writes date-times with milliseconds
  (`2026-11-01T00:00:00.000Z` for `2026-11-01T00:00:00Z`), and sends
  `content-type: application/json` on a request without a body. The BAML
  SDKs leave unset fields out, send strings as given, and send no
  `content-type` without a body.

## Findings, Progenitor ([default](../baseline/progenitor-rust-default/SUMMARY.md), [`full-api`](../baseline/progenitor-rust-full-api/SUMMARY.md))

With `full-api`, 856 scenarios ran through all 332 operations; 721 behave the
same. With default features, 325 scenarios ran through the 127 operations the
client has (74 Platform, 53 Manager), and 279 behave the same; the other 531
are skipped. Every difference below appears in both variants where the
variant has the operation.

| | Default (127 operations) | `full-api` (332) |
|---|---|---|
| Requests agree, every scenario | 100 | 234 |
| Requests agree with every argument set | 127 | 332 |
| Responses decode the same | 126 | 329 |
| Error bodies agree | 125 | 317 |

By category (`full-api` scenarios):
- **An optional request body cannot be left out (98 operations).** The
  client always sends one: left unset, it is the body type's default. When
  that type has required fields, the call fails before sending with
  `INVALID_REQUEST` ("no value supplied for name"; 79 operations); when it
  has none, the client sends `{}` (19). The BAML SDKs and Alien's TypeScript
  SDK send no body.
- **Arguments are validated.** The client checks `pattern`, `minLength`,
  `maxLength` and `format: uuid`: `getManager("not-a-manager-id")` fails
  before sending, where the BAML SDKs send it and report the server's 400.
  The spec's own ID examples are four characters short of their patterns, so
  the client refuses them; the scenarios use values that satisfy the spec.
- **Responses are validated, and enums are closed.** A `platform` of
  `future-cloud` in a Manager fails the whole response
  (`INVALID_RESPONSE_PAYLOAD`); Alien's filter opens three fields
  (`DeploymentResource.status` and two package-type lists). The BAML SDKs
  open every enum (decision 1). Unions are closed too: an external binding
  with a new `service` or `type` fails `deployments.get` ("data did not
  match any variant of untagged enum ExternalBinding"), where the BAML SDKs
  decode it as `ExternalBindingOther` (decision 13).
- **Fields only the compatibility overlay declares are dropped (2
  operations).** The client is generated from the spec without
  `sdk-compatibility-overlay.json`, so `operations.listPlugins` loses
  `installations` and `operations.publishPlugin` loses `enabled` and
  `permissionDiff`.
- **The Manager client drops keys its types do not list** (`stackImport`:
  a resource's `config`). The Platform client keeps them, as the BAML SDKs
  do under `additionalProperties`.
- **Error bodies on statuses the spec does not declare are not read (15
  Platform operations).** For 11, the 404 is not declared:
  `UNEXPECTED_RESPONSE`, where the BAML SDKs report the server's code. 4
  declare no error schema at all, so `into_sdk_error()` does not apply. The
  Manager client reads every Alien error body (53 of 53 agree, where
  Alien's TypeScript SDK reads 9).
- **Other error mapping.**
  - 408 and 429 are `UNEXPECTED_RESPONSE` and not retryable (retryable only
    at 5xx); the BAML SDKs report `PLATFORM_HTTP_ERROR`, retryable.
  - A body that is not JSON on a declared status (500 text, empty 401) and a
    malformed 200 are `INVALID_RESPONSE_PAYLOAD` with no HTTP status, where
    the BAML SDKs report `PLATFORM_HTTP_ERROR` or `MALFORMED_RESPONSE` with
    the status.
  - The Manager reads a body only when it is a whole Alien error (`code`,
    `message`, `retryable`, `internal`). The spec's `ErrorResponse` (`code`,
    `message`, `details`) is `UNEXPECTED_RESPONSE`. A Manager 5xx is
    retryable, where the BAML SDKs, like `@alienplatform/commands`, never
    retry a Manager error.
- **A base URL ending in `/` is kept**: `http://host//` gives
  `///v1/projects/…`; the BAML SDKs and Speakeasy drop the extra slashes.
- **Equivalent, not counted:** query parameters in another order, spec
  defaults filled into request bodies, and date-times written with
  milliseconds. Timeouts behave the same.

