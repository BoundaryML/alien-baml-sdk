# Options for the bindings adapters

## Summary

The Rust crate `alien-bindings` stays the single implementation in every
option below. The question is how the per-language adapters on top of it are
produced and kept in step.

| Option | Effort for Alien | BAML work | What it adds or removes |
|---|---|---|---|
| (a) A BAML interface for foreign native libraries | 1–2 months | A new BAML feature (B11), needed before Alien can start | Adds BAML's runtime to every workload |
| (b) Generate the adapters from one interface description | 2–4 months | None | Removes duplicated adapter code; gives Go a clear path |
| (c) Keep the adapters, add contract tests generated from one spec | 3–5 weeks | None | Catches drift, does not remove it |

- **(a)** does not exist today. BAML would have to build it first.
  - It adds BAML's start-up to Node and Python processes (for this
    repository's SDK, with all 332 operations: 82 ms over bare Node and
    275 ms over bare Python) and 9–21 µs per call.
  - It inherits BAML's limit of one runtime per process.
  - It would let helpers be written once in BAML, but almost no helper code
    sits above the bindings today.
- **(b)** needs nothing from BAML. The largest single piece is the Go target.
  BAML's role would be limited to pure helpers above the bindings, and there
  are almost none.
- **(c)** changes nothing users see. Its tests would fail today on most items
  in the [drift audit](drift-audit.md).

**Recommendation.**

1. Do (c) now.
2. Write its spec so that it can later drive the generator in (b).
3. Move the little shared logic there is into the Rust crate.
4. Start (b) before adding an adapter for another language.

Do not plan on (a) for bindings that run inside customer workloads.

**Where BAML does not help.** BAML generates code from BAML source. It cannot
call a native library today (gap B11), and nothing in the drift audit is
caused by logic that BAML could own. Whatever shared logic sits beneath the
language line already has a home: the Rust crate. BAML's strengths
(generated multi-language clients over an HTTP API, typed errors, built-in
telemetry) apply to the client SDKs in this repository, not to the bindings.
(The BAML gap list is in [gaps.md](../gaps.md); B11 has no entry there
because nothing in the client SDKs needs it.)

## What any option must keep

These come from the current adapters. An option that breaks one of them is a
regression.

| Requirement | Today |
|---|---|
| The Rust crate is the only implementation | Both adapters say they only translate (`crates/alien-bindings-node/src/lib.rs:1-7`, `crates/alien-bindings-python/src/lib.rs:1-5`) |
| In-process, low overhead | Bindings run on the request path of customer workloads |
| Async in each language | napi `async fn`; Python through `pyo3_async_runtimes` and asyncio (`crates/alien-bindings-python/src/lib.rs:35-41`) |
| Byte payloads | KV values, objects, ciphertext and sandbox files are bytes |
| Streams with cancellation | Dropping a sandbox command stream is what stops the command (`crates/alien-bindings-node/src/sandbox.rs:184-190`) |
| Packaging | The Node addon is embedded in single-file executables by `alien build` (`packages/bindings/src/native.ts:1-26`). Python ships abi3 wheels (`crates/alien-bindings-python/Cargo.toml:31`) |

## (a) A BAML interface for foreign native libraries

### What it would look like

BAML code would declare the library's handles and functions. The BAML runtime
would load the library, and BAML's generators would emit the four language
SDKs. The syntax below does not exist; it shows the shape only.

```baml
// Hypothetical. Not valid BAML today.
extern library alien_bindings {
  class Kv { _handle: $foreign }

  function open_kv(name: string) -> Kv throws BindingsError
  function kv_get(kv: Kv, key: string) -> KvEntry? throws BindingsError
  function kv_put(kv: Kv, key: string, value: uint8array, ttl_seconds: int?) -> bool throws BindingsError
}
```

The closest existing mechanism is the standard library's opaque handles
(`_handle: $rust_type`) and host functions (`$rust_function`), for example
`baml_language/crates/baml_builtins2/baml_std/trace/trace.baml:39-61` in the
BAML repository. Their implementations are compiled into the runtime, and the
compiler allows them only in standard-library files
(`baml_language/crates/baml_compiler2_hir/src/diagnostic.rs:55,443-452`).

BAML itself is built the way option (b) describes. Its runtime is a Rust
library exposed through a C ABI, with a bridge per language
([runtime.md](../runtime.md)). Option (a) would let a BAML program call a
second library of that kind.

### What BAML would need to build

1. **Language.** User-declared opaque handles and foreign function
   declarations.
2. **Runtime.**
   - Load a third-party native library on each platform and check its ABI
     version.
   - Call it asynchronously without blocking the VM.
   - Release handles deterministically, because stream cancellation depends
     on it.
3. **A C ABI contract for foreign libraries.** It has to cover bytes, maps,
   optionals, typed errors with a cause chain, async completion and
   pull-based streams.
4. **Generators, in four languages.** Expose handles and streams
   idiomatically: an async iterator in TypeScript and Python, and an iterator
   or channel in Go and Rust.
5. **Packaging.** Ship the foreign library next to BAML's own runtime in every
   npm package, wheel, Go module and crate, for every platform.
