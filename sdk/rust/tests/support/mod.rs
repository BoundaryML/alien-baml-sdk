//! Stub-server driver and JSON conversions for the conformance harness.
//! The generated types have no serde support, so results are converted by hand.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
};

use baml_bridge::{
    BamlValue,
    wire::{self, inbound_map_entry::Key, inbound_value::Value as In},
};
use baml_sdk::{compat, platform};
use serde_json::{Map, Value, json};

/// Refuses to run without a BAML_HOME other than ~/.baml: the runtime keeps
/// state there, and tests must not touch the developer's.
pub fn require_isolated_baml_home() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let value = PathBuf::from(std::env::var_os("BAML_HOME").unwrap_or_default());
    assert!(
        !value.as_os_str().is_empty() && value != home.join(".baml"),
        "BAML_HOME must name an isolated directory (not ~/.baml) before the tests load BAML.\n\
         Run every suite with scripts/verify.sh, or this one by hand with a fresh home:\n  \
         (cd sdk/rust && BAML_HOME=\"$(mktemp -d)\" BAML_BRIDGE_PATH=<libbridge_cffi> cargo test)\n\
         (conformance/README.md, \"Running\")"
    );
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Scenarios with phase <= CONFORMANCE_PHASE (every phase when unset).
pub fn load_scenarios() -> Vec<Value> {
    let max_phase: f64 = std::env::var("CONFORMANCE_PHASE")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.parse().expect("CONFORMANCE_PHASE must be a number"))
        .unwrap_or(f64::INFINITY);
    let dir = repo_root().join("conformance").join("scenarios");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("read scenarios dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            serde_json::from_str::<Value>(&std::fs::read_to_string(&p).expect("read scenario"))
                .expect("scenario JSON")
        })
        .filter(|s| s["phase"].as_f64().expect("phase") <= max_phase)
        .collect()
}

pub struct Stub {
    child: Child,
    addr: String,
    connection: Mutex<Option<BufReader<TcpStream>>>,
}

impl Stub {
    pub fn start() -> Stub {
        let mut child = Command::new("node")
            .arg(
                repo_root()
                    .join("conformance")
                    .join("stub-server")
                    .join("server.mjs"),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the stub server (is node on PATH?)");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stub stdout"))
            .read_line(&mut line)
            .expect("read stub READY line");
        let base = line
            .trim()
            .strip_prefix("READY ")
            .unwrap_or_else(|| panic!("unexpected stub output: {line:?}"))
            .to_string();
        let addr = base.strip_prefix("http://").expect("http base").to_string();
        Stub {
            child,
            addr,
            connection: Mutex::new(None),
        }
    }

    /// POSTs JSON to a control endpoint; panics on any non-200 answer. One
    /// kept-alive connection serves every request: a connection per request
    /// leaves a TIME_WAIT socket each, and runs in parallel on one machine
    /// then fail to connect (EADDRNOTAVAIL).
    pub fn post(&self, path: &str, body: &Value) -> Value {
        let payload = body.to_string();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
            self.addr,
            payload.len()
        );
        let mut connection = self.connection.lock().expect("stub connection");
        // The stub closes a connection idle for 5 s: reconnect once.
        let (status, body) = Self::exchange(&mut connection, &self.addr, &request)
            .or_else(|_| {
                *connection = None;
                Self::exchange(&mut connection, &self.addr, &request)
            })
            .expect("talk to the stub");
        assert!(
            status.starts_with("HTTP/1.1 200"),
            "stub {path} failed: {status}\n{body}"
        );
        serde_json::from_str(&body).expect("stub JSON")
    }

    fn exchange(
        connection: &mut Option<BufReader<TcpStream>>,
        addr: &str,
        request: &str,
    ) -> io::Result<(String, String)> {
        let reader = match connection {
            Some(reader) => reader,
            None => connection.insert(BufReader::new(TcpStream::connect(addr)?)),
        };
        reader.get_mut().write_all(request.as_bytes())?;
        let mut status = String::new();
        if reader.read_line(&mut status)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the stub closed the connection",
            ));
        }
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value
                    .trim()
                    .parse()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let body =
            String::from_utf8(body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok((status.trim_end().to_string(), body))
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Arguments: scenario JSON -> generated types.

pub fn s(v: &Value) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("expected a string, got {v}"))
        .to_string()
}

/// `None` for a missing or `null` value, else `f` of it.
fn opt<T>(v: &Value, f: impl FnOnce(&Value) -> T) -> Option<T> {
    if v.is_null() { None } else { Some(f(v)) }
}

fn opt_s(v: &Value) -> Option<String> {
    opt(v, s)
}

/// Fails on any key of the object `v` that the caller does not map, so a
/// scenario field the Rust harness does not know cannot be dropped silently.
fn only_keys(v: &Value, mapped: &[&str]) {
    let unmapped: Vec<&String> = v
        .as_object()
        .expect("object")
        .keys()
        .filter(|k| !mapped.contains(&k.as_str()))
        .collect();
    assert!(
        unmapped.is_empty(),
        "tests/support/mod.rs does not map {unmapped:?}: add them"
    );
}

