//! The SDK's errors as `alien_error::AlienError` (cargo feature `alien-error`,
//! `src/alien_error.rs`): every field at every level, where the request ID
//! goes, `?` and `into_sdk_error()` in code written as Alien's is, and what
//! `alien_error`'s own `.into_alien_error()` would lose. The last tests fail
//! real calls against conformance/stub-server.
#![cfg(feature = "alien-error")]

#[allow(dead_code)]
mod support;

use std::error::Error as _;

use alien_sdk::alien_error::{self, AlienErrorData, Context, GenericError, HumanLayerPresentation};
use alien_sdk::{Alien, AlienError, BamlOptions, Error, Options, SdkResultExt};
use serde_json::{Value, json};
use support::{Stub, require_isolated_baml_home};

/// An SDK error with this code and nothing else set. `alien_sdk::AlienError`
/// is `#[non_exhaustive]`, so the SDK is the only one that builds it: this is
/// the error of a client without an API key, with its fields overwritten.
fn sdk_error(code: &str) -> AlienError {
    let Err(Error::Alien(mut error)) = Alien::new(Options {
        api_key: Some(String::new()),
        ..Default::default()
    }) else {
        panic!("a client without an API key");
    };
    assert_eq!(
        (&error.context, &error.hint, &error.request_id, &error.source, error.internal, error.http_status_code),
        (&None, &None, &None, &None, None, None)
    );
    error.code = code.to_string();
    error.message = format!("{code} failed");
    error
}

/// A three-level error with every field set on every level, each its own.
fn full_chain() -> AlienError {
    let level = |code: &str, status: u16, retryable: bool, internal: bool, source: Option<AlienError>| {
        let mut error = sdk_error(code);
        error.retryable = retryable;
        error.http_status_code = Some(status);
        error.context = Some(json!({ "level": code, "limits": [1, 2.5, null] }));
        error.hint = Some(format!("{code} hint"));
        error.request_id = Some(format!("req_{code}"));
        error.internal = Some(internal);
        error.source = source.map(Box::new);
        error
    };
    let innermost = level("INNERMOST", 502, false, true, None);
    let inner = level("INNER", 403, true, false, Some(innermost));
    level("OUTER", 500, false, true, Some(inner))
}

#[test]
fn every_field_is_kept_at_every_level() {
    let converted = alien_error::AlienError::from(full_chain());
    let mut level = Some(&converted);
    for (code, status, retryable, internal) in [
        ("OUTER", 500, false, true),
        ("INNER", 403, true, false),
        ("INNERMOST", 502, false, true),
    ] {
        let error = level.unwrap_or_else(|| panic!("no level {code}"));
        assert_eq!(error.code, code);
        assert_eq!(error.message, format!("{code} failed"));
        assert_eq!(error.retryable, retryable, "{code}");
        assert_eq!(error.internal, internal, "{code}");
        assert_eq!(error.http_status_code, Some(status), "{code}");
        assert_eq!(error.hint, Some(format!("{code} hint")));
        // The context as it was, plus the level's own request ID.
        assert_eq!(
            error.context,
            Some(json!({ "level": code, "limits": [1, 2.5, null], "requestId": format!("req_{code}") }))
        );
        assert_eq!(error.error, Some(GenericError { message: format!("{code} failed") }));
        assert_eq!(error.human_layer_presentation, HumanLayerPresentation::Normal);
        level = error.source.as_deref();
    }
    assert!(level.is_none(), "a fourth level");

    // From `Error::Alien` it is the same value.
    assert_eq!(alien_error::AlienError::from(Error::Alien(full_chain())), converted);

    // An error with nothing but a code: no status made up, no context, not internal.
    let bare = alien_error::AlienError::from(sdk_error("BARE"));
    assert_eq!(
        (bare.code.as_str(), bare.retryable, bare.internal, bare.http_status_code, &bare.context, &bare.hint),
        ("BARE", false, false, None, &None, &None)
    );
    assert!(bare.source.is_none());
}

