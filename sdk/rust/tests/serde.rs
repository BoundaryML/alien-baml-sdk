//! The generated types under `alien_api`'s `serde` feature (BAML #5375), read
//! from and written to the JSON the Alien API sends and accepts. Payloads come
//! from the conformance scenarios, so they are the ones every SDK is tested
//! against.
//!
//! The feature is on for the tests only (`[dev-dependencies]`): `alien_sdk`
//! users do not get it, because the derived impls do not follow the SDK's JSON
//! mapping (docs/gaps.md, B26). They are structural: keys are the BAML field
//! names, an unset optional field is `null`, and a union takes the first member
//! that fits; the `baml.FromJson`/`baml.ToJson` overrides the importer gives
//! classes are not applied.
//!
//! Tests named `pinned_*` pin that KNOWN-WRONG behavior: each says what the
//! SDK itself reads or writes instead. When BAML's generator applies the
//! overrides, they fail; turn them into round trips against the wire JSON
//! then, and reconsider turning the feature on for users.
//!
//! Only JSON and union variants are compared, never Rust field names.

use alien_sdk::{manager, platform};
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
/// reads and writes either member (every optional field set: see
/// `pinned_request_writes_null_for_unset_optional_fields`).
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

/// PINNED, known wrong (B13): writing a request puts `null` in every unset optional field,
/// nested ones included. The SDK leaves them out of the request body (the
/// scenario's wire JSON), and for some fields `null` means something else
/// ("Null clears the destination; omit to keep it unchanged", docs/gaps.md B13).
/// Do not send serde's output to the API as a request body.
#[test]
fn pinned_request_writes_null_for_unset_optional_fields() {
    let wire = setup_link_request_wire();
    let req: platform::CreateSetupLinkRequest = serde_json::from_value(wire).unwrap();
    assert_eq!(
        serde_json::to_value(&req).unwrap(),
        json!({
            "externalId": "customer-0002",
            "name": "customer-0002",
            "project": "my-project",
            "maxDeployments": 5,
            "returnUrl": null,
            "refreshUrl": null,
            "description": null,
            "expiresAt": "2026-11-01T00:00:00Z",
            "deploymentSetupConfig": null,
            "setupItems": null,
            "entryPoint": {"item": "deployment", "provider": null},
            "inputValues": null,
            "recoveryDeploymentGroupId": null
        })
    );
}

/// PINNED, known wrong (B14): `StackByPlatform`'s wire key `test` is the field `test_`, and
/// serde knows only the field name. Reading drops `"test"` without an error;
/// writing uses `"test_"` (and `null`s for the unset platforms). The SDK reads
/// and writes `"test"` (scenario `compat.stack_by_platform.roundtrip`).
#[test]
fn pinned_reserved_word_field_uses_the_baml_name() {
    let wire = json!({"aws": {"id": "example", "resources": []}, "test": {"id": "t"}});
    let stack: platform::StackByPlatform = serde_json::from_value(wire).unwrap();
    assert_eq!(
        serde_json::to_value(&stack).unwrap(),
        json!({
            "aws": {"id": "example", "resources": []},
            "gcp": null, "azure": null, "kubernetes": null, "machines": null, "local": null,
            "test_": null
        })
    );
    let from_baml_name: platform::StackByPlatform =
        serde_json::from_value(json!({"test_": {"id": "t"}})).unwrap();
    assert_eq!(
        serde_json::to_value(&from_baml_name).unwrap()["test_"],
        json!({"id": "t"})
    );
}

/// PINNED, known wrong: a field the importer renamed from a snake_case wire name
/// (docs/importer-coverage.md, 308 fields: `vpc_id` → `vpcId`, …) is read and
/// written under the BAML name. Inside a tagged union this also picks the
/// wrong member (next test).
#[test]
fn pinned_snake_case_wire_field_uses_the_baml_name() {
    let wire = json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["subnet-a"], "private_subnet_ids": ["subnet-b"]});
    let err = serde_json::from_value::<manager::NetworkSettingsByoVpcAws>(wire).unwrap_err();
    assert!(
        err.to_string().contains("missing field `privateSubnetIds`"),
        "{err}"
    );
    let baml_names = json!({"type": "byo-vpc-aws", "vpcId": "vpc-1", "publicSubnetIds": ["subnet-a"], "privateSubnetIds": ["subnet-b"], "securityGroupIds": null});
    let n: manager::NetworkSettingsByoVpcAws = serde_json::from_value(baml_names.clone()).unwrap();
    assert_eq!(serde_json::to_value(&n).unwrap(), baml_names);
}

