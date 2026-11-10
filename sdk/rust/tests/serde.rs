//! The generated types under `alien_api`'s `serde` feature, read from and
//! written to the JSON the Alien API sends and accepts. Payloads come from the
//! conformance scenarios, so they are the ones every SDK is tested against.
//!
//! serde is generated Rust and the SDK's own decoding runs in the BAML engine,
//! so `agrees` checks that they read a payload as the same value. Both follow
//! the `@json` attributes the importer puts on classes.
//!
//! Only JSON and union variants are compared, never Rust field names.

use alien_api::baml::json as baml_json;
use alien_sdk::{manager, platform};
use baml_bridge::{BamlValue, OptionalArg};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// The JSON at `pointer` in conformance scenario `name`.
fn scenario_json(name: &str, pointer: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/scenarios")
        .join(format!("{name}.json"));
    let scenario: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    scenario
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{} has no {pointer}", path.display()))
        .clone()
}

/// serde reads `wire` to the value the SDK's own decoding reads.
fn agrees<T: BamlValue + DeserializeOwned + PartialEq + std::fmt::Debug>(wire: &Value) -> T {
    let serde: T = serde_json::from_value(wire.clone()).unwrap();
    let baml = baml_json::to_sync::<T>(wire.clone(), OptionalArg::Unset).unwrap();
    assert_eq!(serde, baml);
    serde
}

/// A request body, as the SDK sends it in `createSetupLink.optional_fields`:
/// required fields, optional ones set, a nested object (`entryPoint`).
fn setup_link_request_wire() -> Value {
    scenario_json(
        "platform.createSetupLink.optional_fields",
        "/exchanges/0/expect/json",
    )
}

#[test]
fn request_reads_wire_json() {
    let wire = setup_link_request_wire();
    let req: platform::CreateSetupLinkRequest = serde_json::from_value(wire.clone()).unwrap();
    // The same through the facade's re-exported serde_json.
    let same: platform::CreateSetupLinkRequest = alien_sdk::serde_json::from_value(wire).unwrap();
    assert_eq!(req, same);
}

/// B13: an optional field that is absent and one that is `null` read the same.
#[test]
fn absent_and_null_optional_fields_read_the_same() {
    let absent =
        json!({"externalId": "customer-0001", "name": "customer-0001", "project": "my-project"});
    let mut nulls = absent.clone();
    for key in [
        "maxDeployments",
        "returnUrl",
        "refreshUrl",
        "entryPoint",
        "deploymentSetupConfig",
    ] {
        nulls[key] = Value::Null;
    }
    let a: platform::CreateSetupLinkRequest = serde_json::from_value(absent).unwrap();
    let b: platform::CreateSetupLinkRequest = serde_json::from_value(nulls).unwrap();
    assert_eq!(a, b);
}

#[test]
fn request_missing_a_required_field_fails() {
    let err = serde_json::from_value::<platform::CreateSetupLinkRequest>(
        json!({"externalId": "customer-0001", "name": "customer-0001"}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("missing field `project`"), "{err}");
}

/// Open enums are strings: a value the spec does not list reads and writes
/// unchanged (`entryPoint.item`, known values `deployment`, `models`, …).
#[test]
fn open_enum_keeps_an_unknown_value() {
    let mut wire = setup_link_request_wire();
    wire["entryPoint"]["item"] = json!("future-item");
    let req: platform::CreateSetupLinkRequest = serde_json::from_value(wire).unwrap();
    assert_eq!(
        serde_json::to_value(&req).unwrap()["entryPoint"]["item"],
        "future-item"
    );
}

/// A union of a string and a list (`setupItems: string | DeploymentSetupItemSelection[]`)
/// reads and writes either member.
#[test]
fn untagged_union_round_trips_each_member() {
    let selection = json!([{"item": "deployment", "required": true, "releaseChannel": "stable", "providerAllowlist": ["aws"]}]);
    for setup_items in [json!("all"), selection] {
        let mut wire = setup_link_request_wire();
        wire["setupItems"] = setup_items.clone();
        let req: platform::CreateSetupLinkRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(
            serde_json::to_value(&req).unwrap()["setupItems"],
            setup_items
        );
    }
}

/// A response, with a nested object (`deploymentGroup`) and an optional field
/// the server sends as `null` (`expiresAt`), round-trips exactly.
#[test]
fn response_round_trips() {
    for name in [
        "platform.createSetupLink.success",
        "platform.createSetupLink.optional_fields",
    ] {
        let wire = scenario_json(name, "/exchanges/0/respond/json");
        let resp: platform::CreateSetupLinkResponse = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&resp).unwrap(), wire, "{name}");
    }
}

/// B13: writing a request leaves out every unset optional field, nested ones
/// included. For some fields `null` means something else ("Null clears the
/// destination; omit to keep it unchanged", docs/gaps.md B13).
#[test]
fn request_writes_only_what_is_set() {
    let wire = setup_link_request_wire();
    let req: platform::CreateSetupLinkRequest = agrees(&wire);
    assert_eq!(serde_json::to_value(&req).unwrap(), wire);
}

/// PINNED (B26): the two request classes with a `baml.ToJson` of their own,
/// which serde does not run, write every unset optional field as `null`. The
/// SDK sends `description` and `error` as `null` too, but leaves `expiresAt`
/// and `statusCode` out.
#[test]
fn pinned_to_json_requests_write_null_for_unset_optional_fields() {
    assert_eq!(
        serde_json::to_value(platform::CreateDeploymentTokenRequest::new()).unwrap(),
        json!({"description": null, "expiresAt": null})
    );
    assert_eq!(
        serde_json::to_value(platform::ReportLogExportDeliveryRequestBody::new(
            "revision-value".into(),
            true
        ))
        .unwrap(),
        json!({"revision": "revision-value", "succeeded": true, "error": null, "statusCode": null})
    );
}