/// The context of the conversion of an SDK error with this context and request ID.
fn merged(context: Option<Value>, request_id: Option<&str>) -> Option<Value> {
    let mut error = sdk_error("X");
    error.context = context;
    error.request_id = request_id.map(str::to_string);
    alien_error::AlienError::from(error).context
}

#[test]
fn the_request_id_goes_into_the_context_by_aliens_rule() {
    // An object gains `requestId`.
    assert_eq!(
        merged(Some(json!({ "field": "name" })), Some("req_1")),
        Some(json!({ "field": "name", "requestId": "req_1" }))
    );
    assert_eq!(merged(Some(json!({})), Some("req_1")), Some(json!({ "requestId": "req_1" })));
    // One it already has is not overwritten, whatever its value.
    assert_eq!(
        merged(Some(json!({ "requestId": "req_servers", "field": "name" })), Some("req_1")),
        Some(json!({ "requestId": "req_servers", "field": "name" }))
    );
    assert_eq!(merged(Some(json!({ "requestId": null })), Some("req_1")), Some(json!({ "requestId": null })));
    // No context becomes one that holds the request ID.
    assert_eq!(merged(None, Some("req_1")), Some(json!({ "requestId": "req_1" })));
    // Any other value is wrapped, under "context".
    for value in [json!("just text"), json!([1, { "a": 2 }]), json!(-12.5), json!(0), json!(false), json!("")] {
        assert_eq!(
            merged(Some(value.clone()), Some("req_1")),
            Some(json!({ "requestId": "req_1", "context": value }))
        );
    }
    // Without a request ID the context is untouched, whatever it is.
    for context in [None, Some(json!({ "field": "name" })), Some(json!("just text")), Some(json!([1])), Some(json!(7))] {
        assert_eq!(merged(context.clone(), None), context);
    }

    // Each level merges its own request ID into its own context.
    let mut inner = sdk_error("INNER");
    inner.context = Some(json!("inner text"));
    inner.request_id = Some("req_inner".into());
    let mut outer = sdk_error("OUTER");
    outer.request_id = Some("req_outer".into());
    outer.source = Some(Box::new(inner));
    let converted = alien_error::AlienError::from(outer);
    assert_eq!(converted.context, Some(json!({ "requestId": "req_outer" })));
    assert_eq!(
        converted.source.expect("a source").context,
        Some(json!({ "requestId": "req_inner", "context": "inner text" }))
    );
}

#[test]
fn internal_is_false_when_the_sdk_error_has_none() {
    let converted = |internal: Option<bool>| {
        let mut error = sdk_error("X");
        error.internal = internal;
        alien_error::AlienError::from(error).internal
    };
    assert_eq!([converted(None), converted(Some(false)), converted(Some(true))], [false, false, true]);
}

/// A runtime failure with a source, as `Error::Runtime` holds them.
#[derive(Debug)]
struct RuntimeFailure(std::io::Error);

impl std::fmt::Display for RuntimeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the runtime failed")
    }
}

impl std::error::Error for RuntimeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[test]
fn a_runtime_error_becomes_a_generic_error() {
    let runtime = || Error::Runtime(Box::new(RuntimeFailure(std::io::Error::other("disk full"))));
    let converted = alien_error::AlienError::from(runtime());
    assert_eq!(
        (converted.code.as_str(), converted.message.as_str(), converted.retryable, converted.internal),
        ("GENERIC_ERROR", "the runtime failed", false, false)
    );
    assert_eq!(converted.context, None);
    assert_eq!(converted.hint, None);
    // `Error::Runtime`'s own source is the held error's source, so that is
    // the next level here too, as with `.into_alien_error()` on the held error.
    let source = converted.source.as_deref().expect("the held error's source");
    assert_eq!((source.code.as_str(), source.message.as_str()), ("GENERIC_ERROR", "disk full"));
    assert_eq!(
        converted,
        alien_error::IntoAlienErrorDirect::into_alien_error(RuntimeFailure(std::io::Error::other("disk full")))
    );
    assert_eq!(Err::<(), _>(runtime()).into_sdk_error().expect_err("an error").code, "GENERIC_ERROR");
}

