//! The importer names every operation, argument and field as Alien's
//! TypeScript SDK does, and makes its arguments optional exactly where that SDK
//! does.
//!
//! The oracle (`tests/fixtures/naming-oracle.json`) was extracted from the
//! incumbent SDK's sources by `scripts/extract_naming_oracle.py`, not from the
//! spec hints the importer reads, so this test checks the importer's rules
//! against what Alien's users actually call.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use openapi2baml::{
    config::Config,
    coverage::Coverage,
    lower::{Def, Lowerer},
    naming::{self, RESERVED_FIELD_NAMES, RESERVED_PARAM_NAMES},
    ops,
    spec::Spec,
};
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn sdk_name(entry: &Value) -> (Option<String>, String) {
    (entry["group"].as_str().map(str::to_string), entry["method_name"].as_str().expect("method_name").to_string())
}

/// The SDK method can be called without arguments: every parameter is optional.
fn sdk_callable_without_arguments(entry: &Value) -> bool {
    entry["params"].as_array().expect("params").iter().all(|p| p["optional"] == true)
}

#[test]
fn names_match_the_typescript_sdk() {
    let oracle = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming-oracle.json"));
    let config: Config = serde_json::from_value(read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("config.json")))
        .expect("config.json");
    let mut problems = Vec::new();
    for service in &config.services {
        let name = service.name.as_str();
        let spec = Spec::load(&repo(), &service.spec, &service.overlays).expect("load spec");
        let mismatches = &oracle["spec_vs_sdk_mismatches"][name];

        // The SDK's name for every operation it has, including the ones only the
        // compatibility overlay declares.
        let mut expected: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
        let mut optional: BTreeMap<String, bool> = BTreeMap::new();
        let in_sdk = oracle["operations"][name]
            .as_object()
            .expect("oracle operations")
            .iter()
            .filter(|(_, entry)| entry["in_sdk"] == true)
            .map(|(id, entry)| (id.clone(), entry));
        let sdk_only = mismatches["sdk_only"]
            .as_array()
            .expect("sdk_only")
            .iter()
            .map(|entry| (entry["operation_id"].as_str().expect("operation_id").to_string(), entry));
        for (id, entry) in in_sdk.chain(sdk_only) {
            expected.insert(id.clone(), sdk_name(entry));
            optional.insert(id, sdk_callable_without_arguments(entry));
        }
        // Spec operations the SDK lacks: ignored on purpose, or newer than the SDK.
        let (ignored, newer): (Vec<&Value>, Vec<&Value>) = mismatches["spec_only"]
            .as_array()
            .expect("spec_only")
            .iter()
            .partition(|entry| entry["x-speakeasy-ignore"] == true);
        let id_of = |entry: &&Value| entry["operation_id"].as_str().expect("operation_id").to_string();
        let ignored: BTreeSet<String> = ignored.iter().map(id_of).collect();
        let newer: BTreeSet<String> = newer.iter().map(id_of).collect();

        let mut coverage = Coverage::default();
        let mut lowerer = Lowerer::new(&spec, config.open_enums, &mut coverage);
        let mut imported = BTreeSet::new();
        for operation in ops::all_operations(&spec).expect("operations") {
            let lowered = ops::lower_operation(&mut lowerer, service, &spec, &operation).expect("lower");
            let (id, method, path, op) = operation;
            let actual = ops::sdk_name(service, &id, &method, &path, &op);
            if let Some(&want) = optional.get(&id) {
                // The generated function can be called with the client alone.
                let ours = lowered.args.iter().all(|a| !a.required);
                if ours != want {
                    problems.push(format!(
                        "{name}.{id}: callable without arguments: importer {ours}, TypeScript SDK {want}"
                    ));
                }
            }
            match expected.get(&id) {
                Some(want) if *want != actual => {
                    problems.push(format!("{name}.{id}: importer {actual:?}, TypeScript SDK {want:?}"))
                }
                Some(_) => {}
                None if newer.contains(&id) => {}
                None => problems.push(format!("{name}.{id}: imported, but neither in the SDK nor known to be newer")),
            }
            imported.insert(id);
        }
        for id in expected.keys().filter(|id| !imported.contains(*id)) {
            problems.push(format!("{name}.{id}: in the TypeScript SDK, not imported"));
        }
        for id in ignored.iter().filter(|id| imported.contains(*id)) {
            problems.push(format!("{name}.{id}: x-speakeasy-ignore, but imported"));
        }
        assert!(imported.len() > 50, "{name}: only {} operations imported", imported.len());
    }
    assert!(problems.is_empty(), "{} naming problems:\n{}", problems.len(), problems.join("\n"));
}