/// B14: `StackByPlatform`'s wire key `test` is the field `test_`.
#[test]
fn reserved_word_field_uses_the_wire_name() {
    let wire = json!({"aws": {"id": "example", "resources": []}, "test": {"id": "t"}});
    let stack: platform::StackByPlatform = agrees(&wire);
    assert_eq!(serde_json::to_value(&stack).unwrap(), wire);
}

/// A field the importer renamed from a snake_case wire name
/// (docs/importer-coverage.md: `vpc_id` → `vpcId`, …).
#[test]
fn snake_case_wire_field_uses_the_wire_name() {
    let wire = json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["subnet-a"], "private_subnet_ids": ["subnet-b"]});
    let network: manager::NetworkSettingsByoVpcAws = agrees(&wire);
    assert_eq!(serde_json::to_value(&network).unwrap(), wire);
}

/// A tagged union picks its member by tag, and an unknown tag reads as the
/// `Other` member, which writes what it read.
#[test]
fn tagged_union_picks_the_member_by_its_tag() {
    let default: manager::NetworkSettings = agrees(&json!({"type": "use-default"}));
    assert!(matches!(default, manager::NetworkSettings::NetworkSettingsUseDefault(_)), "{default:?}");

    let wire = json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["a"], "private_subnet_ids": ["b"]});
    let vpc: manager::NetworkSettings = agrees(&wire);
    assert!(matches!(vpc, manager::NetworkSettings::NetworkSettingsByoVpcAws(_)), "{vpc:?}");
    assert_eq!(serde_json::to_value(&vpc).unwrap(), wire);

    let wire = json!({"type": "future-network-mode", "x": 1});
    let other: manager::NetworkSettings = agrees(&wire);
    assert!(matches!(other, manager::NetworkSettings::NetworkSettingsOther(_)), "{other:?}");
    assert_eq!(serde_json::to_value(&other).unwrap(), wire);
}

/// A known tag whose member does not decode is an error, not an `Other`, and so
/// is an object with no tag.
#[test]
fn a_broken_member_is_an_error() {
    for wire in [json!({"type": "byo-vpc-aws"}), json!({"kind": "no type"})] {
        assert!(serde_json::from_value::<manager::NetworkSettings>(wire.clone()).is_err(), "{wire}");
        assert!(baml_json::to_sync::<manager::NetworkSettings>(wire, OptionalArg::Unset).is_err());
    }
}

/// The same, nested in a request body:
/// `CreateSetupLinkRequest.deploymentSetupConfig.policy.stackSettings.defaults.network`.
#[test]
fn setup_link_request_keeps_a_nested_network_setting() {
    let mut wire = setup_link_request_wire();
    let network = json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["a"], "private_subnet_ids": ["b"]});
    wire["deploymentSetupConfig"] =
        json!({"policy": {"stackSettings": {"defaults": {"network": network}}}});
    let req: platform::CreateSetupLinkRequest = agrees(&wire);
    assert_eq!(serde_json::to_value(&req).unwrap(), wire);
}

/// A union member this SDK does not know (`storage` with service `minio`,
/// scenario `deployments.get.external_binding_new_service`) reads as
/// `ExternalBindingOther`, tagged by two fields.
#[test]
fn unknown_union_member_reads_as_other() {
    let bindings = scenario_json(
        "ops.platform.deployments.get.external_binding_new_service",
        "/exchanges/0/respond/json/stackSettings/externalBindings",
    );
    let s3: platform::ExternalBinding = agrees(&bindings["files"]);
    assert!(matches!(s3, platform::ExternalBinding::ExternalBindingStorageS3(_)), "{s3:?}");
    assert_eq!(serde_json::to_value(&s3).unwrap(), bindings["files"]);

    let minio: platform::ExternalBinding = agrees(&bindings["media"]);
    assert!(matches!(minio, platform::ExternalBinding::ExternalBindingOther(_)), "{minio:?}");
    assert_eq!(serde_json::to_value(&minio).unwrap(), bindings["media"]);
}

/// A whole `Deployment` reads the same both ways, including the class that
/// keeps keys its schema does not list (`stackState.resources.*.config`).
#[test]
fn deployment_reads_and_keeps_extra_keys() {
    let wire = scenario_json(
        "ops.platform.deployments.get.external_binding_new_service",
        "/exchanges/0/respond/json",
    );
    let deployment: platform::Deployment = agrees(&wire);
    let written = serde_json::to_value(&deployment).unwrap();
    let config = "/stackState/resources/resources-key/config";
    assert_eq!(written.pointer(config), wire.pointer(config));
    assert_eq!(written["stackSettings"], wire["stackSettings"]);
}

/// Every `deployments.get` scenario reads the same in serde and in the engine,
/// and a payload one of them refuses, the other refuses too.
#[test]
fn deployment_scenarios_agree() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/scenarios");
    let mut read = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("ops.platform.deployments.get.") {
            continue;
        }
        let scenario: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for exchange in scenario["exchanges"].as_array().into_iter().flatten() {
            let wire = &exchange["respond"]["json"];
            if wire.get("stackSettings").is_none() {
                continue;
            }
            let serde = serde_json::from_value::<platform::Deployment>(wire.clone()).ok();
            let baml = baml_json::to_sync::<platform::Deployment>(wire.clone(), OptionalArg::Unset).ok();
            assert_eq!(serde, baml, "{name}");
            read += usize::from(serde.is_some());
        }
    }
    assert!(read > 0);
}