/// A caller's own error, as code written with `alien-error` declares them.
#[derive(Debug, Clone, serde::Serialize)]
struct LookupFailed {
    id: String,
}

impl AlienErrorData for LookupFailed {
    fn code(&self) -> &'static str {
        "MANAGER_LOOKUP_FAILED"
    }
    fn retryable(&self) -> bool {
        false
    }
    fn internal(&self) -> bool {
        false
    }
    fn message(&self) -> String {
        format!("cannot look up manager {}", self.id)
    }
    fn http_status_code(&self) -> u16 {
        502
    }
    // Take these three from the error underneath, as `#[derive(AlienError)]`
    // does for a variant marked `inherit`.
    fn retryable_inherit(&self) -> Option<bool> {
        None
    }
    fn internal_inherit(&self) -> Option<bool> {
        None
    }
    fn http_status_code_inherit(&self) -> Option<u16> {
        None
    }
}

/// `error` as `.context(..)` keeps it under the caller's error: every field
/// but `error`, the value for pattern matching, which `alien-error` drops
/// from the level it wraps.
fn under_context(error: &alien_error::AlienError) -> alien_error::AlienError {
    let mut kept = error.clone();
    kept.error = None;
    kept
}

/// An SDK call that fails with `full_chain()`.
fn sdk_call() -> Result<(), Error> {
    Err(Error::Alien(full_chain()))
}

/// `?` in a function that returns `alien_error::Result`: it compiles because
/// of `From<alien_sdk::Error> for alien_error::AlienError`.
fn with_question_mark() -> alien_error::Result<()> {
    sdk_call()?;
    Ok(())
}

/// The same for a result whose error is the `AlienError` itself.
fn with_question_mark_on_the_inner_error() -> alien_error::Result<()> {
    Err(full_chain())?;
    Ok(())
}

/// Alien's own shape: convert, then chain the caller's error on.
fn with_context() -> alien_error::Result<(), LookupFailed> {
    sdk_call()
        .into_sdk_error()
        .context(LookupFailed { id: "mgr_1".into() })?;
    Ok(())
}

#[test]
fn question_mark_and_into_sdk_error_keep_the_error() {
    let want = alien_error::AlienError::from(full_chain());
    assert_eq!(with_question_mark().expect_err("an error"), want);
    assert_eq!(with_question_mark_on_the_inner_error().expect_err("an error"), want);
    assert_eq!(sdk_call().into_sdk_error().expect_err("an error"), want);
    assert_eq!(Err::<(), _>(full_chain()).into_sdk_error().expect_err("an error"), want);
    assert_eq!(Ok::<u8, Error>(7).into_sdk_error(), Ok(7));

    // `.context(..)` puts the caller's error on top and the SDK's, whole, under it.
    let chained = with_context().expect_err("an error");
    assert_eq!(chained.code, "MANAGER_LOOKUP_FAILED");
    assert_eq!(chained.message, "cannot look up manager mgr_1");
    assert_eq!(chained.source.as_deref(), Some(&under_context(&want)));
    // What the caller's error inherits comes from the SDK's error.
    assert_eq!((chained.retryable, chained.internal, chained.http_status_code), (false, true, Some(500)));
    let source = chained.source().expect("std's source");
    assert_eq!(source.to_string().lines().next(), Some("OUTER: OUTER failed"));
}