6. **Async Python.** Python calls into a BAML SDK can be synchronous or
   `async` ([runtime.md](../runtime.md#one-runtime-per-process)). The bindings
   are asyncio, so a foreign call made from an `async` function must not block
   the event loop.
7. **Gap B4.** Only one BAML-generated SDK can run in a process
   ([gaps B4](../gaps.md#b4)). An application that uses the bindings and any
   other BAML SDK, including this repository's Platform SDK, would collide.
8. **Telemetry.** BAML must decide what it records about a foreign call whose
   arguments are secrets or object bytes. An SDK records at `medium` when the
   level is unset ([gaps B1](../gaps.md#b1)).

### Effort

Alien's part can start only after BAML has items 1–8.

| Work | Estimate |
|---|---|
| C ABI shim over `alien-bindings` | 2–4 weeks |
| BAML declarations, facades, user migration | 2–4 weeks |

### What Alien keeps and loses

**Keeps:**

- The Rust crate.
- One declaration of the surface, from which BAML generates TypeScript,
  Python, Go and Rust.
- Helpers written once in BAML.

**Loses:**

- **Runtime weight on the request path.** Measured BAML start-up, with all
  332 operations, costs 82 ms over bare Node, 275 ms over bare Python, and
  32–38 ms in total in Rust and Go. Each call into the runtime costs 9–21 µs
  ([metrics](../../metrics/results.md)).
  The current adapters add neither.
- **Idioms that work today.** These include Node `Buffer`s, the async-iterator
  command stream and the bun single-file embedding. Each language would depend
  on the quality of BAML's generator for it. The Go and Rust generators are
  alpha ([gaps](../gaps.md)).
- **Release independence.** Every package must match the BAML runtime version
  exactly ([runtime.md](../runtime.md)).

### Verdict

Option (a) gives what (b) gives, plus helpers in BAML. It needs a new BAML
feature first, then adds a runtime and a version coupling to the hottest path.
Not recommended for in-workload bindings unless BAML builds B11 for other
reasons and fixes B4 first.

## (b) Generate the adapters from one interface description

The description lists the bindings once. A generator writes the per-language
adapter code. BAML is not involved in the adapters. It could hold pure helper
logic above them, but see [BAML's role](#bamls-role-in-b).

There are two realistic shapes.

### (b1) An off-the-shelf generator: UniFFI

Annotate a wrapper crate over `alien-bindings`:

```rust
#[derive(uniffi::Record)]
pub struct KvEntry { pub key: String, pub value: Vec<u8>, pub version: String }

#[derive(uniffi::Object)]
pub struct KvHandle { inner: Arc<dyn alien_bindings::Kv> }

#[uniffi::export(async_runtime = "tokio")]
impl KvHandle {
    pub async fn put(&self, key: String, value: Vec<u8>, ttl_seconds: Option<u64>)
        -> Result<bool, BindingsError> { /* call self.inner */ }
}
```

Language coverage, checked on crates.io and GitHub on 2026-10-21:

- **Python.** Official backend, built on ctypes.
- **Go.** NordSecurity's `uniffi-bindgen-go`. Its latest tag, `v0.7.1+v0.31.0`,
  targets UniFFI 0.31.0; UniFFI itself is at 0.32.2. Every external generator
  must target the same UniFFI version, so the slowest backend sets Alien's
  version.
- **Node.** Only community generators before 1.0: `uniffi-bindgen-node-js`
  0.0.16 (built on the koffi FFI library) and `uniffi-bindgen-js` 0.3.0.

The costs:

- Node would move from napi-rs to a foreign-function library. That changes
  the single-file embedding `alien build` relies on
  (`packages/bindings/src/native.ts:1-26`).
- Python would move from PyO3 to ctypes.
- The JSON error envelope would become a UniFFI error type.

### (b2) An Alien-owned spec and generator

Write one file per handle that gives each method's parameters (names, value
ranges, units), its result and its error codes:

```yaml
handle: kv
methods:
  put:
    params:
      key: string
      value: bytes
      ttl_seconds: { type: int, range: [0, 4294967295], optional: true }
      condition: { enum: [absent, version], optional: true }
      version: { type: string, optional: true, required_when: { condition: version } }
    returns: bool
    errors: [INVALID_INPUT, KV_OPERATION_FAILED]
```

From that file, generate:

- the napi-rs and PyO3 adapter code, including the input checks, so both
  report `INVALID_INPUT` the same way;
- TypeScript's `Raw*` interfaces, which are hand-written today
  (`packages/bindings/src/loader.ts:47-290`);
- a Python type stub for `_native`, which has none today;
- for Go, a C ABI layer and a cgo package.

This keeps the napi-rs and PyO3 toolchains, the packaging and the bun
embedding. The cost is owning a generator.

The idiomatic public APIs can stay hand-written. Examples are KV `setJson` in
TypeScript and the Postgres driver helpers in each language. They are small,
and they differ on purpose.

### BAML's role in (b)

BAML code could hold pure helper functions: the facade passes data in and
gets data back. A BAML function cannot call the bindings, because that is
B11. The audit found these candidates. None fits BAML well:

| Logic | Where it lives today | Better home |
|---|---|---|
| AI provider base-URL resolution, which has already drifted | `crates/alien-bindings-python/src/lib.rs:264-277`, `packages/ai-gateway/src/client.ts:84-104` | The Rust crate. Both adapters already call into it |
| Postgres TLS to driver options | `packages/bindings/src/factories.ts:498-572`, `crates/alien-bindings-python/python/alienplatform/bindings.py:68-94` | Each language. BAML cannot return an `ssl.SSLContext` or node-postgres options |
| KV and Vault text/JSON helpers, queue batch decoding | `packages/bindings/src/factories.ts:375-458` | Each language. Native JSON is simpler |

Putting BAML code into the bindings packages would also run into
[gaps B4](../gaps.md#b4): an application that uses the bindings and the
Platform SDK would load two BAML SDKs into one process.

### Effort

| Work | Estimate |
|---|---|
| (b2): spec format and generator for napi-rs, PyO3, TS types and a Python stub | 4–6 weeks |
| (b2): moving every handle over while keeping the public APIs | 2–3 weeks |
| (b2): Go (C ABI, cgo, packaging) | 4–6 weeks |
| (b1) instead | About 2–3 months, with risk concentrated in the Node backend |

If (c) comes first, its spec becomes the description and its tests become
the acceptance suite.

### What Alien keeps and loses

**Keeps:**

- The Rust crate.
- The current toolchains (b2).
- No added runtime.
- Independent releases.

**Loses:**

- The freedom to tweak one adapter by hand.
- A generator to maintain (b2), or a dependency on young third-party backends
  (b1).

## (c) The current adapters plus generated contract tests

Keep both adapters as they are. Write the surface down once and run the same
checks against both. "Generated" here means the tests come from the spec:
one small runner per language reads the spec and the scenarios, so adding a
case adds it to every language. This is the pattern this repository already uses for the
client SDKs: one JSON scenario per case, run by every language
([conformance](../../conformance/README.md)).

There are two kinds of test.

1. **Surface checks.** Compare each native module with the spec: methods,
   parameter names, optionality and value ranges.
   - Python: PyO3 methods expose `__text_signature__`.
   - TypeScript: check against the addon's generated type declarations, or
     against `loader.ts`.
   - Missing methods fail unless the spec marks them as one-language on
     purpose.
2. **Behaviour scenarios.**
   - Run each call through each native module against the local providers.
     Both test suites already run those (`crates/alien-bindings-python/tests/test_bindings.py:71`,
     `packages/bindings/tests/helpers/`).
   - Compare results and error envelopes: code, `retryable`, context keys and
     cause chain.
   - Run a smaller set through the public APIs, to check that every failure
     reaches the user as `AlienError`.

A scenario looks like this:

```json
{
  "id": "kv.put.version_condition_without_version",
  "binding": { "kind": "kv", "name": "cache" },
  "call": { "method": "put", "args": { "key": "k", "value_b64": "dg==", "condition": "version" } },
  "expect_error": { "code": "INVALID_INPUT", "retryable": false, "context": { "field_name": "version" } }
}
```

Today this scenario fails in both languages, differently. Node returns plain
text with no code. Python raises `ValueError`.

**What the tests would catch from the audit:**

- every row of the bad-input error table;
- the missing cause chain;
- the `object_store` reclassification;
- the poll-result shape;
- the parameter names;
- the missing methods;
- the AI override difference.

**What they would not catch:**

- concurrency races, such as stream close, unless written as dedicated
  scenarios;
- anything in a language that has no adapter, such as Go.

### Effort

| Work | Estimate |
|---|---|
| Spec of the current surface, about 70 native methods | 3–5 days |
| One runner per language | About 1 week |
| A first set of about 80 scenarios | About 1 week |
| Fixing what the tests find | 1–2 weeks. Separate, and partly listed in the audit |

### What Alien keeps and loses

**Keeps** everything. Users see no change.

**Loses** nothing, but it removes no work either. Every feature still lands
twice, and once more for each language added. The tests find drift after
the fact; they do not prevent it.

## Side by side

| | (a) BAML foreign interface | (b) Generated adapters | (c) Contract tests |
|---|---|---|---|
| Removes duplicated adapter code | Yes | Yes | No |
| Stops drift | By construction | By construction, for generated parts | Detects it in CI |
| Go | Through BAML's Go generator (alpha) | C ABI and cgo, or `uniffi-bindgen-go` | A third hand-written adapter |
| Runtime added to workloads | BAML runtime | None | None |
| Alien effort | 1–2 months, once BAML has the feature | 2–4 months | 3–5 weeks |
| Can start now | No: BAML needs B11 first | Yes | Yes |

These estimates are rough, for one engineer who knows the code. They come
from the size of the current adapters (4,294 lines of Rust across both). They
are not measured.

## Open questions

1. Which differences in the audit are intended? Python's `get_optional` and
   Worker may be deliberate. The spec has to record the answer.
2. For (b): would Alien rather own a generator (b2) or depend on UniFFI's
   community backends (b1)?
