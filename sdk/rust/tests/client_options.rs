//! Client construction: the `ALIEN_API_KEY` default, the error when there is
//! no API key, an empty Manager token, the timeout in milliseconds, and `Debug`
//! output that never shows a secret. Its own test
//! binary with one test, because it writes the environment. Nothing here
//! starts the BAML runtime.

use std::time::Duration;

use alien_sdk::{Alien, AlienManager, Error, Options};

const KEY: &str = "alien_test_api_key_never_printed";
const TOKEN: &str = "alien_test_manager_token_never_printed";
const URL_TOKEN: &str = "alien_test_url_token_never_printed";

fn assert_no_api_key(result: Result<Alien, Error>) {
    match result {
        Err(Error::Alien(e)) => {
            assert_eq!(
                (e.code.as_str(), e.retryable, e.http_status_code),
                ("INVALID_REQUEST", false, None)
            );
            assert_eq!(e.message, "no API key: pass api_key or set ALIEN_API_KEY");
        }
        other => panic!("expected INVALID_REQUEST, got {other:?}"),
    }
}

#[test]
fn api_key_default_and_redacted_debug() {
    // SAFETY (all three writes): the only test in this binary, so no other
    // thread reads the environment.
    unsafe { std::env::remove_var("ALIEN_API_KEY") };
    assert_no_api_key(Alien::new(Options::default()));
    unsafe { std::env::set_var("ALIEN_API_KEY", "") };
    assert_no_api_key(Alien::new(Options::default()));
    unsafe { std::env::set_var("ALIEN_API_KEY", KEY) };
    assert_no_api_key(Alien::new(Options {
        api_key: Some(String::new()),
        ..Default::default()
    }));
    assert_eq!(
        Alien::new(Options::default())
            .expect("key from ALIEN_API_KEY")
            .platform()
            .api_key,
        KEY
    );
    let explicit = Options {
        api_key: Some("explicit".into()),
        ..Default::default()
    };
    assert_eq!(
        Alien::new(explicit)
            .expect("explicit key")
            .platform()
            .api_key,
        "explicit"
    );

    let options = Options {
        api_key: Some(KEY.into()),
        server_url: Some(format!("https://alien.example/v1?token={URL_TOKEN}")),
        workspace: Some("my-workspace".into()),
        timeout: None,
    };
    let alien = Alien::new(options.clone()).expect("client");
    let manager = AlienManager::new(
        format!("https://manager.example?token={URL_TOKEN}"),
        Some(TOKEN.into()),
    );
    for text in [
        format!("{options:?}"),
        format!("{options:#?}"),
        format!("{alien:?}"),
        format!("{alien:#?}"),
        format!("{manager:?}"),
        format!("{manager:#?}"),
    ] {
        for secret in [KEY, TOKEN, URL_TOKEN] {
            assert!(
                !text.contains(secret),
                "Debug output shows {secret}: {text}"
            );
        }
    }
    assert_eq!(
        format!("{options:?}"),
        r#"Options { api_key: Some("<redacted>"), server_url: Some("https://alien.example/v1?<redacted>"), workspace: Some("my-workspace"), timeout: None }"#
    );
    assert_eq!(
        format!("{alien:?}"),
        r#"Alien { server_url: "https://alien.example/v1?<redacted>", api_key: "<redacted>", workspace: Some("my-workspace"), timeout_ms: None }"#
    );
    assert_eq!(
        format!("{manager:?}"),
        r#"AlienManager { server_url: "https://manager.example?<redacted>", token: Some("<redacted>"), timeout_ms: None }"#
    );
    assert_eq!(
        format!("{:?}", AlienManager::new("https://manager.example", None)),
        r#"AlienManager { server_url: "https://manager.example", token: None, timeout_ms: None }"#
    );
    // An empty token sends no Authorization header, as in the other SDKs.
    assert_eq!(
        AlienManager::new("https://manager.example", Some(String::new()))
            .client()
            .token,
        None
    );

    // The timeout reaches the generated clients in whole milliseconds, rounded up.
    for (timeout, want) in [
        (None, None),
        (Some(Duration::from_millis(300)), Some(300)),
        (Some(Duration::from_micros(1500)), Some(2)),
        (Some(Duration::from_nanos(1)), Some(1)),
        (Some(Duration::ZERO), Some(0)),
        (Some(Duration::MAX), Some((1 << 62) - 1)),
    ] {
        let alien = Alien::new(Options {
            api_key: Some(KEY.into()),
            timeout,
            ..Default::default()
        })
        .expect("client");
        assert_eq!(
            alien.platform().timeout_ms,
            want,
            "Options::timeout {timeout:?}"
        );
        if let Some(timeout) = timeout {
            assert_eq!(
                AlienManager::new("u", None)
                    .with_timeout(timeout)
                    .client()
                    .timeout_ms,
                want,
                "with_timeout {timeout:?}"
            );
        }
    }
    assert_eq!(AlienManager::new("u", None).client().timeout_ms, None);
}
