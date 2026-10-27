# Bindings drift audit: Node adapter vs Python adapter

**As of** Alien commit `9ab099c3f` (2026-10-14; first released in v3.3.33).
[Upstream changes](#upstream-changes-at-491144c9) adds the adapter changes up
to `491144c9` (2026-10-19; first released in v3.3.35) and the queue batching
of `22fa631ec`, read at `e92b45e5` (the v3.3.39 release, 2026-10-21). Later
Alien releases are not covered.

## Summary

Alien's resource bindings are one Rust crate (`alien-bindings`) with two
hand-written adapters on top: napi-rs for Node and PyO3 for Python. The Rust
crate does the provider work. The adapters only translate arguments, results
and errors. This audit compares the two adapters method by method.

- **Mostly aligned.** Both expose the same handles for KV, Queue, Vault, Key,
  Postgres, Container and Sandbox, with the same methods and the same
  semantics for most calls.
- **Errors differ the most.** Node always returns the structured error
  envelope, including the cause chain. Python drops the cause chain. For
  eight kinds of bad input it raises a plain `ValueError` or `OverflowError`
  instead of the envelope. `except AlienError` does not catch those
  ([Errors](#errors)).
- **Features are missing on each side.**
  - Python lacks Storage `head`, `copy`, write attributes and write results.
    It also lacks remote Storage, Key, Sandbox and AI.
  - Node lacks Worker, local AI and Storage `get_optional`.
- **The same call has different shapes.**
  - KV TTL is `ttl_secs: u32` in Node and `ttl_seconds: u64` in Python.
  - A job poll's ending is nested in Node and flattened in Python.
  - Integer types differ in several places, partly because JavaScript
    numbers cannot carry a `u64`.
- **There are two behaviour bugs.**
  - Python's AI binding refuses an unknown provider even when the local
    override is set. TypeScript accepts it.
  - Python storage errors replace an embedded Alien error code that Node
    keeps.
- **Testing is lopsided.** The Node side has 33 Rust unit tests and 113
  TypeScript test cases. The Python side has 1 Rust test and 12 pytest tests.
  No scenario runs against both.
- **Upstream since 9ab099c3f.** The four binding features added since then
  (egress policies, internal URLs per port, `ack_batch`, `renew_lease`) each
  landed in both adapters by hand. The error drift was copied each time.

## Scope and how to read the evidence

- **Base.** Alien at `9ab099c3f` (2026-10-14).
- **Later commits.** Changes at upstream `491144c9` (2026-10-19) are listed in
  [Upstream changes](#upstream-changes-at-491144c9).
- **Method.** Every claim comes from reading the code. Nothing was built or
  run.
- **Size.** The adapters are 2,782 lines (Node) and 1,512 lines (Python) of
  Rust, including tests. The language layers above them are
  `packages/bindings` (TypeScript) and `alienplatform` (Python).

Citations use these prefixes:

| Prefix | Path in the Alien repository |
|---|---|
| `node/` | `crates/alien-bindings-node/src/` |
| `py/` | `crates/alien-bindings-python/src/` |
| `pyapi/` | `crates/alien-bindings-python/python/alienplatform/` |
| `ts/` | `packages/bindings/src/` |
| `core/` | `crates/alien-bindings/src/` |

"Node" and "Python" mean the native adapters. "TS API" and "Python API" mean
the public packages users import, which wrap the adapters.

napi-rs exposes Rust `snake_case` names to JavaScript in `camelCase`, so Node's
`ttl_secs` is `ttlSecs` in JavaScript (`ts/loader.ts:116-124`). The tables use
the Rust names.

Some integer-width differences are forced by the platform:

- napi-rs converts integers up to `u32`, plus `i64`, to a JavaScript number. A
  `u64` needs a `BigInt` (napi `src/bindgen_runtime/js_values/number.rs:63-70`
  and `bigint.rs:1`; checked in napi 3.9.0, Alien locks 3.12.2).
- PyO3 takes any width.

So a `u32` in Node against a `u64` in Python is not always a mistake. A shared
description should state the allowed range of a value (for example "seconds,
0 to 2^32-1"), not a Rust type.

## Handle coverage

| Handle | Node local | Python local | Node remote | Python remote |
|---|---|---|---|---|
| Storage | `node/lib.rs:77-82` | `py/lib.rs:174-183` | `node/lib.rs:192-197` (narrower handle) | **missing** (`py/lib.rs:95-158`) |
| Key | `node/lib.rs:85-90` | `py/lib.rs:185-192` | `node/lib.rs:214-219` | **missing** |
| KV | `node/lib.rs:93-98` | `py/lib.rs:194-201` | `node/lib.rs:207-212` | `py/lib.rs:150-157` |
| Queue | `node/lib.rs:101-106` | `py/lib.rs:203-210` | `node/lib.rs:199-205` | `py/lib.rs:141-148` |
| Vault | `node/lib.rs:109-114` | `py/lib.rs:212-219` | n/a | n/a |
| Sandbox | `node/lib.rs:117-122` | `py/lib.rs:221-228` | `node/lib.rs:221-226` | **missing** |
| Container | `node/lib.rs:125-130` | `py/lib.rs:239-246` | n/a | n/a |
| Postgres | `node/lib.rs:136-141` | `py/lib.rs:230-237` | n/a | n/a |
| Worker | **missing** (`node/lib.rs:65-142`) | `py/lib.rs:248-255` | n/a | n/a |
| AI | **missing** (TS resolves it in `packages/ai-gateway`) | `py/lib.rs:257-311` | `node/lib.rs:228-240` (lease) | **missing** |

The public APIs follow the adapters. The TS remote `Bindings` offers storage,
queue, kv, key, ai and sandbox (`ts/remote.ts:146-171`). The Python one offers
kv and queue (`pyapi/bindings.py:685-689`).

Neither adapter exposes these trait methods:

- `Vault::secret_presence` (`core/traits.rs:430-433`);
- `Container::get_container_name` (`core/traits.rs:1028`);
- `Sandbox::snapshot` (`core/traits.rs:1374`). Node says it is left out on
  purpose (`node/sandbox.rs:284-286`).

## Errors

### The envelope

Both adapters serialize the Rust error into JSON and put it in the native
error's message. The language layer then parses it.

| Aspect | Node | Python |
|---|---|---|
| Fields | `code, message, context, retryable, internal, httpStatusCode, hint, source` (`node/error.rs:37-60`) | The same **without `source`** (`py/error.rs:7-20`) |
| Carrier | `napi::Error`, status `GenericFailure` (`node/error.rs:69-84`) | `RuntimeError` (`py/error.rs:22-37`) |
| What the language layer catches | Every throw: `guard` (`ts/factories.ts:100-109`) feeds `unwrapNapiError` (`ts/errors.ts:162-203`) | Only `RuntimeError` (`pyapi/errors.py:53-63`). Anything else escapes untranslated |
| Message that is not an envelope | `BINDINGS_ERROR`, `internal: false` (`ts/errors.ts:170-177`) | `BINDINGS_ERROR`, `internal: True` (`pyapi/errors.py:21-24,40-41`) |
| `BINDING_NOT_CONFIGURED` | Typed error; context renamed to `binding`/`envVar` (`ts/errors.ts:181-192`) | Generic `AlienError`; context keeps `binding_name`/`env_var` (`pyapi/errors.py:42-50`) |
| Cause chain | Kept as `source` (`ts/errors.ts:201`). `isSandboxOutcomeUnknown` searches it (`ts/errors.ts:205-218`) | Lost twice: not in the envelope, and re-raised `from None` (`pyapi/errors.py:61`). No equivalent helper |
| `object_store` errors | An Alien error inside the error's source chain passes through unchanged (`node/error.rs:98-100,131-140`, tested at `:396-411`) | Always reclassified as `STORAGE_*` (`py/error.rs:39-64`). The original code is lost |

The `source` gap matters for Sandbox. Node's own test shows that a sandbox
refusal names its cause only in the layer beneath (`node/error.rs:271-294`).
Python callers never see that layer.

### Bad input: same mistake, different error

| Bad input | Node | Python |
|---|---|---|
| Unknown KV `condition`, or `version` condition without a version | Plain text, no envelope (`node/kv.rs:58-64`). The TS API reports `BINDINGS_ERROR`, but its own calls can't trigger this (`ts/factories.ts:377-388`) | `ValueError` (`py/lib.rs:381-388`), reachable from the Python API (`pyapi/bindings.py:188-200`) |
| Invalid storage path | `INVALID_INPUT` with `field_name` (`node/storage.rs:194-202`) | `ValueError` (`py/storage.rs:40-42`) |
| Unsupported signed-URL method | `INVALID_INPUT` (`node/storage.rs:351-357`) | `ValueError` (`py/storage.rs:164`) |
| Unsupported storage `condition` | `INVALID_INPUT` (`node/storage.rs:107-116`) | `INVALID_INPUT` (`py/storage.rs:92-101`): **matches** |
| Queue payload that is not JSON | `SERIALIZATION_FAILED` (`node/queue.rs:31-38`) | `ValueError` (`py/lib.rs:518-520,548-549`) |
| Batch result or received message that fails to serialize | `SERIALIZATION_FAILED` (`node/queue.rs:50-54,97-101,112-116`) | `ValueError` (`py/lib.rs:495-496,526-527,542-543`) |
| Negative `since_seq` | `INVALID_INPUT` (`node/sandbox.rs:474-485`) | `OverflowError` from PyO3's `u64` conversion (`py/sandbox.rs:376`) |
| Preview port above 65535 | napi `InvalidArg`, plain text (`node/sandbox.rs:546-551`) | `OverflowError` from PyO3's `u16` conversion (`py/sandbox.rs:438`) |
| Unknown external AI provider | TS API: typed `UnsupportedProviderError` (`packages/ai-gateway/src/client.ts:96-102`) | `ValueError` (`py/lib.rs:268-272`) |

`ValueError` and `OverflowError` are not `RuntimeError` subclasses. Python
callers who write `except AlienError` miss every Python row above except the
one that matches.

## Per handle

"Same" means the same parameter names and types, the same return shape and
the same error path.

### KV

| Method | Node | Python | Drift |
|---|---|---|---|
| `get` | `get(key)` returns `{key, value: Buffer, version}` or null (`node/kv.rs:90-100`) | `get(key)` returns `KvEntry` or `None` (`py/lib.rs:410-419`) | Same |
| `put` | `put(key, value, ttl_secs: Option<u32>, condition, version)` returns bool (`node/kv.rs:49-53,105-119`) | `put(key, value, ttl_seconds: Option<u64>=None, condition=None, version=None)` returns bool (`py/lib.rs:373-377,421-439`) | TTL **name and width**. Error type on bad condition (above) |
| `delete` | `delete(key, if_version)` (`node/kv.rs:121-128`) | `delete(key, if_version=None)` (`py/lib.rs:441-455`) | Same |
| `exists` | `node/kv.rs:130-135` | `py/lib.rs:457-462` | Same |
| `scan` | `scan(prefix, limit: Option<u32>, cursor)` returns `{items, next_cursor}` (`node/kv.rs:137-151`) | `scan(prefix, limit: Option<usize>=None, cursor=None)` returns `KvPage` (`py/lib.rs:464-483`) | Integer width only |

The public APIs diverge further:

- **TS API.** `get`, `getText`, `getJson`, `set(key, value: string,
  {ttl, ifVersion})`, `setJson`, `delete`, `exists` and `scan`
  (`ts/types.ts:184-208`).
  - `ifVersion: null` means "must be absent". A string means "must match"
    (`ts/types.ts:141-150`).
  - There is no public way to write raw bytes, although the native `put` takes
    a `Buffer` (`ts/factories.ts:375-389`).
- **Python API.** `put(key, value: bytes, *, ttl_seconds, condition,
  version)` (`pyapi/bindings.py:188-200`).
  - The condition is spelled `"absent"` or `"version"`.
  - There are no text or JSON helpers.

### Storage

| Method | Node | Python | Drift |
|---|---|---|---|
| `get` | Returns `{data, meta, attributes}` (`node/storage.rs:221-242`) | Returns `bytes` only (`py/storage.rs:46-61`) | **Python drops metadata and attributes** |
| `get_optional` | **missing** | `NotFound` becomes `None` (`py/storage.rs:63-79`) | Python only |
| `put` | `put(path, data, options?: {attributes, condition})` returns `{e_tag, version}` (`node/storage.rs:63-70,244-265`) | `put(path, data, condition=None)` returns `None` (`py/storage.rs:81-116`) | **Python cannot set content type, cache control or metadata, and gets no ETag or version back** |
| `delete` | `node/storage.rs:267-278` | `py/storage.rs:118-127` | Same |
| `list` | `size: f64` (`node/storage.rs:22-33,280-295`) | `size: u64` (`py/storage.rs:16-24,129-147`) | Integer type only |
| `head` | `node/storage.rs:297-317` | **missing** | Node only |
| `copy` | `node/storage.rs:319-331` | **missing** | Node only |
| `signed_url` | `(method, path, expires_in_secs: u32)` (`node/storage.rs:333-361`) | `(method, path, expires_in: u64)` (`py/storage.rs:149-173`) | **Name and width**. Error type on bad method |

Node's remote storage handle (`node/remote_storage.rs:17-110`) has a drift
of its own. It builds paths with `Path::from` (`node/remote_storage.rs:35,58,74,81,97`),
which never fails. The local handle validates the same input with
`Path::parse` and returns `INVALID_INPUT` (`node/storage.rs:194-202`). For
example, `messages//raw.eml` is rejected locally (`node/storage.rs:381-393`).
TypeScript types both handles as the same `Storage` methods
(`ts/types.ts:139`).

### Queue

| Method | Node | Python | Drift |
|---|---|---|---|
| `send_batch_json` | `(messages)` returns a JSON string (`node/queue.rs:85-102`) | `(values)` (`py/lib.rs:510-529`) | Parameter name. Error type |
| `send_batch_text` | `node/queue.rs:104-117` | `py/lib.rs:531-545` | Parameter name |
| `send_json` | `(json_string)` (`node/queue.rs:119-125`) | `(value)` (`py/lib.rs:547-557`) | Parameter name. Error type |
| `send_text` | `(text)` (`node/queue.rs:127-135`) | `(value)` (`py/lib.rs:558-566`) | Parameter name |
| `receive` | `(max: u32)`, required (`node/queue.rs:137-143`) | `(max_messages: usize = 1)` (`py/lib.rs:567-579`) | **Name and default** |
| `ack`, `nack` | `(receipt)` (`node/queue.rs:145-157`) | `(receipt_handle)` (`py/lib.rs:580-591`) | Parameter name |
| `purge` | `node/queue.rs:159-164` | `py/lib.rs:592-598` | Same |

Received messages have the same four fields on both sides
(`node/queue.rs:15-27`, `py/lib.rs:79-86`).

Batch outcomes cross as a JSON string. The TS API types them as a union
(`ts/types.ts:228-233`). The Python API types them as `list[dict[str, str]]`
(`pyapi/bindings.py:239-248`). The Rust type is `core/traits.rs:900-911`.

The remote queue is send-only in a different way on each side:

- Node returns the full native queue handle (`node/lib.rs:199-205`). Only the
  TS type narrows it (`ts/types.ts:235`).
- Python wraps it in a class with only the send methods
  (`pyapi/bindings.py:640-656`).

### Vault

| Method | Node | Python | Drift |
|---|---|---|---|
| read | `get_secret(name)` (`node/vault.rs:23-28`) | `get(name)` (`py/lib.rs:608-613`) | **Native name** |
| write | `set_secret(name, value)` (`node/vault.rs:30-38`) | `put(name, value)` (`py/lib.rs:614-627`) | **Native name** |
| delete | `delete_secret(name)` (`node/vault.rs:40-45`) | `delete(name)` (`py/lib.rs:628-633`) | **Native name** |
| list | `list_secrets()` (`node/vault.rs:47-52`) | `list()` (`py/lib.rs:634-639`) | **Native name** |

The public names agree: `get`, `put`, `delete` and `list`. The TS API adds
`getJson` and `putJson` (`ts/factories.ts:449-458`). The Python API does not
(`pyapi/bindings.py:286-304`).

### Key

| Method | Node | Python | Drift |
|---|---|---|---|
| `encrypt` | `(plaintext, context: Option<map>)` returns bytes (`node/key.rs:25-37`) | Same (`py/lib.rs:335-349`) | Same |
| `decrypt` | `node/key.rs:39-51` | `py/lib.rs:351-365` | Same |

Checked and equivalent: the Python API turns an empty context into `None`
(`pyapi/bindings.py:166`). The providers encode `None` and an empty map
identically (`core/providers/key/mod.rs:19-21`).

### Postgres

| Method | Node | Python | Drift |
|---|---|---|---|
| `connection()` | Same eight fields, `port: u32`, synchronous (`node/postgres.rs:13-34,62-69`) | Same fields, `port: u16`, synchronous (`py/lib.rs:43-54,647-662`) | Integer width only |

The driver helpers above the adapter differ by design, because they target
different drivers:

- **TS API.** A union typed by `sslmode`, with node-postgres `ssl` options
  (`ts/types.ts:306-354`, `ts/factories.ts:498-572`).
- **Python API.** `ssl_context()` and SQLAlchemy helpers
  (`pyapi/bindings.py:68-94`).

Skew detection differs. The TS API rejects an `sslmode` it does not know
(`ts/errors.ts:52-63`, `ts/factories.ts:501`). The Python API passes it
through (`pyapi/bindings.py:311-323`). Node has two Rust tests for this
mapping (`node/postgres.rs:71-137`); Python has none.

### Container

| Method | Node | Python | Drift |
|---|---|---|---|
| internal URL | `get_internal_url()`, async (`node/container.rs:21-25`) | `internal_url()`, sync (`py/lib.rs:671-673`) | **Name** |
| public URL | `get_public_url()`, async (`node/container.rs:27-31`) | `public_url()`, sync (`py/lib.rs:674-676`) | **Name** |

The public APIs keep the native names: `getInternalUrl` (`ts/types.ts:258-263`)
and `internal_url` (`pyapi/bindings.py:326-336`).

### Sandbox

| Method | Node | Python | Drift |
|---|---|---|---|
| `capabilities` | The struct is destructured, so a new capability fails to compile (`node/sandbox.rs:287-325`) | Fields are read one by one (`py/sandbox.rs:235-255`) | **Python would silently drop a new capability.** Node's comment names this exact bug (`node/sandbox.rs:287-288`). Today both list the same 12 |
| `create` | `timeout_ms: Option<u32>` (`node/sandbox.rs:327-350`) | `Option<u64>` (`py/sandbox.rs:257-274`) | Width |
| `get`, `list`, `terminate`, `pause`, `resume`, `cancel_job`, `read_file` | `node/sandbox.rs:352-358,388-394,496-515,535-540,560-575` | `py/sandbox.rs:276-285,309-318,388-416,450-469` | Same |
| `get_or_create` | `node/sandbox.rs:360-386` | `py/sandbox.rs:287-307` | Same natively. The Python API returns a tuple (`pyapi/bindings.py:502-514`); the TS API returns an object (`ts/types.ts:424-430`) |
| `run_command`, `start_job` | `timeout_ms: u32` (`node/sandbox.rs:396-461`) | `u64` (`py/sandbox.rs:320-368`) | Width. Node caps a command at about 49.7 days |
| `poll_job` | `since_seq: Option<i64>`, negatives rejected. Returns nested `exit {code, truncated}` and `error {code, message}` (`node/sandbox.rs:138-182,463-494`) | `Option<u64>`. Returns flat `exit_code, truncated, error_code, error_message` (`py/sandbox.rs:42-51,113-122,370-386`) | **Return shape** |
| `write_file` | One file (`node/sandbox.rs:517-533`) | One file (`py/sandbox.rs:418-432`) | Same natively. The TS API is `writeFiles(map)` (`ts/factories.ts:344-352`); the Python API is `write_file` (`pyapi/bindings.py:573-575`) |
| `preview` | `port: u32`, range-checked. `allowed_ports: Vec<u32>`, `expires_in_seconds: i64` (`node/sandbox.rs:47-68,542-558`) | `u16`; `Vec<u16>`; `u64` (`py/sandbox.rs:53-71,434-448`) | Widths. Error type |
| Instance and frame fields | `generation: i64`, `seq: Option<i64>` (`node/sandbox.rs:44,103`) | `u64`, `Option<u64>` (`py/sandbox.rs:22,36`) | Widths |

**Command stream `next()` and `close()`.** The contract is the same: one frame
per call, `None` at the end, and `close()` releases the stream so the backend
stops the command. The implementations differ:

| | Node | Python |
|---|---|---|
| Implementation | oneshot channel, checked after the select (`node/sandbox.rs:184-260`) | tokio `watch` channel and a biased select (`py/sandbox.rs:124-192`) |
| Race tests | 5 (`node/sandbox.rs:604-749`) | 1 (`py/sandbox.rs:476-501`) |

**Unknown values.** The TS API rejects a sandbox state or frame kind it does
not know (`ts/errors.ts:84-102`, `ts/factories.ts:160-165`). The Python API
passes it through (`pyapi/bindings.py:442-453`).

### Worker (Python only)

`invoke(method, path, headers, body, timeout_ms=None)` returns
`{status, headers, body}` (`py/worker.rs:23-54`). `public_url()` is at
`py/worker.rs:56-61`.

Python always sends an empty `target_worker` (`py/worker.rs:39`), which
providers read as "the bound worker" (for example
`core/providers/worker/local.rs:45`). Node has no worker accessor
(`node/lib.rs:65-142`), and the TS package exports none (`ts/index.ts:25-41`).

### AI

| | Node and TS | Python |
|---|---|---|
| Local, external provider | Resolved in TypeScript, outside the addon (`packages/ai-gateway/src/client.ts:84-104`) | Resolved inside the adapter (`py/lib.rs:257-290`) |
| `ALIEN_AI_LOCAL_BASE_URL` with an unknown provider | Override applied first, so any provider works (`client.ts:92-94`; test `packages/ai-gateway/src/__tests__/client.test.ts:128-135`) | **Rejected** before the override is read (`py/lib.rs:265-277`), with a `ValueError` |
| Local, managed | (not compared) | Starts an in-process gateway (`py/lib.rs:291-308`) |
| Remote lease | `node/lib.rs:228-240` | **missing** |

The AI row is a real behaviour difference. The same environment works in
TypeScript and fails in Python.

### Entry points

- Both construct from the process environment (`node/lib.rs:67-74`,
  `py/lib.rs:167-172`).
- Both build remote bindings with `for_customer` and `for_deployment`
  (`node/lib.rs:155-190`, `py/lib.rs:98-139`).
- One comment is stale. `core/bindings.rs:122-124` says the napi addon merges
  the environment with per-call overrides through `from_env_map`, but Node
  calls `from_env` (`node/lib.rs:70`).

## Cross-cutting

- **The shape of the native module is written by hand three times.**
  - The Rust structs.
  - TypeScript's `Raw*` interfaces (`ts/loader.ts:47-290`).
  - Nothing at all in Python. `_native` has no type stub
    (`pyapi/bindings.py:13`, `# type: ignore`), so the mypy run in CI
    (`.github/workflows/python-bindings.yml:66`) sees `Any` for every
    native call.
- **Bytes cost more in Python.** The Python API converts `bytes` to a
  `list[int]` before each native call (`pyapi/bindings.py:114,166,174,199,364,575`),
  which costs one list slot per byte. Node passes a `Buffer`.
- **Tests.**

  | | Node side | Python side |
  |---|---|---|
  | Rust unit tests | 33, in `error.rs` (9), `storage.rs` (9), `sandbox.rs` (6), `queue.rs` (4), `kv.rs` (3), `postgres.rs` (2) | 1 (`py/sandbox.rs:476`) |
  | Language tests | 113 (`packages/bindings/tests`, `ts/__tests__`) | 12 (`crates/alien-bindings-python/tests/test_bindings.py`) |

  No test runs the same input through both adapters and compares the results.

## Upstream changes at 491144c9

Between `9ab099c3f` and `491144c9`, one commit touched the adapters: `b652abe59`
("sandbox runtime egress policies and identity tokens"). It added the same
features to both adapters by hand:

| Feature | Node @491144c9 | Python @491144c9 | Drift introduced |
|---|---|---|---|
| Internal URLs by port | `get_internal_urls()` with **string** keys (`node/container.rs:21-29`) | `internal_urls()` with **int** keys (`py/lib.rs:671-673`) | Key type. The TS type says `Record<number, string>` (`ts/types.ts:263`); JavaScript object keys are strings at run time |
| Egress on `create`/`get_or_create` | `egress: Option<serde_json::Value>`, a JS object (`node/sandbox.rs:348,396`) | `egress_json: Option<String>`; the Python API runs `json.dumps` first (`py/sandbox.rs:269-279,319-329`) | Transport and parameter name |
| `update_egress` | `node/sandbox.rs:364-375` | `py/sandbox.rs:292-306` | Same |
| Egress parse error | `SANDBOX_EGRESS_INVALID` envelope (`node/sandbox.rs:31-38`) | Same (`py/sandbox.rs:200-207`) | None. Both copies of the message say `none` is accepted, but `EgressPolicy` has only `internet` and `allow` (`crates/alien-core/src/resources/sandbox_policy.rs:22-33`). The copy carried the mistake to both sides |
| `get_or_create(id, overrides)` overload | TS API only (`ts/types.ts:545-548`) | Not added | Missing in Python |
| Egress type | Generated from the Rust schema: `@alienplatform/core` (`ts/types.ts:1-3` @491144c9), built by `packages/core/package.json:18` | Hand-written `Literal["internet"] \| {allow}` (`pyapi/bindings.py:480-484`) | A hand copy of a generated type |

After `491144c9`, upstream `22fa631ec` (2026-10-20, "portable queue batching")
added `ack_batch` and `renew_lease` to both adapters. Node maps a serialization
failure to `SERIALIZATION_FAILED` (`node/queue.rs:154-165` @e92b45e5). Python
maps it to `ValueError` (`py/lib.rs:586-597` @e92b45e5). This repeats the
error drift in the [error table](#bad-input-same-mistake-different-error).
Changes after `e92b45e5` were not audited.

## Fixes that would remove most of the drift

These need no new tooling.

1. Python: add `source` to the envelope (`py/error.rs:7-20`). Map every
   `ValueError` and integer `OverflowError` path above to `INVALID_INPUT` or
   `SERIALIZATION_FAILED`, as Node does.
2. Python: search an `object_store` error's source chain for an Alien error, as
   `node/error.rs:131-140` does.
3. Python: destructure `SandboxCapabilities` (`py/sandbox.rs:235-255`).
4. Move AI provider resolution into Rust, so that the Python adapter and the TS
   gateway stop carrying separate copies.
5. Pick one name per parameter (`ttl_seconds` and `expires_in_seconds` read
   best), one allowed range per value that both platforms can carry (see
   [Scope](#scope-and-how-to-read-the-evidence)), and one poll-result
   shape.
6. Decide whether Storage `head`/`copy`/attributes, Worker and remote
   Storage/Key/Sandbox/AI are meant to exist in both languages. Record the
   answer where a test can check it.

[options.md](options.md) compares ways to keep the adapters from drifting
again.
