// Runtime helpers for the generated operations (ns_platform, ns_manager) and
// the hand-written namespaces. BAML has no visibility modifiers yet
// (docs/gaps.md#b6), so these are exported by every generated SDK; the language
// facades do not re-export them.

/// Alien's Platform API error body (`APIError`), reduced to the fields the SDK
/// keeps.
class ApiErrorBody {
    code: string,
    message: string,
    retryable: bool?,
    hint: string?,
    requestId: string?,
    context: json?,
}

/// The Manager API error body (`ErrorResponse`).
class ManagerErrorBody {
    code: string,
    message: string,
    details: string?,
}

// ---------------------------------------------------------------------------
// URLs

/// `s` percent-encoded byte by byte: every UTF-8 byte outside the RFC 3986
/// unreserved set (`A-Z a-z 0-9 - . _ ~`) becomes `%XX` with uppercase hex.
///
/// This loop runs for every character of every path and query parameter, so
/// it joins strings with `+`: in BAML 0.23.0 a backtick string costs about
/// 2 µs each time it is built, `+` almost nothing.
function percent_encode(s: string) -> string {
    let hex = "0123456789ABCDEF";
    let out = "";
    for (let c in s.chars()) {
        if (c.is_ascii_alphanumeric() || c == "-" || c == "." || c == "_" || c == "~") {
            out = out + c;
        } else {
            for (let b in c.to_utf8().to_array()) {
                let hi = b / 16;
                let lo = b % 16;
                out = out + "%" + hex.slice(hi, hi + 1) + hex.slice(lo, lo + 1);
            }
        }
    }
    out
}

/// One path parameter, percent-encoded. "", "." and ".." are refused: they
/// would make the request reach a different endpoint (URL parsers resolve `..`
/// even when percent-encoded).
function path_segment(value: string) -> string throws root.errors.AlienError {
    match (value) {
        "" | "." | ".." => throw invalid_request(`a path parameter cannot be "${value}"`),
        _ => percent_encode(value),
    }
}

/// `base` without trailing slashes, followed by `path`. A query string on the
/// base stays at the end, before the operation's own query parameters, as in
/// `@alienplatform/commands`: `https://h/v1?token=x` and `/commands?a=1` give
/// `https://h/v1/commands?token=x&a=1`.
function join_url(base: string, path: string) -> string {
    let trimmed = base;
    let base_query = "";
    if let at: int = base.index_of("?") {
        trimmed = base.slice(0, at);
        base_query = base.slice(at + 1, base.length());
    }
    while (trimmed.ends_with("/")) {
        trimmed = trimmed.slice(0, trimmed.length() - 1);
    }
    match (base_query) {
        "" => `${trimmed}${path}`,
        _ => match (path.index_of("?")) {
            let at: int => `${trimmed}${path.slice(0, at)}?${base_query}&${path.slice(at + 1, path.length())}`,
            null => `${trimmed}${path}?${base_query}`,
        },
    }
}

/// The value of a query parameter. `null` leaves the parameter out.
type QueryValue = string | int | float | string[] | null

/// `?a=1&b=2` from the parameters that are set, in order; "" when none is.
/// Form style, exploded (OpenAPI's default for a query): a string is
/// percent-encoded, a list repeats the name for each element (an empty list
/// is left out), and a float is written as JavaScript writes it
/// (`number_text`).
function query_string(params: map<string, QueryValue>) -> string {
    let parts: string[] = [];
    for (let name in params.keys()) {
        match (params[name]) {
            null => {},
            let text: string => {
                parts.push(`${name}=${percent_encode(text)}`);
            },
            let n: int => {
                parts.push(`${name}=${n}`);
            },
            let x: float => {
                parts.push(`${name}=${percent_encode(number_text(x))}`);
            },
            let items: string[] => {
                for (let item in items) {
                    parts.push(`${name}=${percent_encode(item)}`);
                }
            },
        }
    }
    if (parts.length() == 0) { "" } else { `?${parts.join("&")}` }
}

