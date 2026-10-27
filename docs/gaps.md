# Gaps

What BAML could not do that this SDK needed, each with a minimal repro, what
it means for Alien, and the workaround used here. IDs (`B1`, …) name each
gap. B15, B16 and B19 are fixed in BAML 0.23.0, the release this SDK uses,
and need no workaround; they stay listed because their sections say what
changed and what remains. B13 is BAML's design, which the SDK works with.

| ID | Gap | Workaround here | Impact |
|---|---|---|---|
| [B1](#b1) | Telemetry is on (`medium`) when the level variable is unset, and no setting changes that | None: the SDK keeps this default | — |
| [B2](#b2) | HTTP bodies are recorded verbatim, tokens included | `http_bodies = false` in `baml/baml.toml` (BAML #5389, in 0.23.0) | — |
| [B3](#b3) | Without an API key the runtime uses a saved Boundary login from `BAML_HOME` | None; users can set `BOUNDARY_API_KEY=local`. Tests isolate `BAML_HOME` | On a machine with a saved Boundary login, the SDK uploads its recordings with that login |
| [B4](#b4) | Two BAML-generated SDKs cannot share a process | None in this SDK; needs a change in BAML's runtime and generators | An application that also uses another BAML-generated SDK fails or runs the wrong function |
| [B5](#b5) | Go has no typed error values | Facade decodes the dynamic value | — |
| [B6](#b6) | No visibility: every function and type is exported | Helpers in `ns_internal`; facades re-export the public surface | — |
| [B7](#b7) | Generated names follow BAML, not the host language | Facades bind operations under host-cased names (TypeScript, Python, Go); fields, argument names, Go type names and Rust names stay as generated | Python arguments and fields, and Rust modules and functions, are camelCase (`deploymentGroup=`, `createToken_async`); Go types are `Platform<Name>` and `Manager<Name>` |
| [B9](#b9) | A project cannot depend on another BAML package | One project, `ns_*` namespaces | — |
| [B13](#b13) | An unset optional field and an explicit `null` are one value | Not changed in BAML; the SDK encodes request bodies by omission, so a request cannot send an explicit `null` | — |
| [B14](#b14) | A wire field named `test` cannot be a class field | Field `test_` plus `baml.FromJson`/`baml.ToJson` mapping it to `"test"` | — |
| [B15](#b15) | TypeScript rejects class arguments with an omitted nullable key | None: fixed in 0.23.0 (BAML #5291 and #5377) | — |
| [B16](#b16) | Telemetry is lost on exit unless the host shuts the runtime down | None: fixed in 0.23.0 (BAML #5293); every facade exports `shutdown` | — |
| [B17](#b17) | Constants are not exported to the generated SDKs | Error codes documented, not exported | — |
| [B18](#b18) | `baml test` records telemetry into the project directory by default | Scripts run the CLI with `BAML_TELEMETRY=off`; `.baml/` is gitignored | `baml test` outside the scripts, and an editor's BAML language server, leave recordings in the working tree |
| [B19](#b19) | The Rust generator skips `json` and everything that reaches it | None: fixed in 0.23.0 (BAML #5287) | — |
| [B20](#b20) | The Rust generator skips unions of string literals that are not identifiers | None needed: only internal `compat` types | — |
| [B21](#b21) | A decode failure inside a `baml.FromJson` override loses its location | None: `json_path` is absent or starts at that object; `decode_error` still says what failed | Some malformed responses report what failed without its full location |
| [B22](#b22) | The generated client classes print their credential (Rust `Debug`, Go `fmt` and `encoding/json`) | The facade types hide it; the docs say what to keep out of logs | Logging the generated client (`alien.platform()`, Go `c.Platform`) writes the API key or Manager token |
| [B23](#b23) | The Python bridge keeps each encoded call, API key included, in a local variable | None; the entry gives Sentry settings | Error reporters that record local variables (Sentry by default) store the API key |
| [B24](#b24) | `baml_bridge` turns on serde_json's `arbitrary_precision` and `preserve_order` | None | In every crate of a Rust build that links the SDK, floats inside untagged, internally tagged or flattened serde types fail to deserialize, and `Value` objects keep their input order |
| [B25](#b25) | Rust union type names join every member's name | None | `cargo doc` fails with "File name too long" |

Not in the table: B8 (BAML has no OpenAPI importer) is filled by this
repository's `tools/openapi2baml`. This SDK does not hit B10 (string-only
HTTP bodies, receiver side) or B11 (foreign native libraries).
B12 (the Go and Rust generators are alpha): both SDKs pass every scenario, and
the Rust generator's two limits found on the way are B19 (fixed) and B20.

---

## B1

**Telemetry is on when its level variable is unset.** A generated SDK records
at `medium` unless the variable named in `[bridge.env_var_names]` says
otherwise. The publisher cannot change that default: `baml.toml` has no
setting for the level itself. A project that names no variable there gets
none: its SDK always records at `medium`, and `BAML_TELEMETRY=off` does not
stop it (the [B4](#b4) repro's second project names one for that reason).

**What this SDK does.** It keeps BAML's default: telemetry is on, and
`ALIEN_TELEMETRY=off` turns it off ([telemetry.md](telemetry.md)). Until the
SDK kept HTTP bodies out of recordings ([B2](#b2)), every facade set
`ALIEN_TELEMETRY=off` when it was unset; they no longer do.

**Impact on Alien.** None as long as telemetry on by default is what Alien
wants. An SDK that wanted `off` by default would have to write the variable
into the process environment before the runtime starts, which is unsound in
Rust once other threads run.

## B2

**HTTP request and response bodies are recorded verbatim** at `medium` and
`high` unless the publisher turns them off. Header values outside a short
allowlist and every query-string value are hashed, so the API key
(`authorization`) and presigned query strings in URLs are safe. Bodies are
not.

BAML PR #5389 (in BAML 0.23.0) adds a publisher
setting that keeps every body out of network spans at every level, and this
SDK sets it in `baml/baml.toml`:

```toml
[bridge]
http_bodies = false
```

Repro: remove that line, run `scripts/generate.sh`, then
`scripts/verify.sh --phase 1`, check T3. Each language runs the full
conformance suite at `medium` and at `high` with `BOUNDARY_API_KEY=local`
(local recordings only). Without the setting, T3 fails at both levels in all
four languages: the Manager token (`bootstrapCommands` response), the
setup-link token (`createSetupLink` response) and an S3 presigned signature
(inside the `templateUrl` of a `getProjectTemplateUrls` response) are in
network spans, `network_event_values[1].payload` (the response body):

```sh
baml query --local --from .verify/telemetry/t3-high-python --format json \
  "SELECT span_name, network_event_values FROM spans WHERE span_type = 'network_span'"
```

With it, T3 finds no fixture secret at either level. The Platform API key is
not found in either case.

**Impact on Alien.** Without the setting, recordings would hold short-lived
command capabilities and setup-link tokens, and, once the SDK sends commands,
command payloads (customer data).

**What remains.** The setting covers network spans only. Function spans
record argument and result values only for LLM functions or functions with an
explicit trace policy; this SDK has neither, so its function spans hold timing
and names only.

## B3

**Without an API key, the runtime falls back to a saved Boundary login** under
`$BAML_HOME/login/cache/` for the configured endpoint (the file is named after
the SHA-256 of the endpoint URL), and otherwise writes local recordings under
`$BAML_HOME/btel/`. No `baml.toml` setting turns the fallback off for a
generated SDK.

Repro: T5 plants a fake login for the local sink endpoint in an isolated
`BAML_HOME` and runs the quickstart at `medium` with no API key. The runtime
reads it and contacts the sink (`POST /v1/recordings/<id>/heartbeat`). Without
the planted login, the same run makes no connection and writes its recording
under the isolated `BAML_HOME` only (nothing under a temporary `HOME`).

**Impact on Alien.** Telemetry is on by default, so on a machine where someone
ran `baml auth login` against Boundary's default endpoint (with the default
`BAML_HOME`, `~/.baml`), an Alien SDK uploads its recordings with that login,
to the project named by `BOUNDARY_PROJECT` if set, otherwise to the one
Boundary picks for the login. Recordings hold no HTTP bodies ([B2](#b2)).
Everyone else gets local files only.

**Workaround.** None in the SDK. A user who does not want this sets
`ALIEN_TELEMETRY=off` or `BOUNDARY_API_KEY=local`. Every test and script sets
`BAML_HOME`.

## B4

**Two BAML-generated SDKs cannot share a process.** The runtime is
process-global: initializing a second generated SDK replaces the first one's
program. Every generated package is also named `baml_sdk`.

Repro: `examples/coexistence/run.sh` (after `scripts/generate.sh`). It installs
the Alien SDK and a second BAML project's SDK (`examples/coexistence/other/`,
one function `greet` plus a function whose name happens to match one of
Alien's) the way an application would, calls both, and asserts:

| Setup | Alien SDK call | Other SDK call |
|---|---|---|
| TypeScript, both installed from tarballs (one shared `@boundaryml/baml-bridge`), Alien imported first | **Silently runs the other project's function** with the same name | works |
| Same, other SDK imported first | works | `BamlPanic: Function not found: user.greet` |
| TypeScript, each SDK resolves its own copy of the bridge (`link:` install) | works | works |
| Python, other SDK installed last (it overwrites `baml_sdk/`) | **Silently runs the other project's function** | works |
| Python, Alien SDK installed last | works | `AttributeError: module 'baml_sdk' has no attribute 'greet'` |
| Python, other SDK vendored as `other_baml_sdk` | **Silently runs the other project's function** | works |

So the last SDK to initialize owns the process. Calls through the other SDK
either fail with a panic or, worse, run a different function with the same
fully qualified name and return its result.

**Impact on Alien.** An application that uses the Alien SDK and any other
BAML-generated SDK would break or silently misbehave.

**Status.** Not fixed in this SDK; it needs a change in BAML's runtime and
generators. No workaround is built here. One configuration happens to work
and is checked by `run.sh`: when each SDK resolves a physically separate copy
of `@boundaryml/baml-bridge`, Node loads two native addons and each SDK gets
its own runtime. Package managers deduplicate the bridge by default, so
applications do not get this on their own. In Python, renaming the package
fixes the import clash but the runtime is still shared.

## B5

**Go has no typed error values.** A thrown `AlienError` reaches Go as
`*baml_go.BamlError{Kind: "error", ClassName: "user.errors.AlienError", Value: map[string]any{…}}`.
TypeScript, Python and Rust give the generated class.

Repro: any Go scenario expecting an error, e.g.
`go test ./alien -run 'TestConformance/platform.getManager.rate_limited_429'` with
the conversion in `alien/errors.go` removed.

**Impact on Alien.** Without a facade, Go users would type-assert maps.

**Workaround.** `alien/errors.go` re-encodes `Value` through `encoding/json` into
`*alien.AlienError`, so callers use `errors.As`.

## B6

**No visibility modifiers.** Every BAML function and type is exported by every
generated SDK, including helpers (`internal.percent_encode`,
`internal.send`, …) and test support (`compat.*`, the setup test
helpers in `internal`).

**Impact on Alien.** The generated packages expose internals that Alien would
then have to keep stable or document as private.

**Workaround.** Helpers live in `ns_internal` (test-only code in `ns_compat`),
and the facades re-export only the public surface. The generated `baml_sdk`
modules still contain everything. In Go, users call the facade's methods but
import `baml_sdk` for request and response types; the `alien` package
documentation says its `Internal*` and `Compat*` functions are not part of the
SDK.

## B7

**Generated names follow BAML, not the host language.** `naming_convention` is
fixed per language (`language` for Go, `preserve-case` elsewhere). Examples:

| BAML | TypeScript | Python | Go | Rust |
|---|---|---|---|---|
| `platform.managers.get` | `platform.managers.get` | `platform.managers.get` | `PlatformManagersGet` | `platform::managers::get_async` |
| `setup.cloudformation_launch_url` | `setup.cloudformation_launch_url` | `setup.cloudformation_launch_url` | `SetupCloudformationLaunchUrl` | `setup::cloudformation_launch_url_async` |
| `platform.PlatformClient` | `platform.PlatformClient` | `platform.PlatformClient` | `PlatformPlatformClient` | `platform::PlatformClient` |
| field `templateUrl` | `templateUrl` | `templateUrl` | `TemplateUrl` | `templateUrl` |

**Workaround.** The TypeScript and Python facades bind every generated
operation under host-cased names, without per-operation code
(`alien.deploymentGroups.createToken`,
`alien.deployment_groups.create_token`). Argument and field names use Alien's
TypeScript names (`custom_domains` → `customDomains`, `command_id` →
`commandId`, [api-shape.md](api-shape.md#rules)): Python keyword
arguments and fields are camelCase (`deployments.list(deploymentGroup=…)`).
The Go facade has a generated method per operation
(`c.DeploymentGroups.CreateToken`), written by `sdk/go/internal/genops` from
the generated functions; Go type names stay as generated. Rust calls the generated functions directly; the Rust generator
requires `preserve-case`, so Rust modules and functions are camelCase
(`platform::deploymentGroups::createToken_async`).

## B9

**A project cannot depend on another BAML package** (`[dependencies]` is an
error). Not a problem for this SDK: everything lives in one project under
`ns_errors`, `ns_platform`, `ns_manager`, `ns_setup`, `ns_internal` and
`ns_compat`.

## B13

**An unset optional field and an explicit `null` are one value.** BAML gives
an optional field a single unset state, `null`: decoding `{}` and
`{"x": null}` gives the same value, and BAML's default JSON encoding writes it
as `null`. This stays as it is (BAML does not add a third state), so this
entry records how the SDK works with it rather than a fix to wait for.

**Decoding.** What Alien's regression tests require holds: absent and empty
decode differently, `null` decodes like an absent field and re-encodes as one,
malformed arrays are rejected (`stored_secret_presence` in
`baml/baml_src/ns_compat/`, and the `compat.stored_secret_presence.*`
scenarios).

**Encoding.** The SDK's request classes own their wire encoding. The importer
gives every class reachable from a request body that has optional fields a
`baml.ToJson` that leaves unset optional fields out, as both of Alien's SDKs
do: Speakeasy omits `undefined`, and Progenitor skips `None`. Values inside
`json` fields are written as given, `null`s included.

**Impact on Alien.** A request cannot send an explicit `null` for an optional
field, as with Alien's Rust SDK. Nothing in the SDK handles the fields below
specially; how it could depends on Alien's answer to
[question 6](#questions-for-alien).
Today each of them is left out of the request when it is `null` or unset, in
all four languages, and TypeScript accepts `{ domainId: null }` without an
error.

Optional request fields whose spec text gives `null` a meaning (Platform; the
Manager reads an absent field as `null` everywhere, so its fields are not
listed):

| Operation | Field | Spec text |
|---|---|---|
| `pinDeploymentRelease` (POST) | `releaseId` | "Release ID to pin the deployment to. Set to null to resume its followed channel." The SDK sends `{}`, which is what Alien's CLI sends to unpin (`PinReleaseRequest { release_id: None }`) |
| `updateProject` (PATCH) | `rootDirectory` | "The name of a directory or relative path to the source code of your project. When null is used it will default to the project root" |
| `updateProject` (PATCH) | `domainId` | "Selected domain for this project (null = default system domain)" |
| `updateProject` (PATCH) | `packagesConfig.cli`, `.cloudformation`, `.operatorImage`, `.helm`, `.terraform` | "CLI package configuration. If null, CLI packages will not be generated." (the same sentence for each package type; `operatorImage` adds "Required when Helm is enabled.") |
| `configureProjectSource` (POST) | `rootDirectory` | As `updateProject.rootDirectory` |
| `createProject`, `createProjectFromTemplate` (POST) | `rootDirectory` | As `updateProject.rootDirectory` |
| `createProject`, `createProjectFromTemplate` (POST) | `packagesConfig.cli`, `.cloudformation`, `.operatorImage`, `.helm`, `.terraform` | As in `updateProject`: "If null, … packages will not be generated." Whether leaving the key out generates them is not said |
| `updateDeploymentEnvironmentVariables` (PATCH) | `variables[].targetResources` | "Target resource patterns (null = all resources)" |
| `updateAPIKey` (PATCH) | `deploymentSetupConfig.returnUrl`, `deploymentSetupConfig.refreshUrl` | "Null clears the destination; omit to keep it unchanged." The SDK cannot clear them: it can only leave them unchanged |
| `createSetupLink`, `createDeploymentGroupToken` (POST) | `returnUrl`, `refreshUrl` | As in `updateAPIKey` |

One more mentions `null` without needing it: `createCommand.idempotencyKey`
("use null for a command without arguments", about `params`). Required nullable fields are always sent, `null` included,
for example `environmentVariables[].targetResources` ("null = all resources")
in `createDeployment`, `createDeploymentGroupToken` and `createSetupLink`.

Optional nullable fields of PATCH and PUT bodies whose spec text does not say
what `null` does; by the usual PATCH reading, `null` clears them:

| Operation | Fields |
|---|---|
| `updateProject` (PATCH) | `gitRepository`, `deploymentPortalAppearance`, `packagesConfig`, `defaultManagers`; inside them `deploymentPortalAppearance.avatarUrl`, `.faviconUrl`, `.title`, `.subtitle`, `.supportUrl`, `.docsUrl`, `defaultManagers.aws`, `.gcp`, `.azure`, `.kubernetes`, `.machines`, `.local`, `packagesConfig.cloudformation.displayName`, `.terraform.displayName`, `.operatorImage.brand`, `.envPrefix`, `.labelDomain`; and `packagesConfigPatch.cli`, `.cloudformation` (and its `displayName`), `.terraform` (and its `displayName`), `.operatorImage` (and its `brand`, `envPrefix`, `labelDomain`), `.helm` (and its `logCollector`, `setupResources`, `runtimePersistence`) |
| `updateAPIKey` (PATCH) | `description`, `expiresAt` ("Optional expiration date for the API key"); and 15 fields inside `deploymentSetupConfig.policy.stackSettings.defaults` (`compute`, `domains`, `externalBindings`, `kubernetes`, `network`, `publicEndpoints` and their nullable children) |
| `updateWorkspace` (PATCH) | `logoUrl` |
| `updateManagerDomainBinding` (PUT, "Create, update, or remove") | `domainId` ("Unique identifier for the domain.") |
| `updateDeploymentCompute` (PATCH) | `compute` |
| `updateCommand` (PATCH, runtime-facing) | `dispatchedAt`, `completedAt`, `responseSizeBytes`, `error` |
| `updateDebugSession` (PATCH, runtime-facing) | `error` |

## B14

**A wire field whose name is a BAML reserved word cannot be a class field
under that name.** `test` is reserved:

```baml
class StackByPlatform {
    aws: json?,
    test: json?,      // error[E0010]: unexpected token: expected `'}'`, found `test`
}
```

Since BAML 0.23.0 (BAML #5399), `@alias` can set a field's JSON key. The
alias is the name a model is shown for the field, and `baml.json` uses it as
the key when called with `naming = baml.json.Naming.Aliased` (the default,
`Naming.Declared`, ignores it):

```baml
class StackByPlatform2 {
    aws: json?,
    platformTest: json? @alias("test"),
}
// baml.json.decode<StackByPlatform2>(`{"test": {"a": 1}}`)                                     platformTest = null
// baml.json.decode<StackByPlatform2>(`{"test": {"a": 1}}`, naming = baml.json.Naming.Aliased)  platformTest = {"a": 1}
// baml.json.stringify(s, naming = baml.json.Naming.Aliased)                                     {"aws":null,"test":{"a":1}}
```

**Impact on Alien.** `StackByPlatform` has a `test` property in both the
Platform and Manager specs (the only collision with a BAML keyword in either).

**Rule.** A wire field whose name is a reserved word gets a trailing
underscore (`test` → `test_`), and the class implements `baml.FromJson` and
`baml.ToJson` to map `test_` to and from `"test"`. This SDK does not use
`@alias` for wire names. The same mapping gives a snake_case wire name its TypeScript SDK
name (`custom_domains` ↔ `customDomains`). The importer applies the rule and
lists every renamed field in `docs/importer-coverage.md`. The `stack_by_platform` tests in `ns_compat` run on
the imported `platform.StackByPlatform`: decode, encode, round trips (unset
platforms omitted, `null`s inside a stack kept) and non-object input. The
`field_names` tests round-trip camelCased fields (`vpc_id`, `custom_domains`,
`on-other`) in both APIs.

Decoding goes through `baml.json.decode`, and encoding through
`baml.json.stringify`; both honor the overrides.

## B15

**The TypeScript bridge rejected a class argument whose nullable key was
missing.** Fixed in BAML 0.23.0 by PRs #5291 and #5377: an omitted nullable
field reads as `null`, and the generated TypeScript classes declare nullable
fields optional (`releaseId?: string | null`). So a request body leaves unset
keys out:

```ts
await alien.deployments.pinRelease(id, { body: {} })
```

Values the SDK returns type a nullable field `T | null | undefined`, as
Alien's TypeScript SDK does; at run time it holds `null`.

**What remains.** Rust's generated structs still need every field, with
`None` for unset ones (no `Default`: a blanket one would fill required fields
with `""` and `0`).

## B16

**Telemetry is lost when a process exits without shutting the runtime down.**
The runtime flushes recordings about once a second and seals them at
shutdown. BAML PR #5293, in BAML 0.23.0, adds
`baml_bridge::runtime::shutdown()` and an automatic shutdown at normal exit to
the Rust bridge, and exports `shutdownRuntime` from the TypeScript bridge's
package root. BAML leaves the moment to shut down to the host program, so each
facade exports its own call, the one Alien's users see:

| Language | Shuts down by itself | Facade call |
|---|---|---|
| TypeScript | Node's `beforeExit` only; not on `process.exit()`, signals, or test runners that end workers | `await shutdown()` |
| Python | `atexit`; not on `os._exit()` or SIGTERM | `alien_sdk.shutdown()` |
| Go | Never | `alien.Shutdown()` |
| Rust | Normal exit, waiting with no time limit; not on signals or `abort` | `alien_sdk::shutdown()` |

When to call it is documented in the README quickstart, the client
constructors and [runtime.md](runtime.md#shutting-down). T3 requires a sealed
recording in all four languages: the TypeScript and Go suites call the
facade's shutdown, the Python and Rust suites rely on the automatic one.

**Impact on Alien.** Short-lived processes that end early (`process.exit()`, a
signal, an abort) lose their last second of telemetry unless they call
`shutdown`. Go programs with telemetry on always need it.

## B17

**Constants are not exported to the generated SDKs.** Since BAML 0.23.0 a
project can declare top-level constants and use them in its functions, but
none of the four generators exports them:

```baml
const RATE_LIMITED: string = "RATE_LIMITED"

function code() -> string {
    RATE_LIMITED
}
// baml generate: `code` is in the TypeScript, Python, Go and Rust SDKs; `RATE_LIMITED` is in none of them
```

So the error codes in `ns_errors` cannot reach the SDKs as named constants.

A constant can also hold a compiled regex
(`const REGION: baml.regex.Regex = baml.regex.new("…") catch (e) { … }`). The
setup helper does not keep one: it compiles its region pattern on every call,
part of the helper's cost in [metrics](../metrics/results.md).

**Workaround.** The codes are documented with `AlienError`
([docs/errors.md](errors.md)) and in its doc comment.

## B18

**`baml test` records telemetry into the project directory by default.** The
CLI's own runtime writes local recordings to `<project>/.baml/btel/`, including
the process command line (which can contain local paths). It reads
`BAML_TELEMETRY`, not the variable `[bridge.env_var_names]` renames: that
setting applies only to generated SDKs.

```sh
baml --project baml test                       # creates baml/.baml/btel/recordings/…
ALIEN_TELEMETRY=off baml --project baml test   # still creates them
BAML_TELEMETRY=off baml --project baml test    # creates nothing
```

**Impact on Alien.** Alien's engineers running `baml test` would leave
recordings in their working tree, one `git add` away from a commit.

**Workaround.** `scripts/lib.sh` runs every CLI command with
`BAML_TELEMETRY=off`, `verify.sh` fails if `baml/.baml` appears, `.baml/` is
gitignored, and the secret scan searches binary files too. An editor's BAML
language server (`baml-cli lsp`) also records, into the `.baml/btel/` of each
BAML project it opens: here `baml/.baml/` and
`examples/coexistence/other/baml/.baml/`. Both are gitignored; `verify.sh`
fails on the first, so delete `baml/.baml/` before running it in a checkout
open in an editor.

## B19

**The Rust generator skipped `json`, and every type and function that reaches
it.** `baml.json.json` is a recursive type alias, and the Rust generator
skipped recursive aliases, while TypeScript, Python and Go map `json` to
`unknown`, `Any` and `any`. Alien's specs use free-form JSON (`{}` schemas and
untyped `additionalProperties`) for policies, metadata, error contexts and
command payloads, so Rust lost 29 of the 78 operations the SDK first imported
(`tools/openapi2baml/stage-a.json`; 129 of 615 types) and 91 of the 314 then
in the specs, among them `platform.setupLinks.create`.

Fixed in BAML 0.23.0 by PR #5287: the Rust
generator maps `json` to `baml_bridge::Json` (`serde_json::Value`), as
Progenitor does. The Rust SDK now has every imported operation and passes
every scenario, and `scripts/generate.sh` still fails on any skipped item of
this project except the internal `compat` role types ([B20](#b20)).

One limit remains, set by `serde_json::Value` itself. BAML's `int` is 63-bit,
so a larger JSON integer becomes a BAML `bigint`; `baml_bridge::Json` carries
it as long as it fits `i64` or `u64`, in both directions. A response `json`
value holding an integer outside the `i64`/`u64` range fails the whole call
with `alien_sdk::Error::Runtime` ("baml.json.json value has no serde_json
form: an integer outside the i64/u64 range"). Requests cannot hit it: every
`serde_json` integer is an `i64` or a `u64`. The other languages map `json` to
`unknown`, `Any` and `any`; no scenario covers such a number.

## B20

**The Rust generator skips unions of string literals that are not Rust
identifiers** (`unsupported union arm: "deployment.viewer"`), as it derives an
enum variant name from each literal.

```baml
type Role = "deployment.viewer" | "deployment.admin"
```

**Impact on Alien.** Only the internal `compat` role types, which exist to
characterize Alien's role strings and are exchanged as JSON text. The importer
keeps enums open (`string`), so imported types are not affected.

**Workaround.** None needed. `scripts/generate.sh` tolerates skipped `compat`
types and nothing else.

## B21

**A decode failure inside a `baml.FromJson` override loses its location.**
`baml.json.DecodeError.path` says where in a JSON value decoding failed
(`.items[1].id`), and `MALFORMED_RESPONSE` reports it as `context.json_path`
([errors.md](errors.md#codes)). When a class provides `from_json`, BAML runs
it with the value at that position, but a `DecodeError` the override throws
reaches the caller with the override's own `path`: the position the walk had
reached (`.items[1]`) is not prepended.

```baml
class Q {
    id: string,
    implements baml.FromJson {
        function from_json(j: baml.json.json) -> Self throws baml.json.DecodeError {
            match (j) {
                let o: map<string, baml.json.json> => Q { id: baml.json.to<string>(baml.json.field(o, "id")) },
                _ => throw baml.json.DecodeError { message: "expected object", path: "" },
            }
        }
    }
}
class Page { items: Q[] }
// baml.json.decode<Page>("{\"items\":[{\"id\":\"a\"},{\"id\":3}]}")
//   DecodeError { path: "", message: "expected string" }    expected path: ".items[1].id"
// The same class without the override:
//   DecodeError { path: ".items[1].id", message: "expected string" }
```

The SDK's imported types provide `from_json` for 116 classes (71 Platform, 45
Manager: those that hold an open union, or have renamed or extra keys; 15 of
them because snake_case wire names are camelCased, such as `BillingFeatureFlags`
and the stack settings networks), and the responses of 55 of the 332
operations contain one, among them `billing.getEntitlements`,
`deployments.get`, `deployments.list`, `events.list` and the Manager's
`commands.createCommand` and `commands.getCommandStatus`. The test "B21: …" in
`baml/baml_src/ns_internal/http_tests.baml` records today's behavior.

**Impact on Alien.** Inside those classes, a response that does not decode
says what failed (`decode_error`) but not where: `json_path` is absent, or
names a field relative to the class.

**Workaround.** None. A fix needs two parts: BAML prepends the walk's position
to a `DecodeError` thrown by a provided `from_json`, and the importer's
generated `from_json` bodies prepend each field's name (`.id`) to the errors
of their own `baml.json.to` calls.

## B22

**The generated client classes print their credential.** BAML has no way to
mark a field secret, so the generated `PlatformClient` and `ManagerClient`
format like every other generated class: Rust derives `Debug`, and Go structs
show every field to `fmt` and `encoding/json`. Their `api_key` and `token`
fields hold the credential.

```rust
let alien = Alien::new(Options { api_key: Some("alien_example_key".into()), ..Default::default() })?;
println!("{:?}", alien);            // Alien { server_url: "https://api.alien.dev", api_key: "<redacted>", workspace: None, timeout_ms: None }
println!("{:?}", alien.platform()); // PlatformClient { base_url: "https://api.alien.dev", api_key: "alien_example_key", workspace: None, timeout_ms: None }
```

```go
c, _ := alien.New(alien.Options{APIKey: "alien_example_key"})
fmt.Printf("%+v\n", c)          // alien.Client{APIKey:<redacted> ServerURL:"https://api.alien.dev" Workspace:""}
fmt.Printf("%+v\n", c.Platform) // {BaseUrl:https://api.alien.dev ApiKey:alien_example_key Workspace:<nil> TimeoutMs:<nil>}
```

| Language | Shows the API key or token | Hides it |
|---|---|---|
| Rust | `Debug` of `alien.platform()` and `AlienManager::client()`: `{:?}`, `dbg!`, a `#[derive(Debug)]` struct holding one, a `#[tracing::instrument]` argument | `Debug` of `Alien` and `AlienManager` |
| Go | `%v`, `%+v`, `%#v` of `c.Platform`; `json.Marshal` of `c`, `c.Platform`, `m` and `m.Generated`; `slog` with `c.Platform` (text and JSON handlers), or with `m` or `m.Generated` (JSON handler) | `fmt` and `slog` of `c`; `fmt` of `m` and `m.Generated` print the token's pointer |
| Python | Tracebacks that record local variables ([B23](#b23)) | `repr()` of `Alien` and `AlienManager` |
| TypeScript | Nothing found | `util.inspect`, `JSON.stringify` and `String()` of `Alien` and `AlienManager` |

**Impact on Alien.** Every Rust call takes `alien.platform()` or
`AlienManager::client()`, so the generated client is what application structs
and function arguments hold. `#[tracing::instrument]` records each argument
with `Debug`, a struct that holds the client included. A log line, a `dbg!`
or a span then carries the API key or the Manager token.

**Workaround.** The facade types hide the credential, and the rustdoc of
`Alien::platform` and `AlienManager::client` and the godoc of `alien.Client`
say that the generated client shows it. Keep the generated clients out of
logged values: `#[tracing::instrument(skip(client))]`, a hand-written `Debug`
for a struct that holds one, and in Go log `c`, not `c.Platform`, and do not
`json.Marshal` a client.

## B23

**A Python traceback that records local variables holds the API key.**
`baml_bridge` encodes each call's arguments, the client included, into a bytes
local named `args_proto` in the function that runs the call (`_sync`, and
`_async` for `AsyncAlien`). When the call fails, `AlienError` is chained from
the bridge's error, whose traceback holds that frame. The facade's clients
keep the key out of `repr()`, which does not help here, and the name
`args_proto` does not match scrubbers that look for `api_key` or `token`.

```python
alien = alien_sdk.Alien("alien_example_key", server_url="http://127.0.0.1:9")  # nothing listens there
try:
    alien.managers.get("mgr_1")
except alien_sdk.AlienError as e:
    print("".join(traceback.TracebackException.from_exception(e, capture_locals=True).format()))
# …
#     args_proto = b'\n\x8c\x01\n\x06client2\x…\x12\x1e\n\x07api_key2\x13\x12\x11alien_example_key\x12\x0b\n\tw…'
```

| Records local variables | Shows the key |
|---|---|
| Sentry's Python SDK (2.73.0), default settings | Yes, in the `_sync` frame's `args_proto` |
| `rich` tracebacks with `show_locals=True` | Yes |
| `pytest -l` | Shortened in the middle, so the key may be cut out; in full with `-vv` |
| `traceback` with `capture_locals=True` | Yes |

**Impact on Alien.** A Python service that reports errors to Sentry with its
defaults sends the API key with every failed SDK call.

**Workaround.** None in the SDK. For Sentry, turn local variables off
(`include_local_variables=False`), or keep them and add `args_proto` to the
scrubber's denylist
(`event_scrubber=EventScrubber(denylist=DEFAULT_DENYLIST + ["args_proto"])`).
With `data_collection` set, Sentry ignores `event_scrubber`; use
`data_collection={"stack_frame_variables": False}` there. With sentry-sdk
2.73.0, the key is in the event with the defaults and in none of the three
settings. [errors.md](errors.md#credentials-in-python-tracebacks) has the
code. A fix needs the Python bridge not to keep the encoded call in a local.

## B24

**Adding the Rust SDK turns on two serde_json features for the whole build.**
`baml_bridge` 0.23.0 depends on `serde_json` with
`features = ["arbitrary_precision", "preserve_order"]`. Cargo unifies a
crate's features across a build, so every crate in a binary that links
`alien_sdk` gets a `serde_json` with both, including code that has nothing to
do with the SDK. `cargo tree -e features -i serde_json` shows where they come
from.

Repro: one crate, built with and without `alien_sdk` as a dependency it never
uses (serde 1.0.229, serde_json 1.0.154):

| Code in the crate | Without `alien_sdk` | With it |
|---|---|---|
| `#[serde(untagged)] enum R { V(f64), S(String) }` from `1.5` | `V(1.5)` | `data did not match any variant of untagged enum R` |
| A `#[serde(flatten)]` field whose struct has `ratio: f64`, from `{"name":"a","ratio":0.5}` | decodes | `invalid type: map, expected f64` |
| A `#[serde(tag = "type")]` enum variant with `cpu: f64`, from `{"type":"Scaled","cpu":0.5}` | decodes | `invalid type: map, expected f64` |
| The same three with integers instead (`5`, `3`, `2`) | decodes | decodes |
| A `Value` parsed from `{"b":1,"a":2}`, printed | `{"a":2,"b":1}` | `{"b":1,"a":2}` |
| A `Value` parsed from `1.0` == one parsed from `1.00` | `true` | `false` |
| `from_str::<Value>("1e400")` | `number out of range` | `1e+400` |

With `arbitrary_precision`, serde_json keeps each number's text. A number with
a fraction or an exponent then fails to decode into a float wherever serde
buffers the input first (untagged and internally tagged enums, `flatten`), and
numbers compare by their text. With `preserve_order`, `Value` objects keep
their keys in input order instead of sorting them.

**Impact on Alien.** A binary that adds the SDK, such as Alien's CLI, gets
both features in all its crates. Nothing fails to compile: code that decodes
floats through those serde attributes fails at run time, and output or tests
that rely on sorted `Value` keys change.

**Workaround.** None: the features come from `baml_bridge`'s own manifest, and
a crate that depends on it cannot turn them off.

## B25

**`cargo doc` fails on the Rust SDK.** The Rust generator names a union type
by joining its members' names with `Or`. In this SDK 33 generated enum names
are longer than 200 characters; the longest, the union of event types, has
1,584. rustdoc writes a file named after each type, and most file systems
allow 255 bytes in a file name:

```
$ cargo doc --no-deps                   # in sdk/rust; cargo doc --no-deps -p baml_sdk fails the same way
error: couldn't generate documentation: File name too long (os error 63)
  = note: failed to create or modify ".../target/doc/type.impl/alien_sdk/platform/enum.AgentSessionStatusEventOrAgentSessionStepEventOr…OrAgentSessionEventOther.js": File name too long (os error 63)
```

**Impact on Alien.** No local documentation for the Rust crates, and a
published crate would get no docs.rs page.

**Workaround.** None. Read the generated sources (`sdk/rust/baml_sdk/src/`) or
the BAML types (`baml/baml_src/ns_platform/types.baml`) instead. A fix needs
the generator to name a union after where it is used, not after its members.
Such names would also stop changing when a member is added
(the importer's [DESIGN.md](../tools/openapi2baml/DESIGN.md) says so for open
unions).

---

## Questions for Alien

The setup helper builds the AWS quick-create link format. These are not
blockers:

1. Which stack parameters do the setup stacks of Alien's customers take, and
   do any need values that are not plain strings (lists, `NoEcho` secrets)?
2. Which regions do Alien's customers deploy to? The helper accepts any region
   shaped like the commercial AWS partition's (`^[a-z]{2}-[a-z]+-[0-9]+$`), so new
   regions work, and rejects China (`cn-*`), GovCloud and other partitions, whose
   consoles use different hosts. Should it support any of those?
3. Should the link open the "review" page (`#/stacks/create/review`, used here)
   or the "quick create" page (`#/stacks/quickcreate`)? Which one does the
   server's `launchStackUrl` use?
4. Should the link pre-acknowledge capabilities such as `CAPABILITY_IAM`, or
   set a `ChangeSetName`?

From the importer:

5. The Manager spec declares `bearer` security on 36 of its 53 operations, not
   on `create_command` or `get_command_status`, but `@alienplatform/commands`
   sends the token on every Manager request. This SDK does the same. Does the
   Manager require the token on every operation, and should the spec say so?
6. Several request fields give `null` a meaning ([B13](#b13) lists them with
   the spec text), for example `updateProject.rootDirectory` ("When null is
   used it will default to the project root") and `updateProject.domainId`
   ("null = default system domain"). This SDK leaves unset fields out of
   request bodies and cannot send an explicit `null`, like Alien's Rust SDK.
   (a) In the Platform's PATCH and PUT bodies, does a missing field keep its
   value and `null` clear or reset it, for every field B13 lists? (b) Inside
   objects such as `packagesConfig` and `packagesConfigPatch`, does `null`
   differ from a missing key? (c) In `createProject`, does leaving out
   `packagesConfig.cli` generate CLI packages while `null` does not? (To
   resume a deployment's followed channel, the SDK sends `pinRelease` an empty
   body, as Alien's CLI does.) The setup links' `returnUrl` and `refreshUrl`
   now say it outright ("Null clears the destination; omit to keep it
   unchanged"), so `updateAPIKey` cannot clear them through this SDK.

From the specs and Alien's clients:

7. The Platform spec of Alien v3.3.43, the one vendored here, added
   `maxDeployments` and `currentCount` to `DeploymentInfo.deploymentGroup` as
   required fields; the spec at Alien `9ab099c3f` did not have them. An SDK
   generated from the newer spec fails `deployment.getInfo` against a server
   that leaves them out (`MALFORMED_RESPONSE: … at .deploymentGroup: missing
   required field maxDeployments`). Is the server always deployed before
   clients built from a newer spec are released, or should a field the server
   has just started sending stay optional in the spec for a while?
8. `@alienplatform/commands` gets a Manager URL and token from the Platform API
   (`commands.bootstrap`) and sends that token to the Manager. Alien's CLI
   instead sends the bearer token it uses for the Platform API, plus an
   `x-alien-workspace` header, to the Manager (`connect_manager_by_id` and
   `resolve_manager_inner` in `crates/alien-cli/src/execution_context.rs`,
   v3.3.43). `AlienManager` here sends whichever token it is given, and no
   workspace header; the Manager spec declares only `bearer`. Which
   credentials does the Manager accept, which should SDK users pass, and
   should the spec describe the workspace header?