#[test]
fn display_of_the_converted_error_shows_the_tree() {
    // The SDK's own error prints one line, the converted one the chain.
    assert_eq!(full_chain().to_string(), "OUTER: OUTER failed");
    assert_eq!(Error::Alien(full_chain()).to_string(), "OUTER: OUTER failed");
    assert_eq!(
        alien_error::AlienError::from(full_chain()).to_string(),
        "OUTER: OUTER failed\n├─▶ INNER: INNER failed\n│   ├─▶ INNERMOST: INNERMOST failed\n"
    );
    assert_eq!(
        with_context().expect_err("an error").to_string(),
        "MANAGER_LOOKUP_FAILED: cannot look up manager mgr_1\n├─▶ OUTER: OUTER failed\n│   ├─▶ INNER: INNER failed\n│   │   ├─▶ INNERMOST: INNERMOST failed\n"
    );
    assert_eq!(alien_error::AlienError::from(sdk_error("ALONE")).to_string(), "ALONE: ALONE failed");
}

#[test]
fn into_alien_error_is_the_lossy_path() {
    // `alien-error` gives every std error `.into_alien_error()`, the SDK's
    // included, and Alien's code calls it by habit. It reads only what a std
    // error has, its text and its sources: the code, the retryable flag, the
    // status, the hint and the context are gone, at every level.
    let lossy = alien_error::IntoAlienErrorDirect::into_alien_error(Error::Alien(full_chain()));
    assert_eq!(lossy.code, "GENERIC_ERROR");
    assert_eq!(lossy.message, "OUTER: OUTER failed");
    assert_eq!((lossy.retryable, lossy.internal, lossy.http_status_code), (false, false, Some(500)));
    assert_eq!((&lossy.hint, &lossy.context), (&None, &None));
    let source = lossy.source.as_deref().expect("a source");
    assert_eq!((source.code.as_str(), source.message.as_str()), ("GENERIC_ERROR", "INNER: INNER failed"));
    assert_eq!((source.retryable, &source.hint, &source.context), (false, &None, &None));
    let on_a_result = alien_error::IntoAlienError::into_alien_error(sdk_call()).expect_err("an error");
    assert_eq!(on_a_result, lossy);

    // `?` and `into_sdk_error()` are the lossless ones.
    let lossless = with_question_mark().expect_err("an error");
    assert_eq!(lossless.code, "OUTER");
    assert_ne!(lossless, lossy);
    assert_eq!(sdk_call().into_sdk_error().expect_err("an error"), lossless);
}

/// A trait shaped like the `SdkResultExt` of Alien's own Rust clients
/// (`alien-platform-api`, `alien-manager-api`): the same name, the same
/// method, implemented for the results of another client.
mod theirs {
    use alien_sdk::alien_error::{AlienError, GenericError};

    #[derive(Debug)]
    pub struct ProgenitorError;

    pub trait SdkResultExt<T> {
        fn into_sdk_error(self) -> Result<T, AlienError<GenericError>>;
    }

    impl<T> SdkResultExt<T> for Result<T, ProgenitorError> {
        fn into_sdk_error(self) -> Result<T, AlienError<GenericError>> {
            self.map_err(|_| AlienError::new(GenericError { message: "theirs".into() }))
        }
    }
}

#[test]
fn it_works_next_to_aliens_own_sdk_result_ext() {
    // Both traits in scope in one function: theirs by name, the SDK's unnamed
    // (two imports of one name do not compile). Each call picks the trait
    // that is implemented for its result.
    use alien_sdk::SdkResultExt as _;
    use theirs::SdkResultExt;

    let ours = sdk_call().into_sdk_error().expect_err("an error");
    let other = Err::<(), _>(theirs::ProgenitorError).into_sdk_error().expect_err("an error");
    assert_eq!((ours.code.as_str(), other.message.as_str()), ("OUTER", "theirs"));
    // Named in full, either is still reachable.
    assert_eq!(
        <Result<(), Error> as alien_sdk::SdkResultExt<()>>::into_sdk_error(sdk_call()).expect_err("an error"),
        ours
    );
    let _: fn(Result<(), theirs::ProgenitorError>) -> _ = <Result<(), theirs::ProgenitorError> as SdkResultExt<()>>::into_sdk_error;
}