pub fn string_map(v: &Value) -> baml_bridge::Map<String, String> {
    v.as_object()
        .expect("object")
        .iter()
        .map(|(k, v)| (k.clone(), s(v)))
        .collect()
}

pub fn client_args(v: &Value) -> (String, String, Option<String>) {
    (s(&v["base_url"]), s(&v["api_key"]), opt_s(&v["workspace"]))
}

/// The fields the scenarios set. Not every scenario checks the encoded body
/// (`expect.json`), so any other key fails here.
pub fn create_setup_link_request(v: &Value) -> platform::CreateSetupLinkRequest {
    only_keys(
        v,
        &[
            "externalId",
            "name",
            "project",
            "maxDeployments",
            "description",
            "expiresAt",
            "deploymentSetupConfig",
            "entryPoint",
            "recoveryDeploymentGroupId",
        ],
    );
    platform::CreateSetupLinkRequest {
        externalId: s(&v["externalId"]),
        name: s(&v["name"]),
        project: s(&v["project"]),
        maxDeployments: opt(&v["maxDeployments"], |n| {
            n.as_i64()
                .unwrap_or_else(|| panic!("maxDeployments must be an integer, got {n}"))
        }),
        returnUrl: None,
        refreshUrl: None,
        description: opt_s(&v["description"]),
        expiresAt: opt_s(&v["expiresAt"]),
        deploymentSetupConfig: opt(&v["deploymentSetupConfig"], |config| {
            only_keys(config, &["metadata"]);
            platform::DeploymentSetupConfigInput {
                // A `json` map: serde_json values go to the request as they are.
                metadata: opt(&config["metadata"], |m| {
                    m.as_object()
                        .expect("metadata object")
                        .clone()
                        .into_iter()
                        .collect()
                }),
                policy: None,
                environmentVariables: None,
                validatedReleaseSelection: None,
                publicSubdomain: None,
            }
        }),
        setupItems: None,
        entryPoint: opt(&v["entryPoint"], |entry| {
            only_keys(entry, &["item", "provider"]);
            platform::SetupLinkEntryPoint {
                item: s(&entry["item"]),
                provider: opt_s(&entry["provider"]),
            }
        }),
        inputValues: None,
        recoveryDeploymentGroupId: opt_s(&v["recoveryDeploymentGroupId"]),
    }
}

pub fn bootstrap_request(v: &Value) -> platform::CommandBootstrapRequest {
    if v["role"] == "receiver" {
        platform::CommandBootstrapRequestReceiver {
            deploymentId: s(&v["deploymentId"]),
            role: s(&v["role"]),
            target: s(&v["target"]),
        }
        .into()
    } else {
        platform::CommandBootstrapRequestSender {
            deploymentId: s(&v["deploymentId"]),
            role: s(&v["role"]),
        }
        .into()
    }
}

pub fn launch_input(v: &Value) -> baml_sdk::setup::CloudFormationLaunch {
    baml_sdk::setup::CloudFormationLaunch {
        templateUrl: s(&v["templateUrl"]),
        region: s(&v["region"]),
        stackName: s(&v["stackName"]),
        parameters: string_map(&v["parameters"]),
    }
}

pub fn key_setup_action(v: &Value) -> compat::KeySetupAction {
    compat::KeySetupAction {
        action: s(&v["action"]),
    }
}

// ---------------------------------------------------------------------------
// Results: generated values -> JSON, through the bridge's wire encoding (so
// every generated type works without per-type code). Field names are the BAML
// names, which are the wire names for every type the scenarios return.

pub fn to_json<T: BamlValue>(value: &T) -> Value {
    wire_json(&value.to_baml())
}

fn wire_json(value: &wire::InboundValue) -> Value {
    let entries = |entries: &[wire::InboundMapEntry]| {
        let mut out = Map::new();
        for entry in entries {
            let key = match entry.key.as_ref().expect("map key") {
                Key::StringKey(k) => k.clone(),
                Key::IntKey(k) => k.to_string(),
                Key::BoolKey(k) => k.to_string(),
                Key::EnumKey(k) => k.value.clone(),
            };
            out.insert(key, entry.value.as_ref().map_or(Value::Null, wire_json));
        }
        Value::Object(out)
    };
    match &value.value {
        None => Value::Null,
        Some(In::StringValue(v)) | Some(In::BigintValue(v)) => json!(v),
        Some(In::IntValue(v)) => json!(v),
        Some(In::FloatValue(v)) | Some(In::JsNumberValue(v)) => json!(v),
        Some(In::BoolValue(v)) => json!(v),
        Some(In::ListValue(list)) => Value::Array(list.values.iter().map(wire_json).collect()),
        Some(In::MapValue(map)) => entries(&map.entries),
        Some(In::ClassValue(class)) => entries(&class.fields),
        Some(In::EnumValue(e)) => json!(e.value),
        Some(other) => panic!("no JSON form for wire value {other:?}"),
    }
}
