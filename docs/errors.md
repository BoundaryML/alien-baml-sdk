# Errors

Every SDK call fails with one error type, `AlienError`; so does creating a
Platform client without an API key. It is defined once in BAML (`baml/baml_src/ns_errors/errors.baml`):

```baml
class AlienError {
    code: string,
    message: string,
    retryable: bool,
    http_status: int?,
    context: map<string, string>?,
}
```

It mirrors Alien's error contract (`AlienErrorData`: code, message, retryable,
HTTP status, context).

## Codes

Every code the SDK can produce, where it comes from, and the code Alien's
current SDKs use for the same failure. "Speakeasy" is `@alienplatform/platform-api`,
"Rust client" is `alien-platform-api` (`convert_sdk_error`), "commands" is
`@alienplatform/commands`.

| Code | Retryable | Source (BAML) | When | Alien counterpart |
|---|---|---|---|---|
| *server's own code*, e.g. `SERVICE_UNAVAILABLE`, `NOT_FOUND`, `CONFLICT` | as the body says; absent means no | `internal.platform_http_error` | A Platform API non-2xx whose body is Alien's API error (`{code, message, retryable?, hint?, requestId?, context?, …}`). `http_status` is the response status; `context` holds `requestId`, `hint` and the server's context as JSON text (`details`) | Same code and retryable flag: the API error's `code` in Speakeasy (`APIError`) and the Rust client |
| *Manager's own code* | no | `internal.manager_http_error` | A Manager API non-2xx whose body is its `ErrorResponse` (`{code, message, details?}`); `context.details` holds `details` | commands: same code, not retryable |
| `MANAGER_HTTP_ERROR` | no | `internal.manager_http_error` | A Manager API non-2xx whose body is not an `ErrorResponse`. `context.body` holds its first 1,000 characters; a JSON body of another shape also gets `json_path` and `decode_error`, as below | commands: `MANAGER_HTTP_ERROR`, not retryable |
| `PLATFORM_HTTP_ERROR` | on 408, 429 and 5xx | `internal.platform_http_error` | A Platform API non-2xx whose body is not an API error. `context.body` holds its first 1,000 characters; a JSON body of another shape also gets `json_path` and `decode_error`, as below | commands: `PLATFORM_HTTP_ERROR` (bootstrap, same retry rule). Rust client: `UNEXPECTED_RESPONSE` (retryable on 5xx). Speakeasy: `AlienDefaultError`, no code |
| `COMMUNICATION_ERROR` | yes | `internal.send`, `internal.reply` | A Platform or Manager request could not be sent, timed out, or its response body could not be read (`http_status` is then the response's status). A timeout is the client's (`timeoutMs`, [api-shape decision 9](api-shape.md#decisions)) or BAML's default of 5 minutes: `... failed: timed out after 300 ms (timeout)` | Rust client: `COMMUNICATION_ERROR` (send, a timeout included) and `RESPONSE_BODY_ERROR` (body read). commands: `COMMAND_BOOTSTRAP_FAILED` (bootstrap only). Speakeasy: `ConnectionError` and `RequestTimeoutError`, no code |
| `MALFORMED_RESPONSE` | no | `internal.decode_body`, `internal.decode_reply` | A 2xx body that is not JSON or does not match the expected shape; `http_status` is the 2xx status. `context.json_path` is where in the body decoding failed (`.items[3].status`; absent when BAML gives no location, see below), `context.decode_error` the decoder's whole message. The message gives `json_path` and the first line of `decode_error`, cut at 200 characters | commands: `MALFORMED_RESPONSE`. Rust client: `INVALID_RESPONSE_PAYLOAD`. Speakeasy: `ResponseValidationError`, no code |
| `INVALID_REQUEST` | no | `internal.invalid_request`, from `encode_body`, `path_segment` and `send` | A request the SDK refuses to send: a body that cannot be encoded as JSON, a path parameter that is empty, `.` or `..` (it would reach a different endpoint), a base URL without `https://` or `http://` or with a `#` fragment, or a request the HTTP client rejects. Each facade also raises it when a Platform client is created without an API key | Rust client: `INVALID_REQUEST`. A missing API key: Speakeasy sends the request without one and gets a 401 |
| `SETUP_INVALID_TEMPLATE_URL` | no | `setup.cloudformation_launch_url` | `templateUrl` is not an `https://` URL | none (no client-side setup helper) |
| `SETUP_REGION_UNSUPPORTED` | no | `setup.cloudformation_launch_url` | The region does not match `^[a-z]{2}-[a-z]+-[0-9]+$`, or is a China region (`cn-*`) | none |
| `SETUP_INVALID_STACK_NAME` | no | `setup.cloudformation_launch_url` | The stack name does not match `^[A-Za-z][A-Za-z0-9-]{0,127}$` | none |
| `SETUP_INVALID_PARAMETER` | no | `setup.cloudformation_launch_url` | A parameter key is empty or not ASCII alphanumeric | none |
| `SETUP_TEMPLATE_UNAVAILABLE` | no | `setup.launch_url_for_project` | `getProjectTemplateUrls` returned no AWS template URLs | none |

**Where a body did not decode.** `json_path` uses BAML's notation: `.field`
for a field, `[0]` for an array element, `["key"]` for a map entry, and it
starts at the body's root:

```
MALFORMED_RESPONSE: Response from /v1/managers/mgr_1 did not match the expected shape at .managementConfigs.aws.managingRoleArn: expected string
```

A value that matches no member of a union gets a message naming every
member's mismatch, over 2,000 characters for an external binding; the error
message keeps its first 200 and `context.decode_error` all of it. Inside an
object the SDK decodes with its own code (a class that holds an open union,
or has renamed or extra keys: 116 classes, which the responses of 55 of the
332 operations contain, among them `deployments.get`, `deployments.list`,
`events.list`, `releases.*` and the Manager's `commands.*`), BAML 0.23.0 loses
the location of a failure: `json_path` is then absent or starts at that object
([gaps B21](gaps.md#b21)). `decode_error` still says what failed.

The setup helper validates in this order: template URL, region, stack name,
parameters. It reports the first problem. Regions follow the shape of the
commercial AWS partition, so regions AWS launches later work without a change;
GovCloud, ISO and European Sovereign Cloud regions do not have that shape, and
China regions, which do, are rejected by name. Digits are `[0-9]` because
BAML's `\d` also matches non-ASCII digits.

## Per language

| Language | What you catch | Fields |
|---|---|---|
| TypeScript | `AlienError` (extends `Error`) | `code`, `message`, `retryable`, `statusCode`, `hint` and `requestId` (named as in Alien's TypeScript SDK; `hint` and `requestId` are read from `context`), `context`; `cause` is the BAML error |
| Python | `alien_sdk.AlienError` (an `Exception`; `str()` is `"<code>: <message>"`) | `code`, `message`, `retryable`, `http_status`, `context`; chained from the BAML error |
| Go | `*alien.AlienError` via `errors.As` | `Code`, `Message`, `Retryable`, `HTTPStatus`, `Context`; `Unwrap()` gives the BAML error. `context.Canceled` and `context.DeadlineExceeded` pass through unchanged; a generated call not wrapped in `alien.Do` or `alien.Err` returns `*baml_go.BamlError` |
| Rust | `alien_sdk::Error::Alien(AlienError)` | `code`, `message`, `retryable`, `http_status_code` (`Option<u16>`, named as in `alien_error::AlienError`), `context`. `Error::Runtime` holds the `baml_bridge` error, BAML trace included. A `json` value in a response holding an integer outside the `i64`/`u64` range fails the call with `Error::Runtime` ([B19](gaps.md#b19)) |

Anything else the runtime raises (a BAML panic, a runtime that cannot start)
is not converted: it reaches the caller as the BAML error itself (Rust:
`alien_sdk::Error::Runtime`).

In TypeScript, the errors that are not `AlienError` are `BamlPanic` (for
example an argument of the wrong type from JavaScript), `BamlAbortError` (a
call cancelled with `{ $baml: { signal } }`, or that ran out of its per-call
`{ $baml: { timeoutMs } }`), `BamlError` (a request body that matches no member
of a union) and `TypeError` (an unknown option name, an `AlienManager` without
`serverURL`, a `timeoutMs` that is not an integer).

A per-call timeout ends the call the same way in every language, with the
binding's cancellation error rather than `AlienError`: TypeScript
`BamlAbortError`, Python `baml_bridge.BamlCancelledError` (`_baml={"timeout_ms": …}`),
Go `context.DeadlineExceeded` (the `ctx` deadline), Rust `Error::Runtime`
(`*_with_options`). The client's timeout fails with `COMMUNICATION_ERROR`.

Each facade's conversion is a few lines: `sdk/typescript/src/errors.ts`,
`sdk/python/alien_sdk/_errors.py`, `sdk/go/alien/errors.go`,
`sdk/rust/src/lib.rs`. Go needs the most, because the Go bridge returns thrown
values as untyped maps ([gaps B5](gaps.md#b5)).

## Credentials in Python tracebacks

A Python `AlienError` is chained from the runtime's error, whose traceback
holds the frame in which `baml_bridge` ran the call. That frame keeps the
encoded call, API key included, in a local variable named `args_proto`. A
traceback printed the usual way does not show it. Anything that records local
variables does: Sentry's Python SDK by default, `rich` with
`show_locals=True`, `pytest -l`. The name does not match scrubbers that look
for `api_key` or `token`. For Sentry, turn local variables off or scrub this
one:

```python
sentry_sdk.init(dsn, include_local_variables=False)

from sentry_sdk.scrubber import EventScrubber, DEFAULT_DENYLIST
sentry_sdk.init(dsn, event_scrubber=EventScrubber(denylist=DEFAULT_DENYLIST + ["args_proto"]))
```

With `data_collection` set, Sentry ignores `event_scrubber`; use
`data_collection={"stack_frame_variables": False}` there. All three were
checked with sentry-sdk 2.73.0 ([gaps B23](gaps.md#b23)).
