//! Runs every conformance scenario (conformance/scenarios) through the facade.

mod support;

use std::{path::PathBuf, time::Duration};

use alien_sdk::{Alien, AlienManager, Error, Options, manager, platform};
use baml_bridge::{BamlValue, OptionalArg};
use baml_sdk::compat;
use serde_json::{Value, json};
use support::*;

fn alien(client: &Value) -> Alien {
    let (base_url, api_key, workspace) = client_args(client);
    let timeout = client["timeout_ms"].as_u64().map(Duration::from_millis);
    Alien::new(Options {
        api_key: Some(api_key),
        server_url: Some(base_url),
        workspace,
        timeout,
    })
    .expect("scenario clients have an API key")
}

/// The Manager facade, as `AlienManager::new(..).with_timeout(..)` builds it.
fn alien_manager(client: &Value) -> AlienManager {
    let manager = AlienManager::new(
        s(&client["base_url"]),
        client["token"].as_str().map(str::to_string),
    );
    match client["timeout_ms"].as_u64() {
        Some(ms) => manager.with_timeout(Duration::from_millis(ms)),
        None => manager,
    }
}

fn ok<T: BamlValue>(result: Result<T, Error>) -> Result<Value, Error> {
    result.map(|v| to_json(&v))
}

async fn call<T: BamlValue>(
    operation: impl Future<Output = Result<T, baml_bridge::Error<baml_sdk::errors::AlienError>>>,
) -> Result<Value, Error> {
    Ok(to_json(&operation.await?))
}

async fn run(f: &str, a: &Value) -> Result<Value, Error> {
    let json_arg = || a["json"].as_str().expect("json arg").to_string();
    let client = || alien(&a["client"]);
    match f {
        "platform.createSetupLink" => {
            call(platform::setupLinks::create_async(
                client().platform(),
                create_setup_link_request(&a["request"]),
            ))
            .await
        }
        "platform.getProjectTemplateUrls" => {
            call(platform::projects::getTemplateUrls_async(
                client().platform(),
                s(&a["idOrName"]),
            ))
            .await
        }
        "platform.bootstrapCommands" => {
            call(platform::commands::bootstrap_async(
                client().platform(),
                bootstrap_request(&a["request"]),
            ))
            .await
        }
        "platform.getManager" => {
            call(platform::managers::get_async(
                client().platform(),
                s(&a["id"]),
            ))
            .await
        }
        "manager.getCommandStatus" => {
            call(manager::commands::getCommandStatus_async(
                alien_manager(&a["client"]).client(),
                s(&a["command_id"]),
            ))
            .await
        }
        "setup.cloudformationLaunchUrl" => {
            ok(alien_sdk::cloudformation_launch_url(launch_input(&a["input"])).await)
        }
        "setup.launchUrlForProject" => ok(client()
            .setup()
            .launch_url_for_project(
                a["project"].as_str().expect("project"),
                a["region"].as_str().expect("region"),
                a["stackName"].as_str().expect("stackName"),
                string_map(&a["parameters"]),
            )
            .await),
        "compat.acceptKeySetup" => {
            call(compat::accept_key_setup_async(
                client().platform(),
                key_setup_action(&a["request"]),
            ))
            .await
        }
        "compat.decodeProjectPage" => call(compat::decode_project_page_async(json_arg())).await,
        "compat.decodeMaterialization" => {
            call(compat::decode_materialization_async(json_arg())).await
        }
        "compat.storedSecretPresence" => {
            call(compat::stored_secret_presence_async(json_arg())).await
        }
        "compat.reencodeConfig" => call(compat::reencode_config_async(json_arg())).await,
        "compat.roundtripStackByPlatform" => {
            call(compat::roundtrip_stack_by_platform_async(json_arg())).await
        }
        "compat.roundtripEvent" => call(compat::roundtrip_event_async(json_arg())).await,
        "compat.roleScope" => call(compat::role_scope_async(json_arg())).await,
        "compat.decodeRemoteAccess" => call(compat::decode_remote_access_async(json_arg())).await,
        other => panic!("no Rust binding for scenario fn {other}"),
    }
}

// ---------------------------------------------------------------------------
// Generated operations (`call.op`, conformance/README.md), through one generic
// dispatch: `operation` in support/operations.rs, written by
// scripts/operation_scenarios.py, has one row per operation. A row names the
// generated function and its arguments, `req` ones in order, then `opt` ones.
// A required class argument names its type (`req "body": platform::X`): the
// generated function takes it as `impl Borrow<X>`, which leaves the type to
// the caller. The row passes it by reference. Each row's future is boxed:
// one future holding all of them overflows the test thread's stack.

