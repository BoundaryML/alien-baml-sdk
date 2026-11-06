//! Runs every conformance scenario (conformance/scenarios) through the facade.

mod support;

use std::{path::PathBuf, time::Duration};

use alien_sdk::{Alien, AlienManager, Error, Options};
use baml_bridge::BamlValue;
use alien_api::compat;
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

async fn call<T: BamlValue, E>(operation: impl Future<Output = Result<T, E>>) -> Result<Value, Error>
where
    Error: From<E>,
{
    Ok(to_json(&operation.await?))
}

async fn run(f: &str, a: &Value) -> Result<Value, Error> {
    let json_arg = || a["json"].as_str().expect("json arg").to_string();
    let client = || alien(&a["client"]);
    match f {
        "platform.createSetupLink" => {
            call(
                client()
                    .setup_links()
                    .create(create_setup_link_request(&a["request"]))
                    .send(),
            )
            .await
        }
        "platform.getProjectTemplateUrls" => {
            call(client().projects().get_template_urls(s(&a["idOrName"])).send()).await
        }
        "platform.bootstrapCommands" => {
            call(
                client()
                    .commands()
                    .bootstrap(bootstrap_request(&a["request"]))
                    .send(),
            )
            .await
        }
        "platform.getManager" => {
            call(client().managers().get(s(&a["id"])).send()).await
        }
        "manager.getCommandStatus" => {
            call(
                alien_manager(&a["client"])
                    .commands()
                    .get_command_status(s(&a["command_id"]))
                    .send(),
            )
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
            call(compat::accept_key_setup(
                &platform_client(&a["client"]),
                key_setup_action(&a["request"]),
            ))
            .await
        }
        "compat.decodeProjectPage" => call(compat::decode_project_page(json_arg())).await,
        "compat.decodeMaterialization" => {
            call(compat::decode_materialization(json_arg())).await
        }
        "compat.storedSecretPresence" => {
            call(compat::stored_secret_presence(json_arg())).await
        }
        "compat.reencodeConfig" => call(compat::reencode_config(json_arg())).await,
        "compat.roundtripStackByPlatform" => {
            call(compat::roundtrip_stack_by_platform(json_arg())).await
        }
        "compat.roundtripEvent" => call(compat::roundtrip_event(json_arg())).await,
        "compat.roleScope" => call(compat::role_scope(json_arg())).await,
        "compat.decodeRemoteAccess" => call(compat::decode_remote_access(json_arg())).await,
        other => panic!("no Rust binding for scenario fn {other}"),
    }
}

// ---------------------------------------------------------------------------
// Generated operations (`call.op`, conformance/README.md): `operation` in
// support/operations.rs, which genops writes (gitignored; scripts/generate.sh
// runs it), has one row per operation, calling its builder:
// `alien(..).deployments().list().project(..)…send()`. Required arguments
// are decoded with `arg`, optional ones with `opt` and set when present.

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
    alien_api::internal::decode_body::<T>(
        value.to_string(),
        200,
        format!("scenario argument {name}"),
    )
    .await
    .map_err(|error| harness(format!("decode scenario argument {name:?}: {error}")))
}

/// An optional argument: `None` when the scenario leaves it out.
async fn opt<T: BamlValue>(a: &Value, name: &str) -> Result<Option<T>, Error> {
    Ok(match a.get(name) {
        None => None,
        Some(_) => Some(arg(a, name).await?),
    })
}

/// The generated Platform client, for the internal `compat` functions that
/// take one (the facade keeps its own private).
fn platform_client(client: &Value) -> alien_api::platform::PlatformClient {
    let (base_url, api_key, workspace) = client_args(client);
    alien_api::platform::PlatformClient {
        base_url,
        api_key,
        workspace,
        timeout_ms: client["timeout_ms"].as_i64(),
    }
}

/// Every operation of Alien's TypeScript SDKs (the naming oracle, extracted
/// from their sources) has a builder, and nothing else does: 279 Platform and
/// 53 Manager operations. The conformance scenarios call each one.
#[test]
fn every_operation_has_a_builder() {
    let oracle: Value = serde_json::from_str(
        &std::fs::read_to_string(
            repo_root().join("tools/openapi2baml/tests/fixtures/naming-oracle.json"),
        )
        .expect("read the naming oracle"),
    )
    .expect("oracle JSON");
    let mut expected = std::collections::BTreeSet::new();
    let mut counts = Vec::new();
    for service in ["platform", "manager"] {
        let ops = oracle["operations"][service].as_object().expect("operations");
        let mut count = 0;
        for e in ops.values().filter(|e| e["in_sdk"] == json!(true)) {
            let method = e["method_name"].as_str().expect("method_name");
            expected.insert(match e["group"].as_str() {
                Some(group) => format!("{service}.{group}.{method}"),
                None => format!("{service}.{method}"),
            });
            count += 1;
        }
        counts.push(count);
    }
    let built: std::collections::BTreeSet<String> =
        OPERATIONS.iter().map(|op| op.to_string()).collect();
    assert_eq!(built.len(), OPERATIONS.len(), "an operation is listed twice");
    assert_eq!(counts, [279, 53]);
    assert_eq!(
        built.symmetric_difference(&expected).collect::<Vec<_>>(),
        Vec::<&String>::new()
    );
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