/// Field names: the SDK camelCases a wire name with separators
/// (`custom_domains` → `customDomains`, `on-other` → `onOther`) and keeps any
/// other. The oracle's `field_names` lists every response field the SDK
/// renames or whose wire name has a separator, with the SDK's name.
#[test]
fn field_names_match_the_typescript_sdk() {
    let oracle = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming-oracle.json"));
    let config: Config = serde_json::from_value(read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("config.json")))
        .expect("config.json");
    let mut problems = Vec::new();
    for service in &config.services {
        let name = service.name.as_str();
        let spec = Spec::load(&repo(), &service.spec, &service.overlays).expect("load spec");
        let sdk_names = oracle["field_names"][name].as_object().expect("oracle field_names");
        let mut coverage = Coverage::default();
        let mut lowerer = Lowerer::new(&spec, config.open_enums, &mut coverage);
        for operation in ops::all_operations(&spec).expect("operations") {
            ops::lower_operation(&mut lowerer, service, &spec, &operation).expect("lower");
        }
        let mut seen = BTreeSet::new();
        for def in lowerer.defs.values() {
            let Def::Class(class) = def else { continue };
            for field in &class.fields {
                let want = match sdk_names.get(&field.wire) {
                    Some(sdk) => naming::identifier(sdk.as_str().expect("SDK name"), RESERVED_FIELD_NAMES),
                    // Kept by the SDK, or newer than it: the importer's rule.
                    None => naming::field_name(&field.wire),
                };
                if field.name != want {
                    problems.push(format!("{name}.{}.{}: importer `{}`, TypeScript SDK `{want}`", class.name, field.wire, field.name));
                }
                if sdk_names.contains_key(&field.wire) {
                    seen.insert(field.wire.clone());
                }
                // A name the SDK keeps is kept, apart from reserved words.
                let plain = field.wire.chars().all(|c| c.is_ascii_alphanumeric());
                if plain && field.name != naming::identifier(&field.wire, RESERVED_FIELD_NAMES) {
                    problems.push(format!("{name}.{}.{}: renamed to `{}`", class.name, field.wire, field.name));
                }
            }
        }
        for wire in sdk_names.keys().filter(|w| !seen.contains(*w)) {
            problems.push(format!("{name}: the TypeScript SDK has the field `{wire}`, no imported class does"));
        }
        assert!(!seen.is_empty(), "{name}: no field of the oracle's field_names imported");
    }
    assert!(problems.is_empty(), "{} field naming problems:\n{}", problems.len(), problems.join("\n"));
}

/// Argument names: the SDK camelCases a parameter's wire name with separators
/// (`command_id` → `commandId`, `if-match` → `ifMatch`) and keeps any other.
/// The oracle's `param_names` gives the SDK's name for every parameter of
/// every operation it has (its request type's keys).
#[test]
fn argument_names_match_the_typescript_sdk() {
    let oracle = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/naming-oracle.json"));
    let config: Config = serde_json::from_value(read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("config.json")))
        .expect("config.json");
    let mut problems = Vec::new();
    let mut renamed = 0;
    for service in &config.services {
        let name = service.name.as_str();
        let spec = Spec::load(&repo(), &service.spec, &service.overlays).expect("load spec");
        let mut coverage = Coverage::default();
        let mut lowerer = Lowerer::new(&spec, config.open_enums, &mut coverage);
        let mut checked = 0;
        for operation in ops::all_operations(&spec).expect("operations") {
            let lowered = ops::lower_operation(&mut lowerer, service, &spec, &operation).expect("lower");
            let id = &operation.0;
            // Operations newer than the SDK follow the importer's rule.
            let sdk_names = oracle["operations"][name][id]["param_names"].as_object();
            for arg in &lowered.args {
                let Some(wire) = &arg.wire else { continue };
                let want = match sdk_names {
                    Some(names) => match names.get(wire) {
                        Some(sdk) => naming::identifier(sdk.as_str().expect("SDK name"), RESERVED_PARAM_NAMES),
                        None => {
                            problems.push(format!("{name}.{id}: parameter `{wire}` is not in the TypeScript SDK's request"));
                            continue;
                        }
                    },
                    None => naming::argument_name(wire),
                };
                if arg.name != want {
                    problems.push(format!("{name}.{id}.{wire}: importer `{}`, TypeScript SDK `{want}`", arg.name));
                }
                if arg.name != *wire {
                    renamed += 1;
                }
                checked += 1;
            }
            for wire in sdk_names.into_iter().flat_map(|names| names.keys()) {
                if !lowered.args.iter().any(|a| a.wire.as_ref() == Some(wire)) {
                    problems.push(format!("{name}.{id}: the TypeScript SDK has the parameter `{wire}`, the import does not"));
                }
            }
        }
        assert!(checked > 30, "{name}: only {checked} arguments checked");
    }
    assert!(problems.is_empty(), "{} argument naming problems:\n{}", problems.len(), problems.join("\n"));
    assert!(renamed > 0, "no argument renamed: the oracle no longer covers the rule");
}