/// PINNED, known wrong: a tagged union takes its first member that fits. Tags are plain
/// strings in Rust and unlisted keys are ignored, so every `NetworkSettings`
/// reads as its first member, `NetworkSettingsUseDefault`, keeping only
/// `type`: the VPC is lost without an error, whatever the key casing. The SDK
/// picks the member by `type` and reads an unknown one as
/// `NetworkSettingsOther`.
#[test]
fn pinned_tagged_union_reads_as_its_first_member() {
    for wire in [
        json!({"type": "use-default"}),
        json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["a"], "private_subnet_ids": ["b"]}),
        json!({"type": "byo-vpc-aws", "vpcId": "vpc-1", "publicSubnetIds": ["a"], "privateSubnetIds": ["b"]}),
        json!({"type": "future-network-mode"}),
    ] {
        let n: manager::NetworkSettings = serde_json::from_value(wire.clone()).unwrap();
        assert!(
            matches!(n, manager::NetworkSettings::NetworkSettingsUseDefault(_)),
            "{wire} read as {n:?}"
        );
        assert_eq!(
            serde_json::to_value(&n).unwrap(),
            json!({"type": wire["type"]}),
            "{wire}"
        );
    }
}

/// PINNED, known wrong: the same, nested in the request Alien asked about:
/// `deploymentSetupConfig.policy.stackSettings.defaults.network`.
#[test]
fn pinned_setup_link_request_loses_a_nested_network_setting() {
    let mut wire = setup_link_request_wire();
    let network = json!({"type": "byo-vpc-aws", "vpc_id": "vpc-1", "public_subnet_ids": ["a"], "private_subnet_ids": ["b"]});
    wire["deploymentSetupConfig"] =
        json!({"policy": {"stackSettings": {"defaults": {"network": network}}}});
    let req: platform::CreateSetupLinkRequest = serde_json::from_value(wire).unwrap();
    let written = serde_json::to_value(&req).unwrap();
    assert_eq!(
        written.pointer("/deploymentSetupConfig/policy/stackSettings/defaults/network"),
        Some(&json!({"type": "byo-vpc-aws"}))
    );
}

/// PINNED, known wrong: a union member this SDK does not know (`storage` with service
/// `minio`, scenario `deployments.get.external_binding_new_service`) reads as
/// the first known member it fits, `ExternalBindingStorageS3`, not as
/// `ExternalBindingOther`. A known one reads correctly.
#[test]
fn pinned_unknown_union_member_reads_as_a_known_one() {
    let bindings = scenario_json(
        "ops.platform.deployments.get.external_binding_new_service",
        "/exchanges/0/respond/json/stackSettings/externalBindings",
    );
    let s3: platform::ExternalBinding = serde_json::from_value(bindings["files"].clone()).unwrap();
    assert!(
        matches!(s3, platform::ExternalBinding::ExternalBindingStorageS3(_)),
        "{s3:?}"
    );
    let minio: platform::ExternalBinding =
        serde_json::from_value(bindings["media"].clone()).unwrap();
    assert!(
        matches!(
            minio,
            platform::ExternalBinding::ExternalBindingStorageS3(_)
        ),
        "{minio:?}"
    );
    assert_eq!(serde_json::to_value(&minio).unwrap()["service"], "minio");
}

/// PINNED, known wrong: an `…Other` member is written as its two fields. The
/// SDK writes `raw`, the member as the server sent it.
#[test]
fn pinned_other_member_writes_its_fields_not_raw() {
    let fields = json!({"type": "UNKNOWN", "raw": {"type": "storage", "service": "minio", "bucketName": "media"}});
    let other: platform::ExternalBindingOther = serde_json::from_value(fields.clone()).unwrap();
    assert_eq!(serde_json::to_value(&other).unwrap(), fields);
}

/// PINNED, known wrong: a class that keeps keys its schema does not list
/// (`additionalProperties`, 34 classes) expects them under an
/// `"additionalProperties"` key, so wire JSON fails to read. Here a whole
/// `Deployment` response fails on one such class deep inside it
/// (`stackState.resources.*.config`).
#[test]
fn pinned_additional_properties_class_rejects_wire_json() {
    let wire = scenario_json(
        "ops.platform.deployments.get.external_binding_new_service",
        "/exchanges/0/respond/json",
    );
    let err = serde_json::from_value::<platform::Deployment>(wire).unwrap_err();
    assert!(
        err.to_string()
            .contains("missing field `additionalProperties`"),
        "{err}"
    );
}