/// A number as JavaScript (and so Alien's TypeScript SDK) writes it in a URL:
/// a whole number without a fraction (`2`, not `2.0`), so a `number`
/// parameter such as `limit` reads the same to the server.
function number_text(v: float) -> string {
    // Below 2^53 every whole float is exact as an int.
    if (v.is_finite() && v.fract() == 0.0 && v.abs() < 9007199254740992.0) {
        let whole = v.itrunc() catch (e) {
            _ => return string.from(v),
        };
        return string.from(whole);
    }
    string.from(v)
}

// ---------------------------------------------------------------------------
// Errors

/// A request the SDK refuses to send: retrying cannot help.
function invalid_request(message: string) -> root.errors.AlienError {
    root.errors.AlienError {
        code: "INVALID_REQUEST",
        message: message,
        retryable: false,
        http_status: null,
        context: null,
    }
}

/// The error for a request that could not be sent, or whose response body
/// could not be read (`status` is then the response's).
function communication_error(method: string, path: string, status: int?, detail: string) -> root.errors.AlienError {
    root.errors.AlienError {
        code: "COMMUNICATION_ERROR",
        message: `HTTP request ${method} ${path} failed: ${detail}`,
        retryable: true,
        http_status: status,
        context: { "method": method, "path": path },
    }
}

/// The error for a 2xx response whose body does not decode. `json_path` is
/// where in the body decoding failed (`baml.json.DecodeError.path`, such as
/// `.items[3].status`; "" when BAML gives none) and `reason` the decoder's
/// message. The message leads with both; the context keeps `reason` whole
/// (`decode_error`), however long, and `json_path` when there is one.
function malformed_response(status: int, path: string, json_path: string, reason: string) -> root.errors.AlienError {
    let at = match (json_path) {
        "" => "",
        _ => ` at ${json_path}`,
    };
    root.errors.AlienError {
        code: "MALFORMED_RESPONSE",
        message: `Response from ${path} did not match the expected shape${at}: ${decode_summary(reason)}`,
        retryable: false,
        http_status: status,
        context: with_decode_error({ "path": path }, json_path, reason),
    }
}

/// `context` plus where and why a body did not decode: `json_path` when it
/// is known, and `decode_error`, the decoder's whole message.
function with_decode_error(context: map<string, string>, json_path: string, reason: string) -> map<string, string> {
    if (json_path != "") {
        context["json_path"] = json_path;
    }
    context["decode_error"] = reason;
    context
}

/// A decoder message short enough for an error message: its first line, cut
/// at 200 characters. A message that names every member of a union a value
/// did not match can run to kilobytes; a shortened one says where the rest is.
function decode_summary(reason: string) -> string {
    let line = reason.split("\n")[0];
    if (line.length() > 200) {
        line = `${line.slice(0, 200)}…`;
    }
    if (line == reason) { reason } else { `${line} (the whole message is in context.decode_error)` }
}

/// The context of an error response whose body is not the API's error body:
/// its first 1,000 characters, and, when the body is JSON of another shape,
/// where and why it did not decode as the error body (`with_decode_error`).
function error_body_context(path: string, body: string, mismatch: baml.json.DecodeError?) -> map<string, string> {
    let context: map<string, string> = { "path": path, "body": body.slice(0, 1000) };
    match (mismatch) {
        let e: baml.json.DecodeError => with_decode_error(context, e.path, e.message),
        null => context,
    }
}

