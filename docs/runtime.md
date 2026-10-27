# The BAML runtime

Each generated SDK runs the project's compiled BAML code inside an embedded
runtime, version **0.23.0**, which every runtime package must match exactly
(the generated code checks it at start-up).

| Language | Runtime package | How it gets the native runtime |
|---|---|---|
| TypeScript | npm `@boundaryml/baml-bridge` | A per-platform addon package installed by npm |
| Python | PyPI `baml-bridge` | Linked into the wheel |
| Go | `github.com/boundaryml/baml-go` (cgo) | Loads `libbridge_cffi` at run time |
| Rust | crates.io `baml_bridge` | Loads `libbridge_cffi` at run time |

## Go and Rust: the runtime library

Go and Rust look for `libbridge_cffi` in this order:

1. `BAML_BRIDGE_PATH` (an absolute path; no fallback if it is wrong),
2. the cache at `$BAML_HOME/bridges/0.23.0/<target>/` (`BAML_HOME` defaults
   to `~/.baml`),
3. a download from Boundary's release manifest, verified by SHA-256, into that
   cache. `BAML_BRIDGE_DISABLE_DOWNLOAD=true` turns the download off.

So the first Go or Rust program on a machine downloads the library (about
34 MB for macOS arm64), and every later one loads the cached copy. Go also
caches the manifest (`bridges/0.23.0/manifest.json`) and checks the library
against it at every start; Rust trusts a cached library. Where a program must
not download at run time (a container, CI without network), either fill that
cache at build time by running the program once with the same `BAML_HOME`, or
put the library anywhere and point `BAML_BRIDGE_PATH` at it.

The scripts fetch the library once into `.cache/bridge-home/bridges/` (with
`cargo run --example provision_bridge` in `sdk/rust`, then the manifest), or
use the one of a [local BAML build](../README.md#testing-an-unreleased-baml-build),
and run every Go and Rust test with `BAML_BRIDGE_PATH` pointing at it and
downloads disabled. The README quickstarts run as written, with the release's
library already in their `BAML_HOME`'s cache. Do not set `BAML_BRIDGE_PATH`
for TypeScript: its bridge reads the same variable as the path of its `.node`
addon.

Go needs cgo (`CGO_ENABLED=1`, the default on macOS with Xcode command-line
tools). `baml generate` runs `gofmt`, so Go must be installed where you
generate.

Rust prints `[BAML INFO]` lines on stderr once per process, when the library
loads: where it found it (`Found cached BAML library at …`, or a checksum and
`Successfully downloaded and cached BAML library at …` line after a download)
and `BAML (v0.23.0) loaded`. `BAML_LOG=warn` silences them. Go prints nothing.

## One runtime per process

The runtime is process-global. TypeScript and Python start it when the
generated SDK is imported; Go and Rust on the first call (Rust:
`alien_sdk::init()` starts it early). A second BAML-generated SDK in the same
process replaces it ([gaps B4](gaps.md#b4)).

Calls:

- TypeScript: every Platform and Manager method and `alien.setup.launchUrlForProject`
  are `async`; only `cloudFormationLaunchUrl` is synchronous (it does no I/O).
- Python: synchronous (`Alien`), or `async` (`AsyncAlien`).
- Go: synchronous, with a `context.Context`.
- Rust: each operation has an `async` function (`<name>_async`) and a
  blocking one (`<name>`); the blocking one fails with `Error::Runtime` inside
  a Tokio runtime. `alien_sdk::init()` and `alien_sdk::shutdown()` block.

## Shutting down

Shutting the runtime down waits for calls in flight, seals the telemetry
recording and releases the runtime. SDK calls made afterwards fail, and a
second shutdown does nothing. Each SDK exports one call for it; the runtime
also shuts down by itself in some cases:

| Language | Call | Shuts down by itself |
|---|---|---|
| TypeScript | `await shutdown()` | When Node's event loop runs empty (`beforeExit`). Not on `process.exit()`, on signals, or when a test runner ends its workers |
| Python | `alien_sdk.shutdown()` (blocks, with no time limit, until calls in flight finish; Ctrl+C ends the wait); later calls raise `baml_bridge.BamlPanic`, a `BaseException` | At interpreter exit (`atexit`), which includes `sys.exit()`, an uncaught exception and Ctrl+C, waiting as `shutdown()` does, with no time limit. Not on `os._exit()` or a signal that kills the process (SIGTERM by default) |
| Go | `alien.Shutdown()` | Never. Unless telemetry is off, `defer alien.Shutdown()` in `main` (`os.Exit` and `log.Fatal` skip deferred calls). It does nothing before the first call; calls after it fail with a runtime error that is not an `*alien.AlienError` |
| Rust | `alien_sdk::shutdown()` (blocks, with no time limit, until calls in flight finish; from async code use `tokio::task::spawn_blocking`) | When the process exits normally (`main` returns, `std::process::exit` on Unix), waiting as `shutdown()` does, with no time limit. Not on signals, `std::process::abort`, `panic = "abort"`, or `std::process::exit` on Windows. `alien_sdk::init()` afterwards returns `Ok` but does not restart the runtime |

**When it is needed.** Only with telemetry on (the default; `ALIEN_TELEMETRY=off`
turns it off, see [telemetry.md](telemetry.md)): the runtime writes recordings about
once a second, so a process that ends without a shutdown loses its last
second of telemetry and leaves the recording unsealed. Call the SDK's shutdown
last wherever the table says it does not happen by itself: before
`process.exit()`, in a signal handler, before an abort, in every Go `main`. A
serverless function that is frozen between invocations and then discarded
never exits normally either; shut down where the platform signals the end (a
SIGTERM handler, for example), not after each invocation, since calls made
afterwards fail.

**When it is not.** With telemetry off, and in TypeScript, Python and Rust
programs that end normally.
