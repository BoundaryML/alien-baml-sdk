// Tests for the runtime helpers every generated operation calls (http.baml).
// None of them sends a request: the network paths run in the conformance suites.

/// The code (plus " retryable") `path_segment(value)` throws, or "no error".
function error_of_path_segment(value: string) -> string {
    { path_segment(value); "no error" } catch (e) {
        root.errors.AlienError => if (e.retryable) { `${e.code} retryable` } else { e.code },
    }
}

function error_of_send(base_url: string) -> string {
    { send(base_url, "GET", "/v1/x", null, {}, null, null); "no error" } catch (e) {
        root.errors.AlienError => if (e.retryable) { `${e.code} retryable` } else { e.code },
    }
}

/// An error's context as JSON text: `null` when it has none.
function context_of(e: root.errors.AlienError) -> string {
    baml.json.stringify(e.context) catch (e) {
        _ => "not JSON",
    }
}

/// The string at `key` of an error's context, or null.
function context_string(e: root.errors.AlienError, key: string) -> string? {
    json_string(baml.json.field(e.context, key))
}

/// The context, as JSON text, of a Platform API error whose body has this
/// JSON `context`.
function platform_error_context(context: string) -> string {
    context_of(platform_http_error(400, "/v1/x", `{"code":"BAD","message":"m","context":${context}}`))
}

/// The same for the first `source` level of a Platform API error.
function source_context(context: string) -> string {
    match (platform_error_source(`{"code":"INNER","message":"m","context":${context}}`)) {
        let level: root.errors.AlienError => context_of(level),
        null => "no source",
    }
}

/// A Platform API error body, code `TOP`, whose `source` chain is `depth`
/// levels deep: level n has the code `L<n>`.
function chained_body(depth: int) -> string {
    let source = "null";
    let level = depth;
    while (level >= 1) {
        source = `{"code":"L${level}","message":"level ${level}","source":${source}}`;
        level = level - 1;
    }
    `{"code":"TOP","message":"m","source":${source}}`
}

/// The codes of an error and of its `source` chain, outermost first:
/// `>TOP>L1>L2`; "" for no error.
function chain_codes(e: root.errors.AlienError?) -> string {
    match (e) {
        null => "",
        let level: root.errors.AlienError => `>${level.code}${chain_codes(level.source)}`,
    }
}

/// The `source` of a Platform API error whose body has this JSON `source`.
function platform_error_source(source: string) -> root.errors.AlienError? {
    platform_http_error(400, "/v1/x", `{"code":"BAD","message":"m","source":${source}}`).source
}

/// Whether an error has none of the fields only a server's error body sets.
function has_no_server_fields(e: root.errors.AlienError) -> bool {
    e.hint == null && e.request_id == null && e.source == null && e.internal == null
}

/// A response as BAML's HTTP client hands it over, with header names in
/// lowercase, built without a request.
function response_with(status: int, headers: map<string, string>, body: string) -> baml.http.Response {
    baml.http.Response.new(status, headers, body.to_utf8())
}

/// The error `reply` throws for this response to `GET /v1/x`; `INVALID_REQUEST`
/// "no error" when it throws none.
function reply_error(response: baml.http.Response, manager: bool) -> root.errors.AlienError {
    { reply(response, "GET", "/v1/x", manager); invalid_request("no error") } catch (e) {
        root.errors.AlienError => e,
    }
}

/// The request ID of the error `reply` throws for a non-2xx response with
/// this body, and the `x-request-id` header `header` when it is not null.
function request_id_of(body: string, header: string?, manager: bool) -> string? {
    let headers: map<string, string> = { "content-type": "application/json" };
    if let id: string = header {
        headers["x-request-id"] = id;
    }
    reply_error(response_with(400, headers, body), manager).request_id
}

/// The error of decoding a 200 response with this body and headers as a
/// `Manager`, as `platform_json` does: `reply`, then `decode_body`.
function decoded_reply_error(headers: map<string, string>, body: string) -> root.errors.AlienError {
    {
        let r = reply(response_with(200, headers, body), "GET", "/v1/x", false);
        decode_body<root.platform.Manager>(r.text, r.status, r.path, request_id = r.request_id);
        invalid_request("no error")
    } catch (e) {
        root.errors.AlienError => e,
    }
}

/// The error `decode_body` throws for this body, decoded as `T`.
function decode_error_of<T>(text: string) -> root.errors.AlienError {
    { decode_body<T>(text, 200, "/v1/x"); invalid_request("decoded") } catch (e) {
        root.errors.AlienError => e,
    }
}

