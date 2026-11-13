# Errors

Every SDK call fails with one error type, `AlienError`; so does creating a
Platform client without an API key. It is defined once in BAML (`baml/baml_src/ns_errors/errors.baml`):

```baml
class AlienError {
    code: string,
    message: string,
    retryable: bool,
    http_status: int?,
    context: json?,
    hint: string?,
    request_id: string?,
    source: AlienError?,
    internal: bool?,
}
```

It mirrors Alien's error contract (`AlienErrorData`: code, message, retryable,
HTTP status, context, hint, the `internal` flag and the `source` chain), plus
the request ID. The last four fields come from a server's error body
([The server's fields and the source chain](#the-servers-fields-and-the-source-chain));
the errors the SDK raises itself leave them null. `context` is JSON: the
server's own, exactly as sent, or a small object of the SDK's
([Context](#context)).

## Codes

Every code the SDK can produce, where it comes from, and the code Alien's
current SDKs use for the same failure. "Speakeasy" is `@alienplatform/platform-api`,
"Rust client" is `alien-platform-api` (`convert_sdk_error`), "commands" is
`@alienplatform/commands`.

| Code | Retryable | Source (BAML) | When | Alien counterpart |
|---|---|---|---|---|
| *server's own code*, e.g. `SERVICE_UNAVAILABLE`, `NOT_FOUND`, `CONFLICT` | as the body says; absent means no | `internal.platform_http_error` | A Platform API non-2xx whose body is Alien's API error (`{code, message, retryable?, hint?, requestId?, context?, …}`). `http_status` is the response status; `context`, `hint`, `request_id`, `internal` and `source` are the body's, `context` exactly as sent | Same code and retryable flag: the API error's `code` in Speakeasy (`APIError`) and the Rust client |
| *Manager's own code* | no | `internal.manager_http_error` | A Manager API non-2xx whose body is its `ErrorResponse` (`{code, message, details?}`); `context` is `{"details": details}`. A body in Alien's full error shape also gives `hint`, `request_id`, `internal` and `source`, and its `context` is then the error's, as sent; its `retryable` is not read | commands: same code, not retryable |
| `MANAGER_HTTP_ERROR` | no | `internal.manager_http_error` | A Manager API non-2xx whose body is not an `ErrorResponse`. The body is not kept. A JSON body of another shape gets `context.json_path` and `context.decode_error`, as below; any other body leaves `context` null | commands: `MANAGER_HTTP_ERROR`, not retryable |
| `PLATFORM_HTTP_ERROR` | on 408, 429 and 5xx | `internal.platform_http_error` | A Platform API non-2xx whose body is not an API error. The body is not kept. A JSON body of another shape gets `context.json_path` and `context.decode_error`, as below; any other body leaves `context` null | commands: `PLATFORM_HTTP_ERROR` (bootstrap, same retry rule). Rust client: `UNEXPECTED_RESPONSE` (retryable on 5xx). Speakeasy: `AlienDefaultError`, no code |
| `COMMUNICATION_ERROR` | yes | `internal.send`, `internal.reply` | A Platform or Manager request could not be sent, timed out, or its response body could not be read (`http_status` is then the response's status). `context.method` is the request's method. A timeout is the client's (`timeoutMs`, [api-shape decision 9](api-shape.md#decisions)) or BAML's default of 5 minutes: `... failed: timed out after 300 ms (timeout)` | Rust client: `COMMUNICATION_ERROR` (send, a timeout included) and `RESPONSE_BODY_ERROR` (body read). commands: `COMMAND_BOOTSTRAP_FAILED` (bootstrap only). Speakeasy: `ConnectionError` and `RequestTimeoutError`, no code |
| `MALFORMED_RESPONSE` | no | `internal.decode_body` | A 2xx body that is not JSON or does not match the expected shape; `http_status` is the 2xx status. `context.json_path` is where in the body decoding failed (`.items[3].status`; absent when the body is not JSON or does not match at its root), `context.decode_error` the decoder's whole message. The message gives `json_path` and the first line of `decode_error`, cut at 200 characters | commands: `MALFORMED_RESPONSE`. Rust client: `INVALID_RESPONSE_PAYLOAD`. Speakeasy: `ResponseValidationError`, no code |
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
message keeps its first 200 and `context.decode_error` all of it.

The setup helper validates in this order: template URL, region, stack name,
parameters. It reports the first problem. Regions follow the shape of the
commercial AWS partition, so regions AWS launches later work without a change;
GovCloud, ISO and European Sovereign Cloud regions do not have that shape, and
China regions, which do, are rejected by name. Digits are `[0-9]` because
BAML's `\d` also matches non-ASCII digits.

## The server's fields and the source chain

A Platform API error body is Alien's own error, serialized. Besides `code`,
`message` and `retryable`, the SDK keeps:

| Field | From the body's | Notes |
|---|---|---|
| `hint` | `hint` | The server's remediation hint |
| `request_id` | `requestId` | Only the body is read; the `x-request-id` response header is not |
| `internal` | `internal` | The server's flag, as sent; null when the body has none |
| `source` | `source` | The error that caused this one, itself an `AlienError` with its own `source` |

A Manager error body gives the same four when it has them. All four are null
on the errors the SDK raises itself (`COMMUNICATION_ERROR`,
`MALFORMED_RESPONSE`, `PLATFORM_HTTP_ERROR` and the rest of the table above):
a body that is not the API's error body is not read for them.

**How `source` is read** (`internal.error_chain`). Each level is read on its
own and never fails the call, so a `source` that is not an error cannot change
the code of the error that carries it:

- A `source` that is not a JSON object is no source (`null`).
- A level without a string `code` gets `NESTED_ERROR`, without a string
  `message` gets `Nested error`, and without a boolean `retryable` is not
  retryable. A `hint`, `requestId` or `internal` of the wrong type is null.
- A level's `http_status` is its own `httpStatusCode` when that is an integer
  from 100 to 599, and null otherwise (`70000`, `-1`, `404.5`, `"404"`). Only
  the thrown error's `http_status` is the response's status.
- The chain is kept 8 levels deep below the thrown error. A ninth level is
  dropped: the eighth has no `source`. Alien's own chains are two or three
  levels deep.
- A level's `context` is its own `context`, exactly as sent.

The defaults are those of `parse_source_error` in Alien's Rust client. That
function reads only `code`, `message`, `context`, `retryable` and `source` on a
nested level; this SDK also reads `hint`, `httpStatusCode`, `requestId` and
`internal` there, so a level keeps everything the server sent.

**The language's cause chain follows `source`, and only `source`.** Whatever
walks an error's causes (a traceback, Sentry's chained exceptions,
`errors.As`, `anyhow`'s `{:#}`) walks the server's chain. The BAML runtime's
own error, which carries the failure across the bridge, is the same failure
and not a cause of it: it is in no chain.

## Context

`context` is JSON, the built-in `json` type: any JSON value, or null.

**A server's context is passed through exactly as sent.** Whatever the error
body's `context` holds is what the error's `context` holds: an object, a
string, a number, a boolean or an array, with its nesting, its nulls and the
types of its values (`1` stays a number, `"1"` a string). It is not turned
into text, not wrapped, and the SDK adds no key of its own to it. That is true
of the thrown error and of every level of its `source` chain. What it
contains is the server's business: the SDK does not look inside.

**The SDK's own contexts are small objects, or null.** They never hold the
request path, a query string or an excerpt of a response body: a path can
carry a credential (an invitation token is a path segment) and so can a body.
The message still names the path. `json_path` says where in a body decoding
failed, so it can name keys of that body (`.bindings["files"].region`), and
`decode_error` is the decoder's message.

| Code | `context` |
|---|---|
| *a server's code* (Platform) | The body's `context` as sent; null when the body has none, or a null one |
| *a server's code* (Manager) | The body's `context` as sent when it has one; else `{"details": <details>}` when the body has `details`; else null |
| `PLATFORM_HTTP_ERROR`, `MANAGER_HTTP_ERROR` | `{"json_path"?, "decode_error"}` when the body is JSON that is not the API's error body (where and why it is not); null for any other body |
| `COMMUNICATION_ERROR` | `{"method": <the request's method>}` |
| `MALFORMED_RESPONSE` | `{"json_path"?, "decode_error"}`: where in the body decoding failed, when that is known, and the decoder's whole message |
| `INVALID_REQUEST` | null |
| `SETUP_INVALID_TEMPLATE_URL` | `{"field": "templateUrl"}` |
| `SETUP_REGION_UNSUPPORTED` | `{"field": "region", "region": <the region>}` |
| `SETUP_INVALID_STACK_NAME` | `{"field": "stackName"}` |
| `SETUP_INVALID_PARAMETER` | `{"field": "parameters", "key": <the key>}` |
| `SETUP_TEMPLATE_UNAVAILABLE` | `{"project": <the project>}` |

**Numbers.** BAML reads a JSON integer as an integer and a number with a
fraction or exponent as a float, and each language gets them in its own
number types:

| Language | A context's value | Integers | `1` and `1.0` |
|---|---|---|---|
| TypeScript | Plain objects and arrays at every level (the runtime's own objects have no prototype; the facade copies them), `string`, `number`, `boolean`, `null` | A `number`, so one above 2^53 is rounded to the nearest `number`. One too large for BAML's 63-bit integer is a `bigint` instead, exact, which `JSON.stringify` refuses | The same `number` |
| Python | `dict`, `list`, `str`, `int`, `float`, `bool`, `None`, as `json.loads` gives them. The type is `typing.Any`, which is what the generated SDK declares for `json`; the value is stored as given | `int`, exact at any size | `int` and `float` |
| Go | `map[string]any`, `[]any`, `string`, `float64`, `bool`, `nil`, as `encoding/json` decodes into an `any` | Every number is a `float64`, so an integer above 2^53 is rounded, whatever its size | Both `float64(1)` |
| Rust | `serde_json::Value` (`alien_sdk::Json`). A null context is `None` | `i64` or `u64`, exact. An integer outside both ranges has no `serde_json::Value` form: the call then returns `Error::Runtime`, not the `AlienError`, wherever in a context or a source level it sits ([B19](gaps.md#b19)) | An integer and an `f64`: not equal |

## What changed for callers

`context` used to be a map of strings that the SDK filled. Code that read it
changes as follows:

| Before | Now |
|---|---|
| `context["details"]`, JSON text of the server's context, to be parsed | `context` is the server's context itself. Do not parse it |
| `context["hint"]`, `context["requestId"]` | The fields `hint` and `request_id` (TypeScript `requestId`, Go `RequestID`) |
| `context["path"]` | Gone. The caller knows which call it made; the message names the path |
| `context["body"]`, the first 1,000 characters of an error body that was not the API's | Gone. `PLATFORM_HTTP_ERROR` and `MANAGER_HTTP_ERROR` keep the status, and `json_path` and `decode_error` when the body was JSON |
| A Manager error's `context["details"]` | Still there when the body has no `context`; a body's `context` replaces it |
| `context` always an object of strings | Any JSON value. Check its type before indexing a server's context |
| A `source` level's `context["details"]` | That level's context itself |
| Types: TypeScript `Record<string, string>`, Python `dict[str, str]`, Go `map[string]string`, Rust `HashMap<String, String>` | TypeScript `Json` (exported), Python `Any`, Go `any`, Rust `Json` (`serde_json::Value`) |

`context.method`, `context.json_path`, `context.decode_error` and the keys of
the `SETUP_*` errors are unchanged.

## Per language

| Language | What you catch | Fields | Cause chain |
|---|---|---|---|
| TypeScript | `AlienError` (extends `Error`) | `code`, `message`, `retryable`, `statusCode`, `hint`, `requestId` (named as in Alien's TypeScript SDK), `internal`, `source`, `context` (`Json | null`; the package exports the `Json` type). `null` when not set | `cause` is `source`. Without a source the error has no `cause` property (`"cause" in err` is false) |
| Python | `alien_sdk.AlienError` (an `Exception`; `str()` is `"<code>: <message>"`) | `code`, `message`, `retryable`, `http_status`, `context` (`Any`), `hint`, `request_id`, `source`, `internal`, also the constructor's arguments in that order. `None` when not set | `__cause__` is `source`, set by the constructor, so an error that is built directly, unpickled or copied is chained too. The runtime's error is neither `__cause__` nor `__context__` |
| Go | `*alien.AlienError` via `errors.As` | `Code`, `Message`, `Retryable`, `HTTPStatus`, `Context` (`any`), `Hint` and `RequestID` (`""` when not set), `Source` and `Internal` (`nil` when not set). `context.Canceled` and `context.DeadlineExceeded` pass through unchanged; a generated call not wrapped in `alien.Do` or `alien.Err` returns `*baml_go.BamlError` | `Unwrap()` returns `Source`, and `nil` when there is none, so `errors.As` and `errors.Is` reach the inner errors. There is no `Cause` field |
| Rust | `alien_sdk::Error::Alien(AlienError)` | `code`, `message`, `retryable`, `http_status_code` (`Option<u16>`, named as in `alien_error::AlienError`), `context` (`Option<Json>`), `hint`, `request_id`, `source` (`Option<Box<AlienError>>`), `internal`. The struct is `#[non_exhaustive]`: read its fields, do not build it or match on all of them. `Error::Runtime` holds the `baml_bridge` error, BAML trace included. A `json` value holding an integer outside the `i64`/`u64` range, in a response or in an error's context, fails the call with `Error::Runtime` ([B19](gaps.md#b19)) | `std::error::Error::source()` returns `source`, on `AlienError` and on the `Error::Alien` that holds it. `Display` stays `"<code>: <message>"` and does not print the chain |

The four facades expose the same fields because each is checked against the
generated class: Rust destructures it without `..`, so a field BAML's
`AlienError` gains does not compile until it is mapped; the Python, TypeScript
and Go suites compare the generated class's fields with what the facade maps.

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

The frame in which `baml_bridge` runs a call keeps the call's arguments, the
client among them, in local variables ([gaps B23](gaps.md#b23)). Error
reporters that record local variables (Sentry's Python SDK by default) record
every frame of an exception's traceback, and of every exception chained to it
through `__cause__` or `__context__`.

A Python `AlienError` does not lead to that frame. The facade catches the
runtime's error, builds the `AlienError` from it, and raises it only after the
`except` block has ended (`alien_sdk/_errors.py`), so the runtime's error is
neither its `__cause__` nor its `__context__`, and nothing keeps it alive. The
traceback of an `AlienError` has the caller's frames and two of the facade's;
no local in them shows the API key (`tests/test_facade.py`).

An error that is not an `AlienError` passes through as the runtime raised it,
bridge frame included: `baml_bridge.BamlCancelledError` for a per-call
timeout, `BamlPanic`. That frame holds the client, not its fields: what a
reporter records is the client's `repr()`, and the facade's clients leave the
key out of theirs. With sentry-sdk 2.73.0 and its default settings, the event
for a failed call holds no API key, for an `AlienError` and for a per-call
timeout alike. To record no local variables at all:

```python
sentry_sdk.init(dsn, include_local_variables=False)
```
