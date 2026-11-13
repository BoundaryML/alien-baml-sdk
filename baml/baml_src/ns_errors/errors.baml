/// The one error every public Alien SDK function throws.
///
/// Mirrors Alien's error contract (`AlienErrorData`): a machine-readable
/// `code`, a human-readable `message`, whether retrying can help, the HTTP
/// status when one was involved, string context for debugging, and the
/// server's `hint`, request ID, `internal` flag and `source` chain when the
/// error came from an API error body.
///
/// Codes this SDK raises itself:
///
/// | Code | Retryable | Raised when |
/// |---|---|---|
/// | `PLATFORM_HTTP_ERROR` | 408, 429, >= 500 | A Platform API non-2xx whose body is not an API error |
/// | `MANAGER_HTTP_ERROR` | no | A Manager API non-2xx whose body is not its error response |
/// | `COMMUNICATION_ERROR` | yes | The request could not be sent or the response could not be read |
/// | `MALFORMED_RESPONSE` | no | A 2xx body that is not JSON or does not match the expected shape (`context.json_path`: where; `context.decode_error`: why) |
/// | `INVALID_REQUEST` | no | A request the SDK refuses to send (unencodable body, empty/`.`/`..` path parameter, base URL without an http scheme or with a fragment) |
/// | `SETUP_INVALID_TEMPLATE_URL` | no | `templateUrl` is not an `https://` URL |
/// | `SETUP_REGION_UNSUPPORTED` | no | The region does not match `^[a-z]{2}-[a-z]+-[0-9]+$`, or is a China region (`cn-*`) |
/// | `SETUP_INVALID_STACK_NAME` | no | The stack name does not match `^[A-Za-z][A-Za-z0-9-]{0,127}$` |
/// | `SETUP_INVALID_PARAMETER` | no | A parameter key is empty or not ASCII alphanumeric |
/// | `SETUP_TEMPLATE_UNAVAILABLE` | no | The project returned no AWS template URLs |
///
/// A Platform API non-2xx whose body is Alien's API error (`{code, message,
/// ...}`) keeps the server's `code`, `message` and `retryable` (for example
/// `SERVICE_UNAVAILABLE`), with `http_status` set to the response status. A
/// Manager API error response keeps its `code` and `message` and is never
/// retryable. Both also keep the body's `hint`, `requestId`, `internal` and
/// `source`. The errors this SDK raises itself (the table above) have none of
/// the four.
class AlienError {
    code: string,
    message: string,
    retryable: bool,
    /// The response's status. On a `source` level: that level's own
    /// `httpStatusCode` when it is an integer from 100 to 599, else null.
    http_status: int?,
    /// Strings for debugging: `path`, and for a server error its request ID
    /// (`requestId`), hint (`hint`) and context as JSON text (`details`). On
    /// a `source` level: only `details`, that level's context as JSON text.
    context: map<string, string>?,
    /// The server's remediation hint.
    hint: string?,
    /// The server's request ID: the error body's `requestId`.
    request_id: string?,
    /// The error that caused this one: the body's `source`, read leniently
    /// (`internal.error_chain`). At most 8 levels below the thrown error;
    /// deeper levels are dropped.
    source: AlienError?,
    /// The body's `internal` flag: true for an error the server says should
    /// not be shown to end users.
    internal: bool?,
}