/// A non-2xx Platform API response. Alien's API error body keeps its code,
/// message and retryable flag, and puts its request ID, hint and context (as
/// JSON text, `details`) in the error's context; anything else becomes
/// `PLATFORM_HTTP_ERROR`, retryable on 408, 429 and 5xx.
function platform_http_error(status: int, path: string, body: string) -> root.errors.AlienError {
    let mismatch: baml.json.DecodeError? = null;
    let api = baml.json.decode<ApiErrorBody>(body) catch (e) {
        baml.json.DecodeError => {
            mismatch = e;
            null
        },
        baml.json.ParseError => null,
    };
    match (api) {
        let parsed: ApiErrorBody => {
            let context: map<string, string> = { "path": path };
            if let id: string = parsed.requestId {
                context["requestId"] = id;
            }
            if let hint: string = parsed.hint {
                context["hint"] = hint;
            }
            if (parsed.context != null) {
                // Parsed from JSON text, so it always has a JSON form: no NaN, no cycle.
                context["details"] = baml.json.stringify(parsed.context) catch (e) {
                    _ => throw malformed_response(status, path, e.path, e.message),
                };
            }
            root.errors.AlienError {
                code: parsed.code,
                message: parsed.message,
                retryable: parsed.retryable ?? false,
                http_status: status,
                context: context,
            }
        },
        null => root.errors.AlienError {
            code: "PLATFORM_HTTP_ERROR",
            message: `Platform API returned HTTP ${status} for ${path}`,
            retryable: status == 408 || status == 429 || status >= 500,
            http_status: status,
            context: error_body_context(path, body, mismatch),
        },
    }
}

/// A non-2xx Manager API response. The Manager's error body keeps its code and
/// message (`details` goes to context); anything else becomes
/// `MANAGER_HTTP_ERROR`. Neither is retryable, as in `@alienplatform/commands`.
function manager_http_error(status: int, path: string, body: string) -> root.errors.AlienError {
    let mismatch: baml.json.DecodeError? = null;
    let api = baml.json.decode<ManagerErrorBody>(body) catch (e) {
        baml.json.DecodeError => {
            mismatch = e;
            null
        },
        baml.json.ParseError => null,
    };
    match (api) {
        let parsed: ManagerErrorBody => {
            let context: map<string, string> = { "path": path };
            if let details: string = parsed.details {
                context["details"] = details;
            }
            root.errors.AlienError {
                code: parsed.code,
                message: parsed.message,
                retryable: false,
                http_status: status,
                context: context,
            }
        },
        null => root.errors.AlienError {
            code: "MANAGER_HTTP_ERROR",
            message: `Manager returned HTTP ${status} for ${path}`,
            retryable: false,
            http_status: status,
            context: error_body_context(path, body, mismatch),
        },
    }
}

// ---------------------------------------------------------------------------
// Requests

/// The headers of a request: `accept`, then the operation's `headers` whose
/// value is set, then `authorization` when there is a `token`.
function request_headers(headers: map<string, string?>, token: string?) -> map<string, string> {
    let all: map<string, string> = { "accept": "application/json" };
    for (let name in headers.keys()) {
        if let value: string = headers[name] {
            all[name] = value;
        }
    }
    if let t: string = token {
        all["authorization"] = `Bearer ${t}`;
    }
    all
}

/// Sends one request and returns the response, whatever its status.
/// `timeout_ms` limits the whole exchange, reading the response body included
/// (the client's `timeout_ms`; `null` is BAML's default of 5 minutes, zero or
/// negative is no limit).
function send(
    base_url: string,
    method: string,
    path: string,
    token: string?,
    headers: map<string, string?>,
    body: string?,
    timeout_ms: int?,
) -> baml.http.Response throws root.errors.AlienError {
    let all = request_headers(headers, token);
    if (!base_url.starts_with("https://") && !base_url.starts_with("http://")) {
        throw invalid_request(`the base URL must start with https:// or http://, got "${base_url}"`);
    }
    if (base_url.includes("#")) {
        throw invalid_request(`the base URL cannot have a fragment, got "${base_url}"`);
    }
    let request = baml.http.Request.from_url(join_url(base_url, path)).set_method(method);
    if let text: string = body {
        all["content-type"] = "application/json";
        request = request.set_body(text);
    }
    request = request.set_headers(all);
    if let ms: int = timeout_ms {
        request = request.set_timeout(baml.time.Duration.from_milliseconds(ms));
    }
    baml.http.send(request) catch (e) {
        baml.errors.Io => throw communication_error(method, path, null, e.message),
        baml.errors.Timeout => throw communication_error(method, path, null, timed_out("timed out", e)),
        baml.errors.InvalidArgument => throw invalid_request(`HTTP request ${method} ${path} is invalid: ${e.message}`),
    }
}