// ---------------------------------------------------------------------------
// Real calls, failed by conformance/stub-server.

/// The client for a scenario with one exchange: `GET /v1/managers/mgr_x`,
/// answered with `respond`.
fn client_for(stub: &Stub, respond: Value) -> Alien {
    let loaded = stub.post(
        "/__load",
        &json!({ "scenario": {
            "id": "alien_error", "phase": 0,
            "call": { "fn": "platform.getManager", "args": { "client": { "base_url": "${stub.base}", "api_key": "${secrets.apiKey}" } } },
            "exchanges": [{ "expect": { "method": "GET", "path": "/v1/managers/mgr_x" }, "respond": respond }],
        } }),
    );
    let client = &loaded["call"]["args"]["client"];
    Alien::new(Options {
        api_key: client["api_key"].as_str().map(str::to_string),
        server_url: client["base_url"].as_str().map(str::to_string),
        ..Default::default()
    })
    .expect("a client")
}

/// A function written as Alien's code is: it returns `alien_error::Result`
/// and lets `?` convert the SDK's error.
async fn manager_name(alien: &Alien) -> alien_error::Result<String> {
    Ok(alien.managers().get("mgr_x").send().await?.name)
}

/// The same with the caller's own error chained on.
async fn manager_name_with_context(alien: &Alien) -> alien_error::Result<String, LookupFailed> {
    let manager = alien
        .managers()
        .get("mgr_x")
        .send()
        .await
        .into_sdk_error()
        .context(LookupFailed { id: "mgr_x".into() })?;
    Ok(manager.name)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_call_converts_with_every_field() {
    require_isolated_baml_home();
    alien_sdk::init().expect("start the BAML runtime");
    let stub = Stub::start();

    // A server's error: every field, a context that is not an object, a
    // request ID in the body on top and only in a nested level's body below.
    let api_error = json!({
        "status": 404,
        "headers": { "x-request-id": "req_header" },
        "json": {
            "code": "MANAGER_NOT_FOUND", "message": "No such manager.", "retryable": true, "internal": true,
            "hint": "Check the ID.", "requestId": "req_body", "context": ["mgr_x", 404],
            "source": {
                "code": "STORE_LOOKUP_FAILED", "message": "The store has no such row.", "httpStatusCode": 503,
                "context": { "table": "managers" },
                "source": { "code": "KV_TIMEOUT", "message": "The store timed out.", "requestId": "req_inner", "internal": true },
            },
        },
    });
    let error = manager_name(&client_for(&stub, api_error.clone())).await.expect_err("a 404");
    assert_eq!(
        (error.code.as_str(), error.message.as_str(), error.retryable, error.internal, error.http_status_code),
        ("MANAGER_NOT_FOUND", "No such manager.", true, true, Some(404))
    );
    assert_eq!(error.hint.as_deref(), Some("Check the ID."));
    // Not an object, so it is wrapped; the body's request ID won over the header's.
    assert_eq!(error.context, Some(json!({ "requestId": "req_body", "context": ["mgr_x", 404] })));
    let first = error.source.as_deref().expect("a source");
    assert_eq!(
        (first.code.as_str(), first.retryable, first.internal, first.http_status_code),
        ("STORE_LOOKUP_FAILED", false, false, Some(503))
    );
    // No request ID on this level: its context is untouched.
    assert_eq!(first.context, Some(json!({ "table": "managers" })));
    let second = first.source.as_deref().expect("a second source");
    assert_eq!(
        (second.code.as_str(), second.internal, second.http_status_code, &second.context),
        ("KV_TIMEOUT", true, None, &Some(json!({ "requestId": "req_inner" })))
    );
    assert!(second.source.is_none());
    assert_eq!(
        error.to_string(),
        "MANAGER_NOT_FOUND: No such manager.\n├─▶ STORE_LOOKUP_FAILED: The store has no such row.\n│   ├─▶ KV_TIMEOUT: The store timed out.\n"
    );

    // `.context(..)` chains onto it, and Alien's own lookup of a request ID
    // (the top level of a context, along the source chain) finds the server's.
    let chained = manager_name_with_context(&client_for(&stub, api_error)).await.expect_err("a 404");
    assert_eq!(chained.code, "MANAGER_LOOKUP_FAILED");
    assert_eq!((chained.retryable, chained.internal, chained.http_status_code), (true, true, Some(404)));
    assert_eq!(chained.source.as_deref(), Some(&under_context(&error)));
    let mut request_ids = Vec::new();
    let mut level = chained.source.as_deref();
    while let Some(e) = level {
        request_ids.extend(e.context.as_ref().and_then(|c| c.get("requestId")).and_then(Value::as_str));
        level = e.source.as_deref();
    }
    assert_eq!(request_ids, ["req_body", "req_inner"]);

    // An error the SDK raises for a response: the header's request ID is its
    // whole context.
    let proxy = json!({ "status": 502, "text": "bad gateway", "headers": { "x-request-id": "req_proxy" } });
    let error = manager_name(&client_for(&stub, proxy)).await.expect_err("a 502");
    assert_eq!(
        (error.code.as_str(), error.retryable, error.internal, error.http_status_code),
        ("PLATFORM_HTTP_ERROR", true, false, Some(502))
    );
    assert_eq!(error.context, Some(json!({ "requestId": "req_proxy" })));

    // One with a context of the SDK's own, an object: it gains the request ID.
    let malformed = json!({ "status": 200, "json": { "id": 7 }, "headers": { "x-request-id": "req_ok" } });
    let error = manager_name(&client_for(&stub, malformed)).await.expect_err("a body that does not decode");
    assert_eq!((error.code.as_str(), error.http_status_code), ("MALFORMED_RESPONSE", Some(200)));
    let context = error.context.expect("a context");
    assert_eq!(context["requestId"], "req_ok");
    assert!(context["decode_error"].is_string(), "{context}");

    // And one without a response: no status, no request ID, the SDK's context as it is.
    let unreachable = Alien::new(Options {
        api_key: Some("key".into()),
        server_url: Some("http://127.0.0.1:1".into()),
        ..Default::default()
    })
    .expect("a client");
    let error = manager_name(&unreachable).await.expect_err("nothing listens there");
    assert_eq!(
        (error.code.as_str(), error.retryable, error.http_status_code, &error.context),
        ("COMMUNICATION_ERROR", true, None, &Some(json!({ "method": "GET" })))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_per_call_timeout_converts_to_a_generic_error() {
    require_isolated_baml_home();
    alien_sdk::init().expect("start the BAML runtime");
    let stub = Stub::start();
    let alien = client_for(&stub, json!({ "status": 200, "json": {}, "delay_ms": 2000 }));
    // The per-call limit ends the call with the runtime's own error, not an
    // AlienError (docs/errors.md), so there is no code of Alien's to keep.
    let result = alien
        .managers()
        .get("mgr_x")
        .baml_options(BamlOptions::new().timeout_ms(100))
        .send()
        .await;
    let Err(Error::Runtime(runtime)) = &result else {
        panic!("expected Error::Runtime, got {result:?}");
    };
    let text = runtime.to_string();
    let converted = result.map(|_| ()).into_sdk_error().expect_err("a timeout");
    println!("a per-call timeout converts to {}: {:?}", converted.code, converted.message);
    assert_eq!(converted.code, "GENERIC_ERROR");
    assert_eq!(converted.message, text);
    // What `from_std` gives every std error: not retryable, GenericError's 500.
    assert_eq!((converted.retryable, converted.internal, converted.http_status_code), (false, false, Some(500)));
    assert_eq!(converted.context, None);
}