/// A manager as the Platform API sends it, with this `managementConfigs`.
function manager_with_configs(configs: string) -> string {
    `{"id":"m1","name":"n","targets":[],"managementConfigs":${configs},"isSystem":false,"workspaceId":"w1","status":"healthy","managedDeploymentCount":0,"defaultProjectCount":0,"createdAt":"2026-10-01T00:00:00Z"}`
}

/// An event with this `data`, as the Platform API sends it.
function event_with_data(data: string) -> string {
    `{"id":"e1","data":${data},"state":"success","projectId":"p1","createdAt":"2026-10-01T00:00:00Z","workspaceId":"w1"}`
}

/// Which member an event's `data` decodes as, or the error code.
function event_data_member(data: string) -> string {
    let event = decode_body<root.platform.Event>(event_with_data(data), 200, "/v1/x") catch (e) {
        root.errors.AlienError => return e.code,
    };
    match (event.data) {
        let other: root.platform.EventDataOther => `${other.type} ${baml.json.to<string>(other.raw["type"])}`,
        let known: root.platform.DeploymentCheckRequestedEvent => `known ${known.deploymentId}`,
        _ => "another known member",
    }
}

testset "http helpers" {
    test "path parameters are percent-encoded" {
        assert.equal(path_segment("a b/c?d#é"), "a%20b%2Fc%3Fd%23%C3%A9");
        assert.equal(path_segment("mgr_01.x-~"), "mgr_01.x-~");
    }

    test "path parameters that would change the endpoint are refused" {
        assert.equal(error_of_path_segment(""), "INVALID_REQUEST");
        assert.equal(error_of_path_segment("."), "INVALID_REQUEST");
        assert.equal(error_of_path_segment(".."), "INVALID_REQUEST");
        assert.equal(error_of_path_segment("..."), "no error");
        assert.equal(error_of_path_segment("a..b"), "no error");
    }

    test "a base URL without an http scheme, or with a fragment, is refused before any request" {
        assert.equal(error_of_send("api.alien.dev"), "INVALID_REQUEST");
        assert.equal(error_of_send("ftp://127.0.0.1:9"), "INVALID_REQUEST");
        assert.equal(error_of_send(""), "INVALID_REQUEST");
        assert.equal(error_of_send("http://127.0.0.1:9/v1#x"), "INVALID_REQUEST");
    }

    test "query strings keep the set parameters, in order" {
        assert.equal(query_string({}), "");
        assert.equal(query_string({ "a": null, "b": null }), "");
        assert.equal(query_string({ "a": "x y&z", "b": null, "c": 3, "f": 1.5 }), "?a=x%20y%26z&c=3&f=1.5");
        assert.equal(query_string({ "z": "1", "a": "2" }), "?z=1&a=2");
        // Whole numbers as JavaScript writes them.
        assert.equal(query_string({ "f": 2.0 }), "?f=2");
        assert.equal(query_string({ "f": -3.0 }), "?f=-3");
        assert.equal(query_string({ "f": 0.25 }), "?f=0.25");
        assert.equal(query_string({ "n": -7 }), "?n=-7");
    }

    test "lists repeat the key and empty lists are left out" {
        assert.equal(query_string({ "s": ["a", "b&c"] }), "?s=a&s=b%26c");
        let none: string[] = [];
        assert.equal(query_string({ "s": none }), "");
        assert.equal(query_string({ "s": ["a", "b"], "t": "u" }), "?s=a&s=b&t=u");
    }

    test "join_url drops trailing slashes from the base" {
        assert.equal(join_url("http://127.0.0.1:1//", "/v1/x"), "http://127.0.0.1:1/v1/x");
        assert.equal(join_url("http://127.0.0.1:1", "/v1/x"), "http://127.0.0.1:1/v1/x");
    }

    test "join_url keeps a query string on the base after the path, before the operation's" {
        assert.equal(join_url("http://127.0.0.1:1/v1?token=x", "/commands"), "http://127.0.0.1:1/v1/commands?token=x");
        assert.equal(join_url("http://127.0.0.1:1/v1/?token=x", "/commands/c1"), "http://127.0.0.1:1/v1/commands/c1?token=x");
        assert.equal(
            join_url("http://127.0.0.1:1/v1?token=x&b=?", "/managers?workspace=w&limit=2"),
            "http://127.0.0.1:1/v1/managers?token=x&b=?&workspace=w&limit=2",
        );
        assert.equal(join_url("http://127.0.0.1:1/v1?", "/commands?a=1"), "http://127.0.0.1:1/v1/commands?a=1");
    }

    test "headers are sent only when set, and the token as a bearer authorization" {
        assert.equal(request_headers({ "if-match": "e1" }, null), { "accept": "application/json", "if-match": "e1" });
        assert.equal(request_headers({ "if-match": null }, null), { "accept": "application/json" });
        assert.equal(request_headers({}, "k1"), { "accept": "application/json", "authorization": "Bearer k1" });
    }

    test "platform API errors keep code, retryable flag, hint, request ID and context" {
        let e = platform_http_error(
            422,
            "/v1/x",
            `{"code":"VALIDATION_FAILED","message":"bad name","retryable":true,"hint":"use a shorter name","requestId":"req_1","context":{"field":"name"},"internal":false}`,
        );
        assert.equal(e.code, "VALIDATION_FAILED");
        assert.equal(e.message, "bad name");
        assert.equal(e.retryable, true);
        assert.equal(e.http_status, 422);
        assert.equal(e.hint, "use a shorter name");
        assert.equal(e.request_id, "req_1");
        assert.equal(e.internal, false);
        assert.equal(e.source, null);
        // The context is the body's, with nothing of the SDK's added.
        assert.equal(context_of(e), `{"field":"name"}`);
    }

    test "platform API errors keep the source chain, every field of every level" {
        let e = platform_http_error(
            500,
            "/v1/x",
            `{"code":"DEPLOY_FAILED","message":"deploy failed","retryable":false,"internal":true,"httpStatusCode":500,"source":{"code":"SANDBOX_REFUSED","message":"the sandbox refused","retryable":true,"hint":"ask an admin","requestId":"req_inner","internal":false,"httpStatusCode":403,"context":{"policy":"p1","limits":[1,2.5,null]},"source":{"code":"QUOTA","message":"quota exceeded","internal":true,"context":"cpu"}}}`,
        );
        assert.equal(chain_codes(e), ">DEPLOY_FAILED>SANDBOX_REFUSED>QUOTA");
        assert.equal(e.internal, true);
        assert.equal(e.hint, null);
        assert.equal(e.request_id, null);
        assert.equal(e.http_status, 500);
        let first = e.source ?? invalid_request("no first level");
        assert.equal(first.code, "SANDBOX_REFUSED");
        assert.equal(first.message, "the sandbox refused");
        assert.equal(first.retryable, true);
        assert.equal(first.http_status, 403);
        assert.equal(first.hint, "ask an admin");
        assert.equal(first.request_id, "req_inner");
        assert.equal(first.internal, false);
        // A level's context is its own, as sent.
        assert.equal(context_of(first), `{"policy":"p1","limits":[1,2.5,null]}`);
        let second = first.source ?? invalid_request("no second level");
        assert.equal(second.code, "QUOTA");
        assert.equal(second.message, "quota exceeded");
        assert.equal(second.retryable, false);
        assert.equal(second.http_status, null);
        assert.equal(second.hint, null);
        assert.equal(second.request_id, null);
        assert.equal(second.internal, true);
        assert.equal(context_of(second), `"cpu"`);
        assert.equal(second.source, null);
    }

    test "a source level without code, message or retryable takes the defaults" {
        let level = platform_error_source("{}") ?? invalid_request("no level");
        assert.equal(level.code, "NESTED_ERROR");
        assert.equal(level.message, "Nested error");
        assert.equal(level.retryable, false);
        assert.equal(level.http_status, null);
        assert.equal(level.context, null);
        assert.equal(has_no_server_fields(level), true);
        assert.equal(platform_error_source(`{"context":null}`)?.context, null);
    }

    test "a source that is not a JSON object is no source" {
        assert.equal(platform_error_source("null"), null);
        assert.equal(platform_error_source(`"boom"`), null);
        assert.equal(platform_error_source("7"), null);
        assert.equal(platform_error_source("true"), null);
        assert.equal(platform_error_source(`[{"code":"IN_A_LIST","message":"m"}]`), null);
        // The same one level down: the level above is kept.
        assert.equal(chain_codes(platform_error_source(`{"code":"L1","message":"m","source":[1]}`)), ">L1");
    }

    test "a source field of the wrong type takes its default, or null" {
        let level = platform_error_source(
            `{"code":7,"message":["m"],"retryable":"yes","hint":3,"requestId":{"id":"r"},"internal":"no","httpStatusCode":"404","source":"deeper"}`,
        ) ?? invalid_request("no level");
        assert.equal(level.code, "NESTED_ERROR");
        assert.equal(level.message, "Nested error");
        assert.equal(level.retryable, false);
        assert.equal(level.http_status, null);
        assert.equal(level.context, null);
        assert.equal(has_no_server_fields(level), true);
    }

    test "a source level's httpStatusCode is kept only when it is an integer from 100 to 599" {
        assert.equal(platform_error_source(`{"httpStatusCode":100}`)?.http_status, 100);
        assert.equal(platform_error_source(`{"httpStatusCode":404}`)?.http_status, 404);
        assert.equal(platform_error_source(`{"httpStatusCode":599}`)?.http_status, 599);
        assert.equal(platform_error_source(`{"httpStatusCode":99}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":600}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":70000}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":-1}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":0}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":404.5}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":404.0}`)?.http_status, null);
        assert.equal(platform_error_source(`{"httpStatusCode":123456789012345678901234567890}`)?.http_status, null);
        // The code of the level is still read.
        assert.equal(platform_error_source(`{"code":"L1","httpStatusCode":70000}`)?.code, "L1");
    }

    test "the source chain is kept 8 levels deep, and cut there" {
        assert.equal(max_source_depth(), 8);
        assert.equal(chain_codes(platform_http_error(500, "/v1/x", chained_body(0))), ">TOP");
        assert.equal(chain_codes(platform_http_error(500, "/v1/x", chained_body(1))), ">TOP>L1");
        assert.equal(chain_codes(platform_http_error(500, "/v1/x", chained_body(8))), ">TOP>L1>L2>L3>L4>L5>L6>L7>L8");
        assert.equal(chain_codes(platform_http_error(500, "/v1/x", chained_body(9))), ">TOP>L1>L2>L3>L4>L5>L6>L7>L8");
        assert.equal(chain_codes(platform_http_error(500, "/v1/x", chained_body(40))), ">TOP>L1>L2>L3>L4>L5>L6>L7>L8");
        assert.equal(chain_codes(manager_http_error(500, "/v1/c", chained_body(9))), ">TOP>L1>L2>L3>L4>L5>L6>L7>L8");
    }

    test "a malformed source never changes the code of the error that carries it" {
        for (let source in [`"boom"`, "7", "[]", "{}", `{"code":7,"source":{"source":[{}]}}`, `{"httpStatusCode":70000,"retryable":3}`]) {
            let e = platform_http_error(409, "/v1/x", `{"code":"CONFLICT","message":"exists","retryable":true,"source":${source}}`);
            assert.equal(e.code, "CONFLICT");
            assert.equal(e.message, "exists");
            assert.equal(e.retryable, true);
            assert.equal(e.http_status, 409);
            let m = manager_http_error(409, "/v1/c", `{"code":"CONFLICT","message":"exists","source":${source}}`);
            assert.equal(m.code, "CONFLICT");
            assert.equal(m.http_status, 409);
        }
    }

    test "a server's context is kept exactly as sent, whatever JSON it is, at the top and on a source level" {
        for (let sent in [
            `{"field":"name","limits":{"max":64,"ratio":0.5,"offset":-3},"tags":["a",1,null,true]}`,
            `"why"`,
            `""`,
            `[1,"a",null,[2.5,-7],{"k":false}]`,
            `[]`,
            `{}`,
            "7",
            "-7",
            "0",
            "2.5",
            "-0.125",
            "true",
            "false",
            // Above 2^53, and above BAML's 63-bit int.
            "9007199254740993",
            "123456789012345678901234567890",
        ]) {
            assert.equal(platform_error_context(sent), sent);
            assert.equal(source_context(sent), sent);
        }
        // A whole float stays a float, an integer an integer.
        assert.equal(platform_error_context("1.0"), "1.0");
        assert.equal(platform_error_context("1"), "1");
        assert.equal(platform_error_context(`{"n":1.0,"m":1}`), `{"n":1.0,"m":1}`);
        // Keys keep the order they were sent in.
        assert.equal(platform_error_context(`{"z":1,"a":2}`), `{"z":1,"a":2}`);
        // No context, or a null one, is null.
        assert.equal(platform_error_context("null"), "null");
        assert.equal(platform_http_error(400, "/v1/x", `{"code":"BAD","message":"m"}`).context, null);
        assert.equal(source_context("null"), "null");
    }

    test "a server's context gets no key of the SDK's: no path, request ID, hint or details" {
        let e = platform_http_error(
            404,
            "/v1/managers/mgr_1?token=secret",
            `{"code":"NOT_FOUND","message":"m","hint":"h","requestId":"req_1","context":{"id":"mgr_1"}}`,
        );
        assert.equal(context_of(e), `{"id":"mgr_1"}`);
        assert.equal(e.hint, "h");
        assert.equal(e.request_id, "req_1");
    }

    test "platform API errors without a retryable flag are not retryable" {
        let e = platform_http_error(409, "/v1/x", `{"code":"CONFLICT","message":"exists"}`);
        assert.equal(e.code, "CONFLICT");
        assert.equal(e.retryable, false);
        assert.equal(e.context, null);
    }

    test "other platform error bodies become PLATFORM_HTTP_ERROR, retryable on 408, 429 and 5xx" {
        assert.equal(platform_http_error(503, "/v1/x", "<html>busy</html>").code, "PLATFORM_HTTP_ERROR");
        assert.equal(platform_http_error(503, "/v1/x", "<html>busy</html>").retryable, true);
        assert.equal(platform_http_error(429, "/v1/x", "").retryable, true);
        assert.equal(platform_http_error(408, "/v1/x", "").retryable, true);
        assert.equal(platform_http_error(400, "/v1/x", `{"error":"x"}`).code, "PLATFORM_HTTP_ERROR");
        assert.equal(platform_http_error(400, "/v1/x", `{"error":"x"}`).retryable, false);
        // The body is not kept, nor the path: there is nothing to put in the context.
        assert.equal(platform_http_error(500, "/v1/x?token=secret", "oops").context, null);
        assert.equal(platform_http_error(500, "/v1/x", "").context, null);
    }

    test "manager error bodies keep code and message, never retryable" {
        let e = manager_http_error(503, "/v1/commands/c1", `{"code":"UNAVAILABLE","message":"later","details":"queue full"}`);
        assert.equal(e.code, "UNAVAILABLE");
        assert.equal(e.message, "later");
        assert.equal(e.retryable, false);
        assert.equal(e.http_status, 503);
        // An ErrorResponse has no context: its details stand in.
        assert.equal(context_of(e), `{"details":"queue full"}`);
        assert.equal(has_no_server_fields(e), true);
    }

    test "manager error bodies in Alien's full error shape keep hint, request ID, internal and source" {
        let e = manager_http_error(
            503,
            "/v1/commands/c1",
            `{"code":"UNAVAILABLE","message":"later","retryable":true,"hint":"retry in a minute","requestId":"req_m1","internal":false,"httpStatusCode":503,"context":{"queue":"full"},"source":{"code":"STORE_DOWN","message":"the store is down","retryable":true,"httpStatusCode":502,"context":{"store":"kv"},"source":{"message":"connection refused"}}}`,
        );
        assert.equal(e.code, "UNAVAILABLE");
        assert.equal(e.message, "later");
        // Manager errors are never retryable, whatever the body says.
        assert.equal(e.retryable, false);
        assert.equal(e.http_status, 503);
        assert.equal(e.hint, "retry in a minute");
        assert.equal(e.request_id, "req_m1");
        assert.equal(e.internal, false);
        // The body's context, as sent.
        assert.equal(context_of(e), `{"queue":"full"}`);
        assert.equal(chain_codes(e), ">UNAVAILABLE>STORE_DOWN>NESTED_ERROR");
        let first = e.source ?? invalid_request("no first level");
        // A source level is read as the server wrote it, `retryable` included.
        assert.equal(first.retryable, true);
        assert.equal(first.http_status, 502);
        assert.equal(context_of(first), `{"store":"kv"}`);
        assert.equal(first.source?.message, "connection refused");
    }

    test "a manager error's context is the body's context, else its details, else null" {
        let both = manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","details":"why","context":{"field":"name","n":[1,2.5]}}`);
        assert.equal(context_of(both), `{"field":"name","n":[1,2.5]}`);
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","context":"just text"}`)), `"just text"`);
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","context":[1,-2]}`)), `[1,-2]`);
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","context":0}`)), "0");
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","context":false}`)), "false");
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","details":"why"}`)), `{"details":"why"}`);
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","details":"why","context":null}`)), `{"details":"why"}`);
        assert.equal(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m"}`).context, null);
        assert.equal(manager_http_error(400, "/v1/c", `{"code":"BAD","message":"m","details":null,"context":null}`).context, null);
    }

    test "a response's request ID is its x-request-id header; none, or an empty one, is no ID" {
        assert.equal(response_request_id(response_with(200, { "x-request-id": "req_h1" }, "{}")), "req_h1");
        assert.equal(response_request_id(response_with(200, { "content-type": "application/json", "x-request-id": "req_h1" }, "")), "req_h1");
        // No header: `get`, so no panic.
        assert.equal(response_request_id(response_with(200, {}, "{}")), null);
        assert.equal(response_request_id(response_with(502, { "content-type": "text/plain" }, "bad gateway")), null);
        // An empty value (what a header that is not UTF-8 arrives as) is no ID.
        assert.equal(response_request_id(response_with(200, { "x-request-id": "" }, "{}")), null);
        // The 2xx reply keeps it for the errors of decoding the body.
        assert.equal(reply(response_with(200, { "x-request-id": "req_h1" }, "{}"), "GET", "/v1/x", false).request_id, "req_h1");
        assert.equal(reply(response_with(204, {}, ""), "GET", "/v1/x", true).request_id, null);
    }

    test "an API error body's requestId wins over the header, which is used when the body has none" {
        let body = `{"code":"NOT_FOUND","message":"m","requestId":"req_body"}`;
        let bare = `{"code":"NOT_FOUND","message":"m"}`;
        for (let manager in [false, true]) {
            assert.equal(request_id_of(body, "req_header", manager), "req_body");
            assert.equal(request_id_of(body, null, manager), "req_body");
            assert.equal(request_id_of(body, "", manager), "req_body");
            assert.equal(request_id_of(bare, "req_header", manager), "req_header");
            assert.equal(request_id_of(bare, null, manager), null);
            assert.equal(request_id_of(bare, "", manager), null);
            // A null or empty requestId in the body is none.
            assert.equal(request_id_of(`{"code":"NOT_FOUND","message":"m","requestId":null}`, "req_header", manager), "req_header");
            assert.equal(request_id_of(`{"code":"NOT_FOUND","message":"m","requestId":""}`, "req_header", manager), "req_header");
            // The error is still the server's.
            let e = reply_error(response_with(404, { "x-request-id": "req_header" }, bare), manager);
            assert.equal(e.code, "NOT_FOUND");
            assert.equal(e.http_status, 404);
        }
        // A Manager ErrorResponse has no requestId of its own.
        assert.equal(request_id_of(`{"code":"UNAVAILABLE","message":"later","details":"queue full"}`, "req_header", true), "req_header");
    }

    test "an error response that is not the API's error body takes the header's request ID" {
        let platform = reply_error(response_with(502, { "x-request-id": "req_proxy" }, "bad gateway"), false);
        assert.equal(platform.code, "PLATFORM_HTTP_ERROR");
        assert.equal(platform.request_id, "req_proxy");
        assert.equal(platform.context, null);
        let manager = reply_error(response_with(502, { "x-request-id": "req_proxy" }, "bad gateway"), true);
        assert.equal(manager.code, "MANAGER_HTTP_ERROR");
        assert.equal(manager.request_id, "req_proxy");
        // JSON of another shape, an empty body, and no header at all.
        assert.equal(request_id_of(`{"error":"x","requestId":"not read"}`, "req_proxy", false), "req_proxy");
        assert.equal(request_id_of("", "req_proxy", false), "req_proxy");
        assert.equal(request_id_of("", "req_proxy", true), "req_proxy");
        assert.equal(request_id_of("bad gateway", null, false), null);
        assert.equal(request_id_of("bad gateway", null, true), null);
        assert.equal(request_id_of("bad gateway", "", false), null);
    }

    test "a source level's request ID is only its own: the header never reaches it" {
        let e = reply_error(
            response_with(
                500,
                { "x-request-id": "req_header" },
                `{"code":"TOP","message":"m","source":{"code":"L1","message":"m","source":{"code":"L2","message":"m","requestId":"req_l2"}}}`,
            ),
            false,
        );
        assert.equal(e.request_id, "req_header");
        assert.equal(e.source?.request_id, null);
        assert.equal(e.source?.source?.request_id, "req_l2");
        let m = reply_error(
            response_with(500, { "x-request-id": "req_header" }, `{"code":"TOP","message":"m","requestId":"req_body","source":{"code":"L1","message":"m"}}`),
            true,
        );
        assert.equal(m.request_id, "req_body");
        assert.equal(m.source?.request_id, null);
    }

    test "a 2xx body that does not decode keeps the response's request ID" {
        let e = decoded_reply_error({ "x-request-id": "req_ok" }, `{"id":7}`);
        assert.equal(e.code, "MALFORMED_RESPONSE");
        assert.equal(e.http_status, 200);
        assert.equal(e.request_id, "req_ok");
        assert.equal(decoded_reply_error({ "x-request-id": "req_ok" }, "<html>").request_id, "req_ok");
        assert.equal(decoded_reply_error({}, "<html>").request_id, null);
        assert.equal(decoded_reply_error({ "x-request-id": "" }, "<html>").request_id, null);
        // The helpers take it by name and default to none, for callers that have no response.
        assert.equal(decode_error_of<root.platform.Manager>("<html>").request_id, null);
        assert.equal(malformed_response(200, "/v1/x", "", "why", request_id = "req_ok").request_id, "req_ok");
        // An empty 2xx body decodes as null, with or without an ID.
        assert.equal(decode_optional_body<root.platform.Manager>("", 200, "/v1/x", request_id = "req_ok"), null);
    }

    test "a communication error has the request ID of a response that arrived, and none without a response" {
        let unread = communication_error("GET", "/v1/x", 200, "connection reset", request_id = "req_partial");
        assert.equal(unread.request_id, "req_partial");
        assert.equal(unread.http_status, 200);
        assert.equal(communication_error("GET", "/v1/x", null, "connection refused").request_id, null);
        // Nothing else of a server's is set.
        assert.equal(unread.hint, null);
        assert.equal(unread.source, null);
        assert.equal(unread.internal, null);
    }

    test "the errors the SDK raises itself have no hint, request ID, source or internal flag" {
        assert.equal(has_no_server_fields(invalid_request("m")), true);
        assert.equal(has_no_server_fields(communication_error("GET", "/v1/x", null, "refused")), true);
        assert.equal(has_no_server_fields(malformed_response(200, "/v1/x", "", "why")), true);
        assert.equal(has_no_server_fields(platform_http_error(502, "/v1/x", "bad gateway")), true);
        assert.equal(has_no_server_fields(manager_http_error(502, "/v1/c", "bad gateway")), true);
        assert.equal(has_no_server_fields(setup_error("SETUP_INVALID_PARAMETER", "m", {})), true);
        // A body that is not the API's error body is not read for them either.
        let other = platform_http_error(400, "/v1/x", `{"error":"x","hint":"h","requestId":"r","internal":true,"source":{"code":"S"}}`);
        assert.equal(other.code, "PLATFORM_HTTP_ERROR");
        assert.equal(has_no_server_fields(other), true);
    }

    test "other manager error bodies become MANAGER_HTTP_ERROR, never retryable" {
        let e = manager_http_error(502, "/v1/commands", "bad gateway");
        assert.equal(e.code, "MANAGER_HTTP_ERROR");
        assert.equal(e.retryable, false);
        assert.equal(e.context, null);
    }

    test "a response that does not decode says where in the body, and why" {
        let valid = manager_with_configs("{}");
        let invalid = manager_with_configs(`{"aws":{"managingRoleArn":7,"platform":"aws"}}`);
        let e = decode_error_of<root.platform.Manager[]>(`[${valid},${invalid}]`);
        assert.equal(e.code, "MALFORMED_RESPONSE");
        assert.equal(e.message, "Response from /v1/x did not match the expected shape at [1].managementConfigs.aws.managingRoleArn: expected string");
        assert.equal(context_of(e), `{"json_path":"[1].managementConfigs.aws.managingRoleArn","decode_error":"expected string"}`);
    }

    test "a body that is not JSON, or does not match at its root, has no json_path" {
        let text = decode_error_of<root.platform.Manager>("<html>busy</html>");
        assert.equal(text.message, "Response from /v1/x did not match the expected shape: expected value at line 1 column 1");
        assert.equal(context_of(text), `{"decode_error":"expected value at line 1 column 1"}`);
        assert.equal(context_string(decode_error_of<root.platform.Manager>("[]"), "json_path"), null);
    }

    test "a long decoder message is cut in the error message and kept whole in the context" {
        let long = "no union member matched: ";
        while (long.length() < 2500) {
            long = long + "`platform.ExternalBindingStorageS3`: literal mismatch at `.service`; ";
        }
        let e = malformed_response(200, "/v1/x", "", long);
        assert.equal(context_string(e, "decode_error"), long);
        assert.equal(e.message.length() < 400, true);
        assert.equal(e.message.starts_with("Response from /v1/x did not match the expected shape: no union member matched: "), true);
        assert.equal(e.message.ends_with("… (the whole message is in context.decode_error)"), true);
    }

    test "a failure inside a nested class says where" {
        let e = decode_error_of<root.platform.ListDeploymentsResponse>(`{"items":[{"id":"d1"}]}`);
        assert.equal(e.code, "MALFORMED_RESPONSE");
        assert.equal(context_string(e, "json_path"), ".items[0]");
    }

    test "an error body that is JSON of another shape says why it is not the API's error body" {
        let platform = platform_http_error(400, "/v1/x", `{"code":"BAD","message":"m","retryable":"yes"}`);
        assert.equal(platform.code, "PLATFORM_HTTP_ERROR");
        assert.equal(context_of(platform), `{"json_path":".retryable","decode_error":"expected boolean"}`);
        let missing = platform_http_error(502, "/v1/x", `{"message":"bad gateway"}`);
        assert.equal(context_of(missing), "{\"decode_error\":\"missing required field `code`\"}");
        assert.equal(context_of(manager_http_error(400, "/v1/c", `{"code":1,"message":"m"}`)), `{"json_path":".code","decode_error":"expected string"}`);
        // A body that is not JSON at all leaves nothing to say: no context.
        assert.equal(platform_http_error(500, "/v1/x", "oops").context, null);
        assert.equal(manager_http_error(502, "/v1/c", "bad gateway").context, null);
    }

    test "the contexts of the SDK's own errors hold no request path and no response body" {
        let path = "/v1/user/invitations/tok_secret/accept?workspace=w";
        assert.equal(context_of(communication_error("POST", path, null, "refused")), `{"method":"POST"}`);
        assert.equal(context_of(communication_error("GET", path, 200, "reset")), `{"method":"GET"}`);
        assert.equal(context_of(malformed_response(200, path, ".a", "why")), `{"json_path":".a","decode_error":"why"}`);
        assert.equal(context_of(malformed_response(200, path, "", "why")), `{"decode_error":"why"}`);
        assert.equal(invalid_request("m").context, null);
        assert.equal(context_of(setup_error("SETUP_REGION_UNSUPPORTED", "m", { "field": "region", "region": "cn-north-1" })), `{"field":"region","region":"cn-north-1"}`);
        // The messages still name the path.
        assert.equal(communication_error("POST", path, null, "refused").message, `HTTP request POST ${path} failed: refused`);
    }

    test "a timeout says which limit elapsed, and after how long when BAML says" {
        let total = baml.errors.Timeout { message: "m", duration: baml.time.Duration.from_milliseconds(300), timeout_type: "timeout" };
        assert.equal(timed_out("timed out", total), "timed out after 300 ms (timeout)");
        let connect = baml.errors.Timeout { message: "m", duration: null, timeout_type: "connect_timeout" };
        assert.equal(timed_out("timed out", connect), "timed out (connect_timeout)");
    }

    test "communication errors are retryable and keep the status of a response whose body could not be read" {
        let unread = communication_error("GET", "/v1/x", 200, "connection reset");
        assert.equal(unread.code, "COMMUNICATION_ERROR");
        assert.equal(unread.retryable, true);
        assert.equal(unread.http_status, 200);
        assert.equal(communication_error("GET", "/v1/x", null, "connection refused").http_status, null);
    }

    test "a union member this SDK does not know decodes as the union's Other member, tagged UNKNOWN, and a known one that does not decode still fails" {
        assert.equal(event_data_member(`{"type":"DeploymentCheckRequested","deploymentId":"dep_1"}`), "known dep_1");
        assert.equal(event_data_member(`{"type":"SomethingNew","deploymentId":"dep_1"}`), "UNKNOWN SomethingNew");
        assert.equal(event_data_member(`{"type":"DeploymentCheckRequested"}`), "MALFORMED_RESPONSE");
        assert.equal(event_data_member(`{"kind":"no type"}`), "MALFORMED_RESPONSE");
        let wire = `{"type":"SomethingNew","deploymentId":"dep_1","nested":{"a":[1,null]}}`;
        let event = decode_body<root.platform.Event>(event_with_data(wire), 200, "/v1/x");
        assert.equal(encode_body(event.data), wire);
    }

    test "extra keys of an open object survive decoding and encoding, and never replace a listed field" {
        let wire = `{"id":"r1","type":"function","memory":512,"env":{"A":"1"}}`;
        let config = decode_body<root.platform.DeploymentStackStateResourcesValueConfig>(wire, 200, "/v1/x");
        assert.equal(config.id, "r1");
        assert.equal(config.additionalProperties.length(), 2);
        assert.equal(baml.json.stringify(config.additionalProperties["memory"]), "512");
        assert.equal(encode_body(config), wire);
        let built = root.platform.DeploymentStackStateResourcesValueConfig {
            id: "r2",
            type: "queue",
            additionalProperties: { "id": "other", "visible": true },
        };
        assert.equal(encode_body(built), `{"id":"r2","type":"queue","visible":true}`);
    }
}