/// `what`, the limit that elapsed when BAML reports it, and which one it was:
/// "timed out after 300 ms (timeout)", or `(connect_timeout)` for connecting.
function timed_out(what: string, e: baml.errors.Timeout) -> string {
    let after = match (e.duration) {
        let limit: baml.time.Duration => ` after ${limit.to_milliseconds()} ms`,
        null => "",
    };
    `${what}${after} (${e.timeout_type})`
}

/// A 2xx response's status and body, and the request's path and query, as
/// errors name it.
class Reply {
    status: int,
    text: string,
    path: string,
}

/// Reads a response. A non-2xx status throws the error `manager_http_error` or
/// `platform_http_error` maps it to.
function reply(
    response: baml.http.Response,
    method: string,
    path: string,
    manager: bool,
) -> Reply throws root.errors.AlienError {
    let text = response.text() catch (e) {
        baml.errors.Io => throw communication_error(method, path, response.status_code, e.message),
        baml.errors.Timeout => throw communication_error(method, path, response.status_code, timed_out("timed out reading the response", e)),
    };
    if (!response.ok()) {
        throw if (manager) {
            manager_http_error(response.status_code, path, text)
        } else {
            platform_http_error(response.status_code, path, text)
        };
    }
    Reply { status: response.status_code, text: text, path: path }
}

/// Decodes a JSON body into `T`, throwing `MALFORMED_RESPONSE` on failure.
/// `decode` honors `baml.FromJson` overrides (for wire names that are BAML
/// reserved words, docs/gaps.md#b14).
function decode_body<T>(text: string, status: int, path: string) -> T throws root.errors.AlienError {
    baml.json.decode<T>(text) catch (e) {
        baml.json.ParseError => throw malformed_response(status, path, "", e.message),
        baml.json.DecodeError => throw malformed_response(status, path, e.path, e.message),
    }
}

/// `null` for a 204 or any 2xx with an empty body; the decoded body otherwise.
function decode_optional_body<T>(text: string, status: int, path: string) -> T? throws root.errors.AlienError {
    if (status == 204 || text == "") {
        return null;
    }
    decode_body<T>(text, status, path)
}

/// Decodes a reply's JSON body with `decode`, throwing `MALFORMED_RESPONSE`
/// when it does not fit. Operations returning an open union pass a `decode`
/// that picks the union's member by its tag (tools/openapi2baml/DESIGN.md,
/// "Open unions").
function decode_reply<T>(r: Reply, decode: (baml.json.json) -> T throws baml.json.DecodeError) -> T throws root.errors.AlienError {
    let body = decode_body<baml.json.json>(r.text, r.status, r.path);
    decode(body) catch (e) {
        baml.json.DecodeError => throw malformed_response(r.status, r.path, e.path, e.message),
    }
}

// Platform API operations: always authenticated with the client's API key.
// Each sends `method` to `path` followed by the query string of `query`
// (`query_string`), with `headers` (one whose value is `null` is not sent) and
// `body` (JSON text, from `json_body`; `null` sends none).

/// The 2xx reply to a Platform API request.
function platform_reply(
    client: root.platform.PlatformClient,
    method: string,
    path: string,
    query: map<string, QueryValue>,
    headers: map<string, string?>,
    body: string?,
) -> Reply throws root.errors.AlienError {
    let target = `${path}${query_string(query)}`;
    reply(send(client.base_url, method, target, client.api_key, headers, body, client.timeout_ms), method, target, false)
}

