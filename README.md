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

> **Temporary:** the repository builds against an unreleased BAML commit,
> built locally, which adds some developer papercuts. Read
> [Temporary: building against BAML canary](#temporary-building-against-baml-canary)
> before you build or run anything. It goes away before this is shared.

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

Generated `baml_sdk/` directories are not checked in, nor are the facades'
generated methods (`sdk/python/alien_sdk/_operations.py`,
`sdk/go/alien/operations.go`, `sdk/rust/src/operations.rs`). Never edit them, nor
the imported `baml/baml_src/ns_platform/` and `ns_manager/`: change the
importer or its config and run `scripts/import.sh`, or change the hand-written
namespaces, then run `scripts/generate.sh`.

## Requirements

Tested on macOS arm64 with Node 24.22, pnpm 12.14, Python 3.14, uv 0.13,
Go 1.27 and Rust 1.96.

- `baml` on PATH: the BAML wrapper (`brew install boundaryml/baml/baml`). The
  scripts run the CLI of the BAML build below through it
- Node and pnpm
- Python 3.10+ and uv
- Go 1.24+ with cgo (`CGO_ENABLED=1`, the default on macOS); `baml generate` runs `gofmt`
- Rust 1.91+
- A [BAML](https://github.com/BoundaryML/baml) checkout at the pinned commit,
  built with `scripts/build_baml.sh` ([Building BAML](#building-baml))

The SDKs need BAML changes that are on BAML's canary branch but in no release
yet, so the scripts use one BAML commit, `BAML_COMMIT_PIN` in
`scripts/lib.sh`, built locally: its CLI, and its runtime packages in place of
the published ones. It still reports version 0.23.0, which the manifests pin
exactly (npm `@boundaryml/baml-bridge`, PyPI `baml-bridge`, Go
`github.com/boundaryml/baml-go`, crates.io `baml_bridge`). The scripts keep
all BAML state in the repository (`.baml_home/`, `.cache/`), never in
`~/.baml`.

## Temporary: building against BAML canary

> **Remove this section** once the SDKs move to a published BAML release or
> nightly that has the changes below. This setup is for internal development
> only, and it must be cleaned up before the SDKs are shared with Alien. The
> steps are in the comment at `BAML_COMMIT_PIN` in `scripts/lib.sh`. Also
> remove the canary notes in [Requirements](#requirements),
> [Using it from another project](#using-it-from-another-project),
> [Building BAML](#building-baml) and [docs/runtime.md](docs/runtime.md).

**Why.** The SDKs need BAML changes that are on BAML's canary branch but in no
release: `naming_convention = "language"` (BAML #5481), async functions with
`_sync` twins (#5519), known words such as `OAuth` kept whole (#5524),
serde_json left alone (#5509) and Rust logging through `log` (#5521). So every script uses one BAML commit, `BAML_COMMIT_PIN` in
`scripts/lib.sh`, built from a BAML checkout you point at with `BAML_SOURCE`
([Building BAML](#building-baml)).

**Papercuts, and what to do about each one:**

- **You build BAML yourself.** That needs Rust, pnpm and uv (for maturin). Run
  `scripts/build_baml.sh $BAML_SOURCE`: the first build takes a while, later
  ones are incremental. Every script needs `BAML_SOURCE` set.
- **The checkout must be exactly at the pin.** Otherwise every script fails
  and prints the git commands to run. To try another BAML commit (a BAML pull
  request, say), set `BAML_UNPINNED=1`; the `==> BAML` line then says
  `UNPINNED`. `BAML_LOCAL_BUILD=0` (the published 0.23.0) fails: that release
  cannot build this repository.
- **Go and Rust programs you run by hand need `BAML_BRIDGE_PATH`.** These SDKs
  load BAML's engine, `libbridge_cffi`, at run time. Without the variable they
  download the published 0.23.0 library, which is too old for code generated
  by the pinned commit. Rust then fails with `incompatible BAML C API
  (abi_version 4 … needs ABI revision 5)`, and Go with `unsupported BAML ABI
  version 4 (expected 5)`. Point the variable at the local build, per command
  rather than with `export`, because the TypeScript bridge reads the same
  variable as the path of its own addon:
  `BAML_BRIDGE_PATH=$BAML_SOURCE/baml_language/target/release/libbridge_cffi.dylib cargo run …`
  (`.so` on Linux). The scripts, test suites, quickstarts and bench set it
  themselves. TypeScript and Python don't need it.
- **`pnpm install` in `sdk/typescript` undoes the local bridge.** It replaces
  the link `generate.sh` makes to the local TypeScript bridge with npm's
  0.23.0. Run `scripts/generate.sh` again. `verify.sh` refuses to test against
  npm's bridge.
- **`sdk/rust/Cargo.lock` records the local bridge.** It is a path entry, with
  no machine path. A cargo command run in `sdk/rust` without the gitignored
  `sdk/rust/.cargo/config.toml` (which `generate.sh` writes) resolves crates.io's
  bridge and rewrites the lockfile. Restore it with
  `git checkout sdk/rust/Cargo.lock`. Moving the pin also moves the lockfiles;
  the steps are at `BAML_COMMIT_PIN`.
- **The SDKs can't be installed elsewhere as they are.** The manifests still
  pin the published 0.23.0 runtime packages, which code generated by the
  pinned commit can't use. [Using it from another project](#using-it-from-another-project)
  lists what to swap in. Don't hand the SDKs to Alien in this state.
- **Use the Homebrew `baml` launcher.** The launcher a BAML checkout builds
  (`target/release/baml`) fails the scripts' version check. Keep it off `PATH`,
  or behind the Homebrew one.

## Quickstart

```sh
git clone <this repository> alien-baml-sdk && cd alien-baml-sdk
scripts/build_baml.sh /path/to/baml   # a BAML checkout at BAML_COMMIT_PIN (Building BAML)
export BAML_SOURCE=/path/to/baml
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
    template_url="https://example.com/setup.yaml", region="us-east-1",
    stack_name="alien-setup", parameters={"ExternalId": "customer-0001"})))
```

**Go.** Save as `sdk/go/cmd/quickstart/main.go`, then run
`cd sdk/go && go run ./cmd/quickstart`.
Go and Rust load BAML's runtime library at run time. With the BAML build,
point `BAML_BRIDGE_PATH` at the build's library for each Go and Rust command
(`BAML_BRIDGE_PATH=$BAML_SOURCE/baml_language/target/release/libbridge_cffi.dylib go run ./cmd/quickstart`;
`.so` on Linux). Without it, the program downloads the published 0.23.0
library into `~/.baml/bridges/0.23.0/` and fails on its older BAML ABI
version. [docs/runtime.md](docs/runtime.md) has how a release provides it.

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
It prints nothing else: BAML's Rust bridge logs through the `log` crate
(target `baml_bridge`) to whatever logger the program installs, such as
`env_logger` (`RUST_LOG=baml_bridge=debug` shows how it finds and loads the
runtime library) or a `tracing` subscriber with `tracing-log`, which
`tracing-subscriber` enables by default. `BAML_LOG` does not change it.

<!-- quickstart rust sdk/rust/examples/quickstart.rs :: cd sdk/rust && cargo run --quiet --example quickstart -->
```rust
#[tokio::main]
async fn main() -> Result<(), alien_sdk::Error> {
    let url = alien_sdk::cloudformation_launch_url(alien_sdk::CloudFormationLaunch {
        template_url: "https://example.com/setup.yaml".into(), region: "us-east-1".into(),
        stack_name: "alien-setup".into(),
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
page = alien.deployments.list(project="my-project", deployment_group="prod", limit=10)
page.next_cursor   # groups, methods, keywords and fields are snake_case
# Methods are typed (mypy and editors see each signature); a wrong keyword raises TypeError.
from alien_sdk.platform import CreateSetupLinkRequest   # request and response types, per API
request = CreateSetupLinkRequest.model_validate(form)   # camelCase (wire) or snake_case keys
```

```go
client, err := alien.New(alien.Options{Workspace: "my-workspace"}) // API key from ALIEN_API_KEY
manager, err := client.Managers.Get(ctx, "mgr_...")
page, err := client.Deployments.List(ctx, &alien.ListDeploymentsParams{Project: alien.Ptr("my-project"), Limit: alien.Ptr[int64](10)})
```

```rust
use alien_sdk::{Alien, Options};

// The API key comes from ALIEN_API_KEY unless `api_key: Some(..)` is set;
// with neither, `new` fails with INVALID_REQUEST.
let alien = Alien::new(Options { workspace: Some("my-workspace".into()), ..Default::default() })?;
let manager = alien.managers().get("mgr_...").send().await?;
// Required arguments go to the method, optional ones to the builder's setters.
let page = alien.deployments().list().project("my-project").limit(10).send().await?;
```

Each client also takes a limit for each HTTP request, BAML's 5 minutes when
unset: TypeScript `timeoutMs`, Python `timeout=` (seconds; `None` for no
limit), Go `Options.Timeout` (a `time.Duration`), Rust `Options::timeout` (a
`Duration`); a request that runs out fails with `COMMUNICATION_ERROR`
(decision 9 in [docs/api-shape.md](docs/api-shape.md#decisions)).

Every call fails with one error type, `AlienError` (`code`, `message`,
`retryable`, HTTP status, context). See [docs/errors.md](docs/errors.md).

**Keep the generated clients out of logs.** The clients you create (`Alien`
and `AlienManager`; in Go `*alien.Client` and `*alien.ManagerClient`) do not
print the API key or the token. The generated clients they hold do:

- Rust: none is reachable. `Alien`, `AlienManager` and every group and builder
  print without the key or token, so they are safe in `dbg!`, logs and
  `#[tracing::instrument]` arguments.
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
  `alien_sdk = { path = "/path/to/alien-baml-sdk/sdk/rust" }`.

Each recipe brings the published BAML 0.23.0 runtime package, which code
generated by the BAML build cannot use. Give your project the build's
packages instead, as `examples/coexistence/run.sh` does (`$BAML_SOURCE` is the
BAML checkout): TypeScript, a pnpm override
`"@boundaryml/baml-bridge": "file:$BAML_SOURCE/baml_language/sdks/typescript/bridge_typescript"`;
Python, the wheel in `$BAML_SOURCE/baml_language/sdks/python/dist/`, installed
first; Go, `replace github.com/boundaryml/baml-go => $BAML_SOURCE/baml_language/sdks/go/baml_go`;
Rust, `[patch.crates-io] baml_bridge = { path = "$BAML_SOURCE/baml_language/sdks/rust/bridge_rust" }`.
Go and Rust programs also need `BAML_BRIDGE_PATH` ([Quickstart](#quickstart)).

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

## Building BAML

The scripts build against one BAML commit, `BAML_COMMIT_PIN` in
`scripts/lib.sh`, because the SDKs need BAML changes that no release has yet.
Check it out in a BAML checkout, build it once, and give every script the
checkout as `BAML_SOURCE`; the toolchain and all four runtime packages then
come from that build:

```sh
git clone https://github.com/BoundaryML/baml /path/to/baml
git -C /path/to/baml checkout <BAML_COMMIT_PIN>
scripts/build_baml.sh /path/to/baml
export BAML_SOURCE=/path/to/baml
scripts/verify.sh --phase 2    # ==> BAML 0.23.0, local build of the pinned BAML commit <sha>
```

`build_baml.sh` runs, in the checkout's `baml_language/`:

```sh
cargo build --release -p baml_cli -p bridge_cffi
(cd sdks/typescript/bridge_typescript && pnpm install --ignore-workspace --ignore-scripts --frozen-lockfile && pnpm build:release)
(cd sdks/python && rm -f dist/baml_bridge-*.whl && uvx --from 'maturin>=1.10,<2.0' maturin build --release --strip --out dist)
```

- `build_baml.sh` and every script fail unless the checkout's HEAD is the
  pinned commit; they print the git commands that check it out, and never run
  them. They also fail unless the CLI and every bridge were built from that
  commit, and refuse uncommitted changes under `baml_language/` (except the
  TypeScript bridge's `dist/` and generated protobuf code, which its build
  rewrites).
- `BAML_LOCAL_BUILD=0`, the published BAML 0.23.0, fails: that release cannot
  build this repository.
- To try another BAML commit (a BAML pull request, say), check it out, then
  run `build_baml.sh` and the scripts with `BAML_UNPINNED=1`. The `==> BAML`
  line then says `UNPINNED`. Its `baml-cli --version` must report 0.23.0;
  another version needs the pins moved first (see `BAML_PIN` in
  `scripts/lib.sh`).
- `generate.sh` links the TypeScript bridge into `node_modules`, installs the
  Python wheel into `.venv`, and points Go and Rust at the checkout's bridge
  sources with the gitignored `sdk/go/go.work` and `sdk/rust/.cargo/config.toml`.
  `sdk/rust/Cargo.lock` and `bench/rust/Cargo.lock` record that local
  `baml_bridge` and its dependencies (a path dependency: no path, no
  checksum), as the pinned commit leaves them. Another commit may change them.
- Go and Rust programs you run yourself need
  `BAML_BRIDGE_PATH=$BAML_SOURCE/baml_language/target/release/libbridge_cffi.dylib`
  (`.so` on Linux). Set it per command, not with `export`: the TypeScript
  bridge reads the same variable as the path of its own addon.
- Back to a release: the comment at `BAML_COMMIT_PIN` in `scripts/lib.sh` has
  the steps.
- The scripts need a `baml` launcher on `PATH` whose `--version` prints
  `baml toolchain 0.23.0 …` or `baml toolchain baml-cli 0.23.0 …`
  (`toolchain_is_pinned` in `scripts/lib.sh`). The Homebrew wrapper (0.2.4)
  does. The launcher a BAML checkout builds (`target/release/baml`, wrapper
  0.2.5) prints `baml wrapper 0.2.5` and `baml-cli 0.23.0` instead, so the
  check rejects it: keep it off `PATH`, or behind the Homebrew one.

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
