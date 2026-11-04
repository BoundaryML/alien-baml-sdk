//! Client construction: the error when there is no API key, and `Debug`
//! output of everything public that holds a client (the clients, groups,
//! builders and the setup helper) that never shows a secret (docs/gaps.md#b22). Its own test
//! binary with one test, because it writes the environment. Nothing here
//! starts the BAML runtime.

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
    // Which key the client holds, and the timeout in milliseconds: the unit
    // tests in src/lib.rs (the generated client is private).
    Alien::new(Options::default()).expect("key from ALIEN_API_KEY");

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
    // Everything public that holds a client: the clients, their groups and
    // builders (with and without arguments set), and the setup helper.
    let texts = [
        format!("{options:?}"),
        format!("{options:#?}"),
        format!("{alien:?}"),
        format!("{alien:#?}"),
        format!("{manager:?}"),
        format!("{manager:#?}"),
        format!("{:?}", alien.deployments()),
        format!("{:#?}", alien.deployments().list().project("web").limit(10)),
        format!("{:?}", alien.managers().get("mgr_1")),
        format!("{:?}", alien.deployment_groups().create_token("dg_1")),
        format!("{:?}", alien.auth().whoami()),
        format!("{:?}", alien.accept_workspace_invitation(TOKEN)),
        format!("{:?}", alien.setup()),
        format!("{:?}", manager.commands()),
        format!("{:#?}", manager.commands().get_command_status("cmd_1")),
    ];
    for text in &texts {
        for secret in [KEY, TOKEN, URL_TOKEN] {
            assert!(
                !text.contains(secret),
                "Debug output shows {secret}: {text}"
            );
        }
    }
    assert_eq!(texts[6], "Deployments { .. }");
    assert_eq!(texts[8], "ManagersGet { .. }");
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
}