macro_rules! op {
    (@req $a:ident, $name:literal) => { arg($a, $name).await? };
    (@req $a:ident, $name:literal $ty:ty) => { &arg::<$ty>($a, $name).await? };
    (@opt $a:ident, $name:literal) => { opt($a, $name).await? };
    // An operation in a group: platform::managers::get_async.
    ($a:ident, $ns:ident :: $group:ident :: $f:ident $(, $kind:ident $name:literal $(: $ty:ty)?)* $(,)?) => {
        Box::pin(async move {
            Ok(to_json(&$ns::$group::$f(&client::$ns(&$a["client"]) $(, op!(@$kind $a, $name $($ty)?))*).await?))
        }).await
    };
    // An operation without a group: platform::acceptWorkspaceInvitation_async.
    ($a:ident, $ns:ident :: $f:ident $(, $kind:ident $name:literal $(: $ty:ty)?)* $(,)?) => {
        Box::pin(async move {
            Ok(to_json(&$ns::$f(&client::$ns(&$a["client"]) $(, op!(@$kind $a, $name $($ty)?))*).await?))
        }).await
    };
}

include!("support/operations.rs");

/// A broken scenario or binding, never an SDK outcome.
fn harness(message: String) -> Error {
    Error::Runtime(format!("conformance harness: {message}").into())
}

/// Decodes a scenario argument with the SDK's own JSON decoding (BAML's,
/// through a generated helper), so every generated type works, unions included.
async fn arg<T: BamlValue>(a: &Value, name: &str) -> Result<T, Error> {
    let value = a
        .get(name)
        .ok_or_else(|| harness(format!("scenario args have no {name:?}")))?;
    baml_sdk::internal::decode_body_async::<T>(
        value.to_string(),
        200,
        format!("scenario argument {name}"),
    )
    .await
    .map_err(|error| harness(format!("decode scenario argument {name:?}: {error}")))
}

async fn opt<T: BamlValue>(a: &Value, name: &str) -> Result<OptionalArg<T>, Error> {
    Ok(match a.get(name) {
        None => OptionalArg::Unset,
        Some(_) => OptionalArg::Set(arg(a, name).await?),
    })
}

/// The client a generated operation takes first, from the scenario's `client`
/// args. `op!` picks the function named after the operation's namespace.
mod client {
    use super::*;

    pub fn platform(client: &Value) -> platform::PlatformClient {
        let (base_url, api_key, workspace) = client_args(client);
        platform::PlatformClient {
            base_url,
            api_key,
            workspace,
            timeout_ms: client["timeout_ms"].as_i64(),
        }
    }

    pub fn manager(client: &Value) -> manager::ManagerClient {
        manager::ManagerClient {
            base_url: s(&client["base_url"]),
            token: client["token"].as_str().map(str::to_string),
            timeout_ms: client["timeout_ms"].as_i64(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn conformance() {
    require_isolated_baml_home();
    alien_sdk::init().expect("start the BAML runtime");
    let scenarios = load_scenarios();
    assert!(!scenarios.is_empty(), "no scenarios found");
    let stub = Stub::start();
    let out = std::env::var("CONFORMANCE_OUT")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| PathBuf::from(v).join("rust"));
    if let Some(dir) = &out {
        std::fs::create_dir_all(dir).expect("create CONFORMANCE_OUT/rust");
    }

    let mut failures = Vec::new();
    for scenario in &scenarios {
        let id = scenario["id"].as_str().expect("id");
        let loaded = stub.post("/__load", &json!({ "scenario": scenario }));
        let call = &loaded["call"];
        let result = match call["op"].as_str() {
            Some(op) => operation(op, &call["args"]).await,
            None => run(call["fn"].as_str().expect("fn"), &call["args"]).await,
        };
        let outcome = match &result {
            Ok(value) => json!({ "ok": value }),
            Err(Error::Alien(e)) => json!({ "error": {
                "code": e.code, "retryable": e.retryable, "http_status": e.http_status_code, "message": e.message, "context": e.context,
            } }),
            Err(other) => {
                println!("FAIL {id}: {other}");
                failures.push(format!("{id}: {other}"));
                continue;
            }
        };
        let verdict = stub.post("/__result", &outcome);
        if let Some(dir) = &out {
            std::fs::write(
                dir.join(format!("{id}.json")),
                format!("{}\n", verdict["normalized"]),
            )
            .expect("write outcome");
            if scenario["capture_raw"].as_bool() == Some(true) {
                let raw = result
                    .as_ref()
                    .ok()
                    .and_then(|v| v.as_str())
                    .expect("capture_raw needs a string result");
                std::fs::write(dir.join(format!("{id}.txt")), raw).expect("write raw outcome");
            }
        }
        if verdict["ok"] == json!(true) {
            println!("PASS {id}");
        } else {
            println!("FAIL {id}: {}", verdict["problems"]);
            failures.push(format!("{id}: {}", verdict["problems"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} scenarios failed:\n{}",
        failures.len(),
        scenarios.len(),
        failures.join("\n")
    );
    println!("{} scenarios passed", scenarios.len());
}
