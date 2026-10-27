# Alien SDK on BAML (reference implementation)

Built by Boundary as a reference for Alien; not an official Alien SDK.

One BAML project, four SDKs. The logic in [`baml/`](baml/) is written once in
BAML; `baml generate` turns it into a TypeScript, Python, Go and Rust package.
Each package runs the same BAML code inside an embedded BAML runtime, so the
four SDKs behave identically by construction. A thin hand-written facade per
language adds client construction, idiomatic names and error conversion, and
nothing else.

This is a reference repository, not a release: nothing here is published, and
package names (`@alien-baml/sdk`, `alien_sdk`, `example.com/alien-baml-sdk/go`)
are placeholders.

**It covers:**

- Platform and Manager API operations imported from Alien's OpenAPI specs
  ([`specs/`](specs/)) by [`tools/openapi2baml`](tools/openapi2baml/DESIGN.md):
  all 332: 279 Platform (every one Alien's TypeScript SDK would generate) and
  53 Manager operations, in the shape [docs/api-shape.md](docs/api-shape.md)
  describes;
- a setup-link helper that builds a customized AWS CloudFormation quick-create
  link from a server-provided template URL;
- the decode rules that the regression tests of Alien's Platform Rust client
  check (unknown fields and enum values, optional success bodies,
  `storedSecretInputIds` presence, per-platform stacks, event states, roles),
  ported to [`baml/baml_src/ns_compat/`](baml/baml_src/ns_compat/) and run in
  all four languages;
- a repro of two BAML-generated SDKs in one process ([`examples/coexistence/`](examples/coexistence/)).

**Not covered:**

- Alien's commands client (`@alienplatform/commands`, with `CommandsClient`
  and `invoke`, and the Rust `alien-commands-client`). The SDK has the
  operations it calls (`commands.bootstrap` on the Platform API,
  `commands.createCommand` and `commands.getCommandStatus` on the Manager), not
  the `invoke` flow built on them;
- Alien's bindings (`alien-bindings` and its Node and Python adapters).
  [docs/bindings/](docs/bindings/) compares the two adapters and the ways to
  produce them;
- publishing and release: no package is published, and the package names are
  placeholders.

## Layout

```
specs/                   Alien's OpenAPI specs, vendored (see specs/README.md)
tools/openapi2baml/      the OpenAPI → BAML importer (scripts/import.sh)
baml/                    the BAML project: baml.toml + baml_src/ns_*/ (ns_platform, ns_manager imported)
sdk/typescript/          baml_sdk/ (generated)  src/ (facade)        test/
sdk/python/              baml_sdk/ (generated)  alien_sdk/ (facade)  tests/
sdk/go/                  baml_sdk/ (generated)  alien/ (facade + tests)
sdk/rust/                baml_sdk/ (generated)  src/ (facade crate)  tests/
conformance/             language-neutral scenarios, stub server, fixtures
docs/                    API shape, errors, gaps, importer coverage, telemetry;
                         bindings/ compares Alien's Node and Python bindings adapters
bench/                   benchmark programs (ours and Alien's current SDKs); bench/run.sh measures
metrics/                 size, startup, per-call, large-page and Rust build measurements (written by bench/run.sh)
scripts/                 import.sh, generate.sh, verify.sh
```

Generated `baml_sdk/` directories are not checked in. Never edit them, nor
the imported `baml/baml_src/ns_platform/` and `ns_manager/`: change the
importer or its config and run `scripts/import.sh`, or change the hand-written
namespaces, then run `scripts/generate.sh`.

## Requirements

Tested on macOS arm64 with Node 24.22, pnpm 12.14, Python 3.14, uv 0.13,
Go 1.27 and Rust 1.96.

- `baml` on PATH: the BAML wrapper (`brew install boundaryml/baml/baml`). The
  scripts install the toolchain they need, BAML 0.23.0, into `.baml_home/cli`
- Node and pnpm
- Python 3.10+ and uv
- Go 1.24+ with cgo (`CGO_ENABLED=1`, the default on macOS); `baml generate` runs `gofmt`
- Rust 1.91+

The SDKs use the published BAML 0.23.0 runtime packages, pinned exactly: npm
`@boundaryml/baml-bridge`, PyPI `baml-bridge`, Go `github.com/boundaryml/baml-go`,
crates.io `baml_bridge`. No BAML checkout is needed. The scripts keep all BAML
state in the repository (`.baml_home/`, `.cache/`), never in `~/.baml`.

## Quickstart

```sh
git clone <this repository> alien-baml-sdk && cd alien-baml-sdk
scripts/generate.sh
```

`generate.sh` regenerates the four SDKs and builds the facades. Each example
below builds a CloudFormation quick-create link, calls no API and prints:

```
https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1#/stacks/create/review?templateURL=https%3A%2F%2Fexample.com%2Fsetup.yaml&stackName=alien-setup&param_ExternalId=customer-0001
```

**TypeScript.** Save as `sdk/typescript/quickstart.mjs`, then run `cd sdk/typescript && node quickstart.mjs`.

<!-- quickstart typescript sdk/typescript/quickstart.mjs :: cd sdk/typescript && node quickstart.mjs -->
```js
import { cloudFormationLaunchUrl } from "@alien-baml/sdk"

console.log(cloudFormationLaunchUrl({
  templateUrl: "https://example.com/setup.yaml", region: "us-east-1",
  stackName: "alien-setup", parameters: { ExternalId: "customer-0001" },
}))
```

**Python.** Save as `sdk/python/quickstart.py`, then run `cd sdk/python && uv run python quickstart.py`.

<!-- quickstart python sdk/python/quickstart.py :: cd sdk/python && uv run python quickstart.py -->
```python
import alien_sdk

print(alien_sdk.cloudformation_launch_url(alien_sdk.CloudFormationLaunch(
    templateUrl="https://example.com/setup.yaml", region="us-east-1",
    stackName="alien-setup", parameters={"ExternalId": "customer-0001"})))
```

**Go.** Save as `sdk/go/cmd/quickstart/main.go`, then run
`cd sdk/go && go run ./cmd/quickstart`.
Go and Rust load BAML's runtime library at run time. The first program to run
downloads it (about 34 MB, checked against the release's SHA-256) into
`~/.baml/bridges/0.23.0/` (`$BAML_HOME` when set); every later run uses that
copy. [docs/runtime.md](docs/runtime.md) has the other ways to provide it.

<!-- quickstart go sdk/go/cmd/quickstart/main.go :: cd sdk/go && go run ./cmd/quickstart -->
```go
package main

import (
	"context"
	"fmt"
	"log"

	"example.com/alien-baml-sdk/go/alien"
)

func main() {
	defer func() { // seals the telemetry recording
		if err := alien.Shutdown(); err != nil {
			log.Print(err)
		}
	}()
	url, err := alien.CloudFormationLaunchURL(context.Background(), alien.CloudFormationLaunch{
		TemplateUrl: "https://example.com/setup.yaml", Region: "us-east-1",
		StackName: "alien-setup", Parameters: map[string]string{"ExternalId": "customer-0001"},
	})
	if err != nil {
		panic(err)
	}
	fmt.Println(url)
}
```

**Rust.** Save as `sdk/rust/examples/quickstart.rs`, then run
`cd sdk/rust && cargo run --quiet --example quickstart`.
Like every Rust program using the SDK, it also prints `[BAML INFO]` lines on
stderr: where it found the runtime library (or that it downloaded and checked
it), and `BAML (v0.23.0) loaded`. `BAML_LOG=warn` silences them. The crate
also turns on serde_json's `arbitrary_precision` and `preserve_order` features
for every crate in the build, which changes how your own code decodes some
numbers and orders JSON objects ([gaps B24](docs/gaps.md#b24)).

<!-- quickstart rust sdk/rust/examples/quickstart.rs :: cd sdk/rust && cargo run --quiet --example quickstart -->
```rust
#[tokio::main]
async fn main() -> Result<(), alien_sdk::Error> {
    let url = alien_sdk::cloudformation_launch_url(alien_sdk::CloudFormationLaunch {
        templateUrl: "https://example.com/setup.yaml".into(), region: "us-east-1".into(),
        stackName: "alien-setup".into(),
        parameters: [("ExternalId".to_string(), "customer-0001".to_string())].into_iter().collect(),
    }).await?;
    println!("{url}");
    Ok(())
}
```

### Shutting down

Each SDK runs an embedded runtime, which seals its telemetry recording when it
shuts down ([Telemetry](#telemetry); with `ALIEN_TELEMETRY=off` none of this
matters). Python and Rust shut the runtime down when the process exits
normally, TypeScript when Node's event loop runs empty; Go never does it by
itself. Call the SDK's `shutdown` last wherever that does not happen:
`process.exit()`, a signal handler, an abort, a serverless platform ending the
process, and every Go `main` (as above).

| Language | Call | Automatic at |
|---|---|---|
| TypeScript | `await shutdown()` | Node's `beforeExit`; not on `process.exit()` or signals |
| Python | `alien_sdk.shutdown()` | interpreter exit (`atexit`); not on `os._exit()` or SIGTERM |
| Go | `defer alien.Shutdown()` | never |
| Rust | `alien_sdk::shutdown()?` | normal exit; not on signals or `abort` |

SDK calls made after it fail. Details: [docs/runtime.md](docs/runtime.md#shutting-down).

## Calling the Platform API

The client defaults to `https://api.alien.dev`. The examples above do not call
it; these sketches show the shape of the API (the conformance suites run the
same calls against a local stub server).

Every operation is `<group>.<method>` as in Alien's TypeScript SDK
([docs/api-shape.md](docs/api-shape.md)):

```ts
const alien = new Alien({ workspace: "my-workspace" })   // apiKey defaults to ALIEN_API_KEY, as in Alien's SDK
const manager = await alien.managers.get("mgr_...")
const page = await alien.deployments.list({ project: "my-project", limit: 10 })
const url = await alien.setup.launchUrlForProject("my-project", "us-east-1", "alien-setup", { ExternalId: "customer-0001" })
```

```python
alien = alien_sdk.Alien(workspace="my-workspace")   # api_key defaults to ALIEN_API_KEY
manager = alien.managers.get("mgr_...")
page = alien.deployments.list(project="my-project", limit=10)
```

```go
client, err := alien.New(alien.Options{Workspace: "my-workspace"}) // API key from ALIEN_API_KEY
manager, err := client.Managers.Get(ctx, "mgr_...")
page, err := client.Deployments.List(ctx, &alien.ListDeploymentsParams{Project: alien.Ptr("my-project"), Limit: alien.Ptr[int64](10)})
```

```rust
use alien_sdk::{Alien, Options, platform};

// The API key comes from ALIEN_API_KEY unless `api_key: Some(..)` is set;
// with neither, `new` fails with INVALID_REQUEST.
let alien = Alien::new(Options { workspace: Some("my-workspace".into()), ..Default::default() })?;
let manager = platform::managers::get_async(alien.platform(), "mgr_...".into()).await?;
// Optional arguments are positional: deployments.list has 11 (project, …, limit, cursor).
let page = platform::deployments::list_async(alien.platform(), Some("my-project".into()),
    None, None, None, None, None, None, None, None, Some(10), None).await?;
```

Each client also takes a limit for each HTTP request, BAML's 5 minutes when
unset: TypeScript `timeoutMs`, Python `timeout_ms=`, Go `Options.Timeout` (a
`time.Duration`), Rust `Options::timeout` (a `Duration`); a request that runs
out fails with `COMMUNICATION_ERROR` (decision 9 in
[docs/api-shape.md](docs/api-shape.md#decisions)).

Every call fails with one error type, `AlienError` (`code`, `message`,
`retryable`, HTTP status, context). See [docs/errors.md](docs/errors.md).

**Keep the generated clients out of logs.** The clients you create (`Alien`
and `AlienManager`; in Go `*alien.Client` and `*alien.ManagerClient`) do not
print the API key or the token. In Rust and Go, every call takes a generated
client, which does:

- Rust: `Debug` of `alien.platform()` or `AlienManager::client()` shows the
  key or token. Do not `dbg!` them, do not hold one in a `#[derive(Debug)]`
  struct that gets logged, and skip them in `#[tracing::instrument]`, which
  records every argument with `Debug`: `#[tracing::instrument(skip(client))]`.
- Go: `%v`, `%+v` and `%#v` of `client.Platform` show the API key, so
  `log.Printf("%+v", client.Platform)` logs it. `json.Marshal` of a client or
  of its generated client writes the key or token.
- Python: a traceback that records local variables, as Sentry does by default,
  holds the API key ([docs/errors.md](docs/errors.md#credentials-in-python-tracebacks)).

Details: [gaps B22](docs/gaps.md#b22) and [B23](docs/gaps.md#b23).

## Using it from another project

Each recipe uses the generated code, so run `scripts/generate.sh` first. The
TypeScript and Python packages are copies: pack or install them again after
regenerating. In your own project (`/path/to/alien-baml-sdk` is this
repository):

- **TypeScript.** Pack the package and install the tarball, as
  `examples/coexistence/run.sh` does:
  `(cd /path/to/alien-baml-sdk/sdk/typescript && pnpm pack --pack-destination /path/to/packs)`,
  then `pnpm add /path/to/packs/alien-baml-sdk-0.0.0.tgz` (or `npm install` the
  same file).
  pnpm 12 fails the install over the install script of `protobufjs`, a
  dependency of the bridge that only prints a notice, unless your
  `pnpm-workspace.yaml` has `allowBuilds: { protobufjs: false }`.
- **Python.** `uv pip install /path/to/alien-baml-sdk/sdk/python` (or
  `pip install …`) into your environment, then `import alien_sdk`.
- **Go.** `go mod edit -replace example.com/alien-baml-sdk/go=/path/to/alien-baml-sdk/sdk/go`,
  then `go get example.com/alien-baml-sdk/go/alien`.
- **Rust.** A path dependency:
  `alien_sdk = { path = "/path/to/alien-baml-sdk/sdk/rust" }`. Note the
  serde_json features it brings ([gaps B24](docs/gaps.md#b24)).

## Telemetry

The BAML runtime records spans, timings and HTTP calls for every SDK function
without instrumentation code. Telemetry is **on by default** (`medium`);
**`ALIEN_TELEMETRY=off`** turns it off (the other levels are `low`, `medium`
and `high`). Without a Boundary API key it writes local files under
`$BAML_HOME/btel/` (`~/.baml` by default), which nothing deletes, and makes no
connection, unless the machine has a saved Boundary login (`baml auth login`):
then it uploads with that login ([gaps B3](docs/gaps.md#b3)). It never records
HTTP request or response bodies, which carry tokens (`http_bodies = false` in
`baml/baml.toml`). A process that ends without shutting the runtime down
leaves its recording unsealed: see [Shutting down](#shutting-down). The SDK
never prints telemetry warnings: if the endpoint cannot be reached or rejects
the key, telemetry is dropped silently. Details and the checks behind these
statements: [docs/telemetry.md](docs/telemetry.md).

## Verifying

```sh
scripts/verify.sh --phase 1   # or --phase 2: also tests the importer, checks the imported BAML and operation scenarios are current, and runs every operation
```

Runs `baml check` and `baml test`, regenerates and builds the four SDKs, runs
every conformance scenario in all four languages, runs the README quickstarts,
the telemetry checks and a secret scan, and prints a summary table. It exits
non-zero on the first failure.

## Testing an unreleased BAML build

The scripts use the published BAML 0.23.0. To try an unreleased BAML fix,
build a BAML checkout whose `baml-cli --version` reports 0.23.0
(another version needs the pins moved first: see `BAML_PIN` in
`scripts/lib.sh`), in its `baml_language/`:

```sh
cargo build --release -p baml_cli -p bridge_cffi
(cd sdks/typescript/bridge_typescript && pnpm install --ignore-workspace --ignore-scripts --frozen-lockfile && pnpm build:release)
(cd sdks/python && rm -f dist/baml_bridge-*.whl && uvx --from 'maturin>=1.10,<2.0' maturin build --release --strip --out dist)
```

Then set `BAML_LOCAL_BUILD=1` and `BAML_SOURCE` for every script; the
toolchain and all four runtime packages then come from that build:

```sh
export BAML_LOCAL_BUILD=1 BAML_SOURCE=/path/to/baml
scripts/verify.sh --phase 2    # ==> BAML 0.23.0, local build of BAML commit <sha>
```

- The scripts fail unless the CLI and every bridge were built from the
  checkout's current commit, and refuse uncommitted changes under
  `baml_language/` (except the TypeScript bridge's `dist/` and generated
  protobuf code, which its build rewrites).
- `generate.sh` links the TypeScript bridge into `node_modules`, installs the
  Python wheel into `.venv`, and points Go and Rust at the checkout's bridge
  sources with the gitignored `sdk/go/go.work` and `sdk/rust/.cargo/config.toml`.
  It also rewrites `baml_bridge` in `sdk/rust/Cargo.lock`: restore it with
  `git checkout sdk/rust/Cargo.lock` before going back to the release.
- Go and Rust programs you run yourself need
  `BAML_BRIDGE_PATH=$BAML_SOURCE/baml_language/target/release/libbridge_cffi.dylib`
  (`.so` on Linux). Set it per command, not with `export`: the TypeScript
  bridge reads the same variable as the path of its own addon.
- Back to the release: unset both variables and run `scripts/generate.sh`.

## Known gaps

[docs/gaps.md](docs/gaps.md) lists what BAML could not do for this SDK, with a minimal
repro and the workaround used here or the BAML pull request that fixes it,
plus open questions for Alien.

## Third-party material

`specs/` is copied from [alienplatform/alien](https://github.com/alienplatform/alien) at `c5c3a30`
(v3.3.43). `baml/baml_src/ns_platform/`, `baml/baml_src/ns_manager/`,
`conformance/scenarios/ops.*` and `docs/importer-coverage.md` are generated
from those specs, `tools/openapi2baml/tests/fixtures/naming-oracle.json` from
Alien's TypeScript SDK, and `baml/baml_src/ns_compat/` ports tests of Alien's
Rust client. That material is Copyright 2026 Alien Software, Inc., under the
[FSL-1.1-Apache-2.0](https://github.com/alienplatform/alien/blob/c5c3a30b3a71e819f55eb3dd8729b51dfd0db57f/LICENSE.md).
