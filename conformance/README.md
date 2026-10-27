# Conformance scenarios

One JSON file per scenario in `scenarios/`. Every language runs every scenario
through its facade against the same stub server, so the four SDKs are checked
against the same expectations.

```json
{
  "id": "platform.getManager.rate_limited_429",
  "phase": 1,
  "call": { "fn": "platform.getManager", "args": { "client": { "base_url": "${stub.base}", "api_key": "${secrets.apiKey}", "workspace": null }, "id": "mgr_…" } },
  "exchanges": [
    { "expect": { "method": "GET", "path": "/v1/managers/mgr_…" },
      "respond": { "status": 429, "json": { "error": "slow down" } } }
  ],
  "expect_result": null,
  "expect_error": { "code": "PLATFORM_HTTP_ERROR", "retryable": true, "http_status": 429 },
  "assert": []
}
```

| Field | Meaning |
|---|---|
| `id` | File name without `.json` |
| `phase` | Harnesses run scenarios with `phase <= CONFORMANCE_PHASE` (all when unset) |
| `rule` | Optional: the Alien regression test a `compat` scenario ports |
| `call.fn`, `call.args` | The function to call and its arguments, by wire name. Each harness maps `fn` to its facade (or, for `compat.*`, to the generated SDK); a client's `timeout_ms` becomes the facade's timeout option |
| `call.op`, `call.params` | Instead of `fn`: a generated operation and its parameter list ([Operation scenarios](#operation-scenarios)) |
| `exchanges` | Requests the stub expects, in order, and its answers |
| `expect_result.ok` | The expected result, compared after normalization |
| `expect_error` | The expected error, reduced to `{code, retryable, http_status}`, plus `context` when the scenario lists one |
| `assert` | Extra checks on the request log: `no_requests`, `api_key_never_sent_to:<path prefix>` |
| `capture_raw` | The harness also saves the raw result (the shared setup fixture) |

Templates in any string: `${stub.base}` (the stub's base URL),
`${secrets.<name>}` (from `secrets.json`), `${now+300s}` (an ISO-8601 instant,
the same for every occurrence in one scenario), and `${$}` for a literal `$`
(`${$}{x}` is the text `${x}`, as in the spec value `projects/${projectName}`).

## Exchanges

`expect` may check `method`, `path` (with query string, exact), `headers`
(subset, names case-insensitive), `headers_absent`, `json` (deep equality) and
`body_empty`. `respond` sets `status`, `json` or `text`, `headers`,
`delay_ms`, or `drop: true` (close the connection without answering).

A request that does not match the next exchange, or arrives when none is left,
is answered with 599 and fails the scenario. So does an exchange that is never
requested.

## Normalization

- Object keys are sorted.
- Members whose value is `null` are dropped, so absent and `null` compare
  equal across languages. (BAML-level absent-vs-null behavior is tested by
  the `ns_compat` functions, which return strings.)
- Errors are reduced to `{code, retryable, http_status}`; `context` is
  compared too when the expected error has it.

The comparison lives in the stub (`stub-server/normalize.mjs`), so every
language uses the same one.

## Operation scenarios

`ops.*.json` (phase 2) are generated, one set per imported operation, by
`scripts/operation_scenarios.py`; `--check` (run by `scripts/verify.sh
--phase 2`) fails when they are out of date. Do not edit them: change the
script, or the specs and importer it reads, and rerun it.

- The spec is the oracle. Method, path, query string (`workspace` first, then
  spec order), auth and header parameters, and JSON body come from the
  operation in the OpenAPI document; the response is synthesized from its
  success schema (every required property, optional ones at the top level).
  Values come from the spec's `example` where a scalar has one, else from the
  property's name and format. A string is then made to satisfy its schema's
  `pattern`, `minLength`, `maxLength` and `format: uuid`, which Alien's Rust
  client checks: the spec's ID examples are four characters short of their
  patterns, so `dep_0c29fq4a2yjb7kx3smwdgxlc` becomes
  `dep_0c29fq4a2yjb7kx3smwdgxlcdep0`.
- `ops.<op>.success` sets every argument; `ops.<op>.required_only` (when the
  operation has optional arguments) sets none of them; `ops.<op>.api_error`
  answers with the service's error body (Platform `APIError`, Manager
  `ErrorResponse`) and the error status the operation declares first, and
  expects an `AlienError` with the server's code. `EXTRA_CASES` in the
  script adds other error mapping cases for a few representative operations,
  and `UNION_CASES` three `deployments.get` responses with external bindings
  this SDK does not know: a known `type` with a new `service`
  (`storage`/`minio`) and a new `type` decode as `ExternalBindingOther`, and
  a known pair that does not decode fails (docs/api-shape.md, "Unions").
  Every imported operation, 332, has these: 834 scenarios.
- The expected result is the host view of the response: BAML field names
  (`customDomains` for the wire name `custom_domains`, `test_` for `test`,
  B14) and unlisted keys under `additionalProperties`.

The call names the operation by its BAML path and lists its parameters in
declaration order:

```json
"call": {
  "op": "platform.managers.list",
  "params": [{ "name": "search", "type": "string", "optional": true }, { "name": "limit", "type": "int", "optional": true }],
  "args": { "client": { "base_url": "${stub.base}", "api_key": "${secrets.apiKey}", "workspace": "my-workspace" }, "limit": 2 }
}
```

Each harness runs every `op` through one generic dispatch over the generated
SDK (Go: over the facade's generated methods), so facade renames do not touch
it. A `required_only` scenario passes no optional arguments at all.

| Language | Dispatch | Arguments | Result |
|---|---|---|---|
| TypeScript | `sdk.<service>.<group>.<name>_async`, found by path | plain JSON | `JSON.stringify` |
| Python | `baml_sdk.<service>.<group>.<name>`, found by path | pydantic, from `params[].type` | `model_dump` |
| Go | the operation's method (`c.Deployments.List(ctx, …)`), from the `operations` table `internal/genops` writes into `sdk/go/alien/operations_table_test.go` (gitignored) | decoded by the SDK (`InternalDecodeBody`) | struct fields by `json` tag, a union as its arm |
| Rust | `operation` table in `sdk/rust/tests/support/operations.rs`, written by `scripts/operation_scenarios.py` (`--check` keeps it current) | decoded by the SDK (`internal::decode_body_async`) | the wire value's fields |

Go and Rust need one table row per operation (they cannot look a function up
by name). Both tables are generated, so they hold no hand-written
per-operation code, and the compiler checks every row against the generated
signatures. Go's rows call the facade's methods, so they also check
`internal/genops`.

## Stub server

`node stub-server/server.mjs [--port N]` prints `READY http://127.0.0.1:<port>`.
Control endpoints:

| Endpoint | Purpose |
|---|---|
| `POST /__load {scenario}` | Load a scenario; returns the resolved `call` (and the whole resolved `scenario`) |
| `POST /__result {ok}` or `{error}` | Compare the outcome; returns `{ok, problems, normalized}` |
| `GET /__log` | Every request so far, with a timestamp in milliseconds |

## Running

`scripts/verify.sh` runs every suite, each with its own empty `BAML_HOME`. To
run them by hand after `scripts/verify.sh`, give each run a fresh
`BAML_HOME`. Every suite refuses to start when `BAML_HOME` is unset or is
`~/.baml`. Go and Rust take the runtime library that `verify.sh` fetched
(with a [local BAML build](../README.md#testing-an-unreleased-baml-build):
`$BAML_SOURCE/baml_language/target/release/libbridge_cffi.dylib`, `.so` on Linux):

```sh
export BAML_HOME="$(mktemp -d)"   # never the developer's ~/.baml
bridge=$(echo "$PWD"/.cache/bridge-home/bridges/0.23.0/*/libbridge_cffi.*)
pnpm -C sdk/typescript test
uv run --frozen --directory sdk/python pytest
(cd sdk/go && BAML_BRIDGE_PATH=$bridge CGO_ENABLED=1 go test ./...)
(cd sdk/rust && BAML_BRIDGE_PATH=$bridge cargo test)
```

The Go tests also exit with status 2 unless `BAML_BRIDGE_PATH` or
`BAML_BRIDGE_DISABLE_DOWNLOAD=true` is set (the commands above set
`BAML_BRIDGE_PATH`).

Go and Rust load that runtime library ([docs/runtime.md](../docs/runtime.md)).
Set `BAML_BRIDGE_PATH` per command, not with `export`: the TypeScript bridge
reads the same variable as the path of its own addon.
`CONFORMANCE_OUT=<dir>` makes each harness write its normalized outcomes there.

## Fixtures and secrets

- `fixtures/setup_launch_url.json`: the shared setup-helper fixture. Every
  language must produce `expected` byte for byte (`scripts/verify.sh` compares
  the raw outputs with `cmp`).
- `secrets.json`: fake secrets. `scripts/secret_scan.py` fails if any of them
  appears in another file, and the telemetry checks search recordings for them.