/// A Platform API request whose response is JSON, decoded as `T`.
function platform_json<T>(
    client: root.platform.PlatformClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T throws root.errors.AlienError {
    let r = platform_reply(client, method, path, query, headers, body);
    decode_body<T>(r.text, r.status, r.path)
}

/// A Platform API request whose response is JSON decoded as `T`, or empty
/// (`null`): `decode_optional_body`.
function platform_optional_json<T>(
    client: root.platform.PlatformClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T? throws root.errors.AlienError {
    let r = platform_reply(client, method, path, query, headers, body);
    decode_optional_body<T>(r.text, r.status, r.path)
}

/// A Platform API request whose response body is ignored.
function platform_empty(
    client: root.platform.PlatformClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> null throws root.errors.AlienError {
    platform_reply(client, method, path, query, headers, body);
    null
}

/// A Platform API request whose JSON response `decode` turns into a `T`:
/// `decode_reply`.
function platform_decoded<T>(
    client: root.platform.PlatformClient,
    method: string,
    path: string,
    decode: (baml.json.json) -> T throws baml.json.DecodeError,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T throws root.errors.AlienError {
    decode_reply<T>(platform_reply(client, method, path, query, headers, body), decode)
}

// Manager API operations: authenticated with the client's token whenever it
// is set, as `@alienplatform/commands` does, although the Manager spec declares
// security on only some operations. Arguments as for the Platform API.

/// The 2xx reply to a Manager API request.
function manager_reply(
    client: root.manager.ManagerClient,
    method: string,
    path: string,
    query: map<string, QueryValue>,
    headers: map<string, string?>,
    body: string?,
) -> Reply throws root.errors.AlienError {
    let target = `${path}${query_string(query)}`;
    reply(send(client.base_url, method, target, client.token, headers, body, client.timeout_ms), method, target, true)
}

/// A Manager API request whose response is JSON, decoded as `T`.
function manager_json<T>(
    client: root.manager.ManagerClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T throws root.errors.AlienError {
    let r = manager_reply(client, method, path, query, headers, body);
    decode_body<T>(r.text, r.status, r.path)
}

/// A Manager API request whose response is JSON decoded as `T`, or empty
/// (`null`): `decode_optional_body`.
function manager_optional_json<T>(
    client: root.manager.ManagerClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T? throws root.errors.AlienError {
    let r = manager_reply(client, method, path, query, headers, body);
    decode_optional_body<T>(r.text, r.status, r.path)
}

/// A Manager API request whose response body is ignored.
function manager_empty(
    client: root.manager.ManagerClient,
    method: string,
    path: string,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> null throws root.errors.AlienError {
    manager_reply(client, method, path, query, headers, body);
    null
}

/// A Manager API request whose JSON response `decode` turns into a `T`:
/// `decode_reply`.
function manager_decoded<T>(
    client: root.manager.ManagerClient,
    method: string,
    path: string,
    decode: (baml.json.json) -> T throws baml.json.DecodeError,
    query: map<string, QueryValue> = {},
    headers: map<string, string?> = {},
    body: string? = null,
) -> T throws root.errors.AlienError {
    decode_reply<T>(manager_reply(client, method, path, query, headers, body), decode)
}

// ---------------------------------------------------------------------------
// Request bodies

/// `value` as JSON text. `baml.json.stringify` honors `baml.ToJson` overrides:
/// request classes implement it to omit unset optional fields (as Alien's SDKs
/// do) and to write wire names that are BAML reserved words (docs/gaps.md#b14).
/// Values inside `json` fields are written as given, `null`s included.
function encode_body<T>(value: T) -> string throws root.errors.AlienError {
    baml.json.stringify(value) catch (e) {
        _ => throw invalid_request(`the request body cannot be encoded as JSON: ${e.message}`),
    }
}

/// An operation's request body: `value` as JSON text (`encode_body`), or
/// `null`, which sends no body.
function json_body<T>(value: T?) -> string? throws root.errors.AlienError {
    match (value) {
        null => null,
        let v: T => encode_body<T>(v),
    }
}
