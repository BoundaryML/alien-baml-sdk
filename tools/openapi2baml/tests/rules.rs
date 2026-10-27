//! Import rules (DESIGN.md), each checked on a small spec.

use std::path::Path;

use openapi2baml::{config::Config, run};
use serde_json::{Value, json};

/// Imports `paths` and `schemas` as one service; returns its generated files
/// (`types.baml`, `operations.baml`, the coverage report) or the import error.
fn import(name: &str, paths: Value, schemas: Value) -> Result<Files, String> {
    import_with(json!({}), name, paths, schemas)
}

/// [`import`] with these config settings (`open_enums`, `open_unions`).
fn import_with(settings: Value, name: &str, paths: Value, schemas: Value) -> Result<Files, String> {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&root).expect("create test root");
    let spec = json!({"openapi": "3.1.0", "paths": paths, "components": {"schemas": schemas}});
    std::fs::write(root.join("openapi.json"), spec.to_string()).expect("write spec");
    let mut config = json!({
        "open_enums": "all",
        "coverage": "coverage.md",
        "services": [{
            "name": "platform", "spec": "openapi.json", "operations": "all", "out": "out",
            "client": "platform", "naming": "first-tag",
        }],
    });
    for (key, value) in settings.as_object().expect("settings") {
        config[key] = value.clone();
    }
    let config: Config = serde_json::from_value(config).expect("config");
    let mut files = run(&root, &config).map_err(|e| e.to_string())?.files;
    let mut take = |path: &str| files.remove(path).unwrap_or_else(|| panic!("{path} not generated"));
    Ok(Files { types: take("out/types.baml"), operations: take("out/operations.baml"), coverage: take("coverage.md") })
}

struct Files {
    types: String,
    operations: String,
    coverage: String,
}

/// `GET <path>` (operation `id`) returning `schema`.
fn get(id: &str, schema: Value) -> Value {
    json!({"get": {"operationId": id, "responses": {"200": {"content": {"application/json": {"schema": schema}}}}}})
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

#[test]
fn literals_stay_only_where_they_tag_a_union_member() {
    let pet = json!({"oneOf": [{"$ref": "#/components/schemas/Cat"}, {"$ref": "#/components/schemas/Dog"}]});
    let info = object(
        json!({
            "schemaVersion": {"type": "integer", "enum": [1]},
            "via": {"type": "string", "enum": ["mcp"], "description": "Who asked."},
            "state": {"oneOf": [{"type": "string", "enum": ["none"]}, object(json!({"failed": {"type": "string"}}), &["failed"])]},
        }),
        &["schemaVersion", "state"],
    );
    let files = import(
        "literals",
        json!({"/pet": get("getPet", pet), "/info": get("getInfo", info)}),
        json!({
            "Cat": object(json!({"kind": {"type": "string", "enum": ["cat"]}, "purrs": {"type": "boolean"}}), &["kind"]),
            "Dog": object(json!({"kind": {"const": "dog"}, "barks": {"type": "boolean"}}), &["kind"]),
        }),
    )
    .expect("import");
    let types = &files.types;
    assert!(types.contains("    kind: \"cat\",\n"), "{types}");
    assert!(types.contains("    kind: \"dog\",\n"), "{types}");
    assert!(types.contains("    /// Known values: `1`. Others may appear.\n    schemaVersion: int,\n"), "{types}");
    assert!(types.contains("    /// Who asked.\n    ///\n    /// Known values: `mcp`. Others may appear.\n    via: string?,\n"), "{types}");
    assert!(types.contains("    /// Known values: `none`. Others may appear.\n    state: string | GetInfoResponseState2,\n"), "{types}");
}

#[test]
fn enum_policies_none_and_rust_filter_keep_enums_closed() {
    // The two fields Alien's Rust filter opens, and two it does not.
    let schemas = json!({
        "DeploymentLinkSetupResponse": object(
            json!({"visiblePackageTypes": {"type": "array", "items": {"type": "string", "enum": ["cli", "helm"]}}}),
            &["visiblePackageTypes"],
        ),
        "CapabilityMaterialization": object(
            json!({"packages": {"type": "array", "items": object(json!({"type": {"type": "string", "enum": ["a", "b"]}}), &["type"])}}),
            &["packages"],
        ),
    });
    let info = object(
        json!({"status": {"type": "string", "enum": ["ok", "failed"]}, "via": {"type": "string", "enum": ["mcp"]}}),
        &["status"],
    );
    let paths = json!({
        "/link": get("getLink", json!({"$ref": "#/components/schemas/DeploymentLinkSetupResponse"})),
        "/capability": get("getCapability", json!({"$ref": "#/components/schemas/CapabilityMaterialization"})),
        "/info": get("getInfo", info),
        // A closed enum component is a type alias; as a parameter it is a `string`.
        "/list": {"get": {
            "operationId": "list",
            "parameters": [{"name": "purpose", "in": "query", "schema": {"$ref": "#/components/schemas/Purpose"}}],
            "responses": {"204": {"description": "done"}},
        }},
    });
    let mut schemas = schemas;
    schemas["Purpose"] = json!({"type": "string", "enum": ["x", "y"]});
    let types = |policy: &str| {
        let files =
            import_with(json!({"open_enums": policy}), &format!("policy-{policy}"), paths.clone(), schemas.clone()).expect("import");
        assert!(files.operations.contains("purpose: string? = null"), "{policy}: {}", files.operations);
        files.types
    };

    let all = types("all");
    for line in ["visiblePackageTypes: string[],", "type: string,", "status: string,", "via: string?,"] {
        assert!(all.contains(line), "all: {line}\n{all}");
    }
    let rust_filter = types("rust-filter");
    for line in ["visiblePackageTypes: string[],", "type: string,", "status: \"ok\" | \"failed\",", "via: \"mcp\"?,"] {
        assert!(rust_filter.contains(line), "rust-filter: {line}\n{rust_filter}");
    }
    let none = types("none");
    for line in ["visiblePackageTypes: (\"cli\" | \"helm\")[],", "type: \"a\" | \"b\",", "status: \"ok\" | \"failed\",", "via: \"mcp\"?,"] {
        assert!(none.contains(line), "none: {line}\n{none}");
    }

    // "rust-filter" fails, as Alien's filter does, when a field it opens is no longer an enum.
    let mut changed = schemas.clone();
    changed["DeploymentLinkSetupResponse"]["properties"]["visiblePackageTypes"]["items"] = json!({"type": "string"});
    let error = import_with(json!({"open_enums": "rust-filter"}), "policy-rust-filter-stale", paths, changed).err().expect("an error");
    assert!(error.contains("/components/schemas/DeploymentLinkSetupResponse/properties/visiblePackageTypes/items is missing or not an enum"), "{error}");
}

#[test]
fn open_unions_get_an_other_member_for_tags_this_sdk_does_not_know() {
    let paths = json!({
        "/pets": get("listPets", json!({"type": "array", "items": {"$ref": "#/components/schemas/Pet"}})),
        "/home": get("getHome", json!({"$ref": "#/components/schemas/Home"})),
        "/shape": get("getShape", json!({"oneOf": [{"type": "string"}, {"$ref": "#/components/schemas/Cat"}]})),
    });
    let schemas = json!({
        "Pet": {"oneOf": [{"$ref": "#/components/schemas/Cat"}, {"$ref": "#/components/schemas/Dog"}]},
        "Cat": object(json!({"kind": {"type": "string", "enum": ["cat"]}, "purrs": {"type": "boolean"}}), &["kind"]),
        "Dog": object(json!({"kind": {"const": "dog"}, "barks": {"type": "boolean"}}), &["kind"]),
        "Home": object(json!({"pet": {"$ref": "#/components/schemas/Pet"}, "name": {"type": "string"}}), &["pet", "name"]),
    });
    let closed = import("closed-unions", paths.clone(), schemas.clone()).expect("import").types;
    assert!(closed.contains("type Pet = Cat | Dog\n"), "{closed}");

    let files = import_with(json!({"open_unions": true}), "open-unions", paths.clone(), schemas.clone()).expect("import");
    let types = files.types;
    // The Other member's tag is the literal "UNKNOWN"; raw keeps the member and is what it encodes as.
    assert!(types.contains("type Pet = Cat | Dog | PetOther\n"), "{types}");
    assert!(types.contains("class PetOther {\n    /// Always `\"UNKNOWN\"`. The server's `kind` is `raw[\"kind\"]`.\n    kind: \"UNKNOWN\",\n"), "{types}");
    assert!(types.contains("    raw: map<string, json>,\n"), "{types}");
    assert!(types.contains("            baml.json.from(self.raw)\n"), "{types}");
    // A class holding an open union decodes it by its tag: a known tag as its member, any other as Other.
    let pet = "pet: match (baml.json.field(object, \"pet\")) {
    let member: map<string, baml.json.json> => match (baml.json.to<string>(baml.json.field(member, \"kind\"))) {
        \"cat\" => baml.json.to<Cat>(member),
        \"dog\" => baml.json.to<Dog>(member),
        _ => PetOther { kind: \"UNKNOWN\", raw: member },
    },
    _ => throw baml.json.DecodeError { message: \"expected a JSON object\", path: \"\" },
},"
    .replace('\n', "\n                    ");
    assert!(types.contains(&pet), "{types}");
    assert!(types.contains("name: baml.json.to<string>(baml.json.field(object, \"name\")),"), "{types}");
    // So does an operation returning one, here in a list, with a `decode` function.
    let operations = files.operations;
    assert!(operations.contains("    root.internal.platform_decoded<Pet[]>(\n        client, \"GET\", \"/pets\",\n        decode = (response) -> {\n"), "{operations}");
    assert!(operations.contains("let items: baml.json.json[] => items.map((item) -> {\n"), "{operations}");
    assert!(operations.contains("\"dog\" => baml.json.to<Dog>(member),"), "{operations}");
    // A union with a member that is not a class has no tag: it stays closed.
    assert!(!types.contains("GetShapeResponseOther"), "{types}");

    // A response that may be empty cannot hold an open union.
    let mut maybe = paths.clone();
    maybe["/maybe"] = json!({"get": {"operationId": "getMaybe", "responses": {
        "200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pet"}}}},
        "204": {"description": "nothing"},
    }}});
    let error = import_with(json!({"open_unions": true}), "open-unions-optional", maybe, schemas.clone()).err().expect("an error");
    assert!(error.contains("(operation getMaybe): a response that may be empty and holds an open union is not implemented"), "{error}");

    // A member tagged "UNKNOWN" would be indistinguishable from the Other member.
    let mut clash = schemas;
    clash["Dog"]["properties"]["kind"] = json!({"const": "UNKNOWN"});
    let error = import_with(json!({"open_unions": true}), "open-unions-clash", paths, clash).err().expect("an error");
    assert!(error.contains("a member's `kind` is \"UNKNOWN\", the tag of the union's Other member"), "{error}");
}

#[test]
fn open_unions_tagged_by_several_fields_get_one_other_member() {
    // As Alien's `ExternalBinding`: no one field tells the members apart. They
    // share a `kind`, a `service` tells those apart, and `Ai` has no `service`.
    let paths = json!({
        "/binding": get("getBinding", json!({"$ref": "#/components/schemas/Binding"})),
        "/stack": get("getStack", json!({"$ref": "#/components/schemas/Stack"})),
    });
    let schemas = json!({
        "Binding": {"oneOf": [
            {"$ref": "#/components/schemas/S3"}, {"$ref": "#/components/schemas/Gcs"},
            {"$ref": "#/components/schemas/Redis"}, {"$ref": "#/components/schemas/Ai"},
        ]},
        "S3": object(json!({"kind": {"const": "storage"}, "service": {"const": "s3"}, "bucket": {"type": "string"}}), &["kind", "service", "bucket"]),
        "Gcs": object(json!({"kind": {"const": "storage"}, "service": {"const": "gcs"}, "bucket": {"type": "string"}}), &["kind", "service", "bucket"]),
        "Redis": object(json!({"kind": {"const": "kv"}, "service": {"const": "redis"}, "url": {"type": "string"}}), &["kind", "service", "url"]),
        "Ai": object(json!({"kind": {"const": "ai"}, "provider": {"type": "string"}}), &["kind", "provider"]),
        "Stack": object(json!({"bindings": {"type": "object", "additionalProperties": {"$ref": "#/components/schemas/Binding"}}}), &[]),
    });
    let files = import_with(json!({"open_unions": true}), "open-unions-two-tags", paths.clone(), schemas.clone()).expect("import");
    let types = files.types;
    // One Other member, shaped as for one tag field: the first tag field is "UNKNOWN".
    assert!(types.contains("type Binding = S3 | Gcs | Redis | Ai | BindingOther\n"), "{types}");
    assert!(types.contains("    /// Always `\"UNKNOWN\"`. The server's `kind` is `raw[\"kind\"]`, its `service` `raw[\"service\"]`.\n    kind: \"UNKNOWN\",\n    /// The member as the server sent it, every key included.\n    raw: map<string, json>,\n"), "{types}");
    assert!(types.contains("Members are told apart by `kind`, then `service`"), "{types}");
    assert_eq!(types.matches("Other {\n").count(), 1, "{types}");
    // Decoding reads `kind`, then `service` where members share the `kind`. A
    // `service` no member of that `kind` has, or a new `kind`, is the Other member.
    let operations = files.operations;
    let select = "match (baml.json.to<string>(baml.json.field(member, \"kind\"))) {
    \"storage\" => match (baml.json.to<string>(baml.json.field(member, \"service\"))) {
        \"s3\" => baml.json.to<S3>(member),
        \"gcs\" => baml.json.to<Gcs>(member),
        _ => BindingOther { kind: \"UNKNOWN\", raw: member },
    },
    \"kv\" => match (baml.json.to<string>(baml.json.field(member, \"service\"))) {
        \"redis\" => baml.json.to<Redis>(member),
        _ => BindingOther { kind: \"UNKNOWN\", raw: member },
    },
    \"ai\" => baml.json.to<Ai>(member),
    _ => BindingOther { kind: \"UNKNOWN\", raw: member },
}";
    // A member alone with its `kind` and without a `service` (`Ai`) is picked by its `kind`.
    assert!(operations.contains(&select.replace('\n', &format!("\n{}", " ".repeat(16)))), "{operations}");
    // A class holding the union decodes it the same way, here as map values.
    assert!(types.contains("out[key] = match (entries[key]) {"), "{types}");
    assert!(types.contains(&select.replace('\n', &format!("\n{}", " ".repeat(36)))), "{types}");

    // When the fields cannot tell every member apart, the union stays closed:
    // here `Ai` shares the `kind` "storage" but has no `service`.
    let mut ambiguous = schemas;
    ambiguous["Ai"]["properties"]["kind"] = json!({"const": "storage"});
    let closed = import_with(json!({"open_unions": true}), "open-unions-two-tags-ambiguous", paths, ambiguous).expect("import").types;
    assert!(closed.contains("type Binding = S3 | Gcs | Redis | Ai\n"), "{closed}");
    assert!(!closed.contains("BindingOther"), "{closed}");
}

#[test]
fn all_of_distributes_over_a_union_behind_a_ref() {
    // `allOf: [$ref: Storage, {type: "storage"}]` with `Storage: oneOf [S3, Gcs]`
    // is `S3 ∧ {type} | Gcs ∧ {type}`, as when the union is inline.
    let tagged = |service: &str| object(json!({"service": {"type": "string", "enum": [service]}, "bucket": {"type": "string"}}), &["service", "bucket"]);
    let binding = json!({"oneOf": [{"allOf": [
        {"$ref": "#/components/schemas/Storage"},
        object(json!({"type": {"type": "string", "enum": ["storage"]}}), &["type"]),
    ]}]});
    let files = import(
        "all-of-ref-union",
        json!({"/b": get("getBinding", json!({"$ref": "#/components/schemas/Binding"}))}),
        json!({"Binding": binding, "Storage": {"oneOf": [tagged("s3"), tagged("gcs")]}}),
    )
    .expect("import");
    let types = &files.types;
    assert!(types.contains("type Binding = BindingS3 | BindingGcs\n"), "{types}");
    assert!(types.contains("class BindingS3 {\n    service: \"s3\",\n    bucket: string,\n    type: \"storage\",\n}"), "{types}");
    assert!(files.coverage.contains("| AllOf distributed over a union | 1 |"), "{}", files.coverage);
    assert!(files.coverage.contains("| Typed as `json` | 0 |"), "{}", files.coverage);
}

#[test]
fn classes_no_operation_reaches_are_dropped() {
    // A union with a free-form member is `json`: its other members are unused.
    let files = import(
        "unreachable",
        json!({"/a": get("getA", json!({"anyOf": [{"$ref": "#/components/schemas/Used"}, {}]}))}),
        json!({"Used": object(json!({"x": {"type": "string"}}), &["x"])}),
    )
    .expect("import");
    assert!(files.operations.contains("-> json throws"), "{}", files.operations);
    assert!(!files.types.contains("class Used"), "{}", files.types);
}

#[test]
fn merged_classes_keep_the_name_of_the_first_schema() {
    let item = || object(json!({"x": {"type": "string"}}), &["x"]);
    let shape = || object(json!({"item": item()}), &["item"]);
    let paths = json!({"/z": get("b", shape()), "/a": get("longer", shape())});
    let files = import("merge-paths", paths, json!({})).expect("import");
    // `/paths/~1a/…` sorts before `/paths/~1z/…`, although `BResponseItem` is shorter.
    assert!(files.types.contains("class LongerResponseItem {"), "{}", files.types);
    assert!(!files.types.contains("class BResponseItem {"), "{}", files.types);

    // A component's shape wins over any operation's, and a shallower one over a deeper one.
    let paths = json!({
        "/z": get("b", shape()),
        "/a": get("longer", shape()),
        "/c": get("c", json!({"$ref": "#/components/schemas/Zed"})),
        "/d": get("d", json!({"$ref": "#/components/schemas/Abc"})),
    });
    let schemas = json!({"Zed": shape(), "Abc": object(json!({"deeper": shape()}), &["deeper"])});
    let files = import("merge-components", paths, schemas).expect("import");
    assert!(files.types.contains("class ZedItem {"), "{}", files.types);
    assert!(!files.types.contains("class LongerResponseItem {"), "{}", files.types);
    assert!(!files.types.contains("class AbcDeeperItem {"), "{}", files.types);
}

#[test]
fn unimplemented_constructs_fail_or_are_counted() {
    let with_parameter = |parameter: Value| {
        json!({"/x/{id}": {"get": {
            "operationId": "getX",
            "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}, parameter],
            "responses": {"204": {"description": "done"}},
        }}})
    };
    let cases = [
        (
            "style",
            with_parameter(json!({"name": "f", "in": "query", "style": "deepObject", "schema": {"type": "string"}})),
            "sets a serialization other than style form, explode true",
        ),
        (
            "explode",
            with_parameter(json!({"name": "f", "in": "query", "explode": false, "schema": {"type": "array", "items": {"type": "string"}}})),
            "sets a serialization other than style form, explode true",
        ),
        (
            "reserved",
            with_parameter(json!({"name": "f", "in": "query", "allowReserved": true, "schema": {"type": "string"}})),
            "sets a serialization other than style form, explode true",
        ),
        (
            "content",
            with_parameter(json!({"name": "f", "in": "query", "content": {"application/json": {"schema": {}}}})),
            "parameter `f` has no schema",
        ),
        (
            "bool-query",
            with_parameter(json!({"name": "f", "in": "query", "schema": {"type": "boolean"}})),
            "Query parameter `f` has a type the importer cannot serialize: bool",
        ),
        (
            "int-list-query",
            with_parameter(json!({"name": "f", "in": "query", "schema": {"type": "array", "items": {"type": "integer"}}})),
            "Query parameter `f` has a type the importer cannot serialize: int[]",
        ),
        (
            "two-headers",
            json!({"/x": {"get": {
                "operationId": "getX",
                "parameters": [
                    {"name": "a", "in": "header", "schema": {"type": "string"}},
                    {"name": "b", "in": "header", "schema": {"type": "string"}},
                ],
                "responses": {"204": {"description": "done"}},
            }}}),
            "more than one header parameter is not implemented",
        ),
        (
            "path-parameter-not-in-path",
            with_parameter(json!({"name": "other", "in": "path", "required": true, "schema": {"type": "string"}})),
            "path parameter `other` is not in the path",
        ),
        (
            "path-variable-without-parameter",
            json!({"/x/{id}": {"get": {"operationId": "getX", "responses": {"204": {"description": "done"}}}}}),
            "`{id}` in the path is not a path parameter",
        ),
        (
            "unclosed-path-variable",
            json!({"/x/{id": {"get": {"operationId": "getX", "responses": {"204": {"description": "done"}}}}}),
            "malformed path template `/x/{id`",
        ),
        (
            "required-body-may-be-null",
            json!({"/x": {"post": {
                "operationId": "postX",
                "requestBody": {"required": true, "content": {"application/json": {"schema": {}}}},
                "responses": {"204": {"description": "done"}},
            }}}),
            "a required request body that may be null is not implemented",
        ),
        (
            "allof-additional-properties",
            json!({"/x": get("getX", json!({"allOf": [
                object(json!({"a": {"type": "string"}}), &[]),
                {"type": "object", "properties": {"b": {"type": "string"}}, "additionalProperties": {"type": "string"}},
            ]}))}),
            "merging an allOf part with additionalProperties is not implemented",
        ),
    ];
    for (name, paths, want) in cases {
        match import(name, paths, json!({})) {
            Ok(_) => panic!("{name}: imported, expected an error containing {want:?}"),
            Err(e) => assert!(e.contains(want), "{name}: {e}"),
        }
    }

    // An untyped schema with a keyword the importer does not implement is `json`, and reported.
    let files = import("not", json!({"/x": get("getX", json!({"not": {"type": "string"}}))}), json!({})).expect("import");
    assert!(files.operations.contains("-> json throws"), "{}", files.operations);
    assert!(files.coverage.contains("| `/paths/~1x/get/responses/200` | `not` is not implemented |"), "{}", files.coverage);
}

#[test]
fn operations_make_one_call() {
    let paths = json!({
        "/items/{id}/parts/{n}": {"post": {
            "operationId": "addPart",
            "parameters": [
                {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                {"name": "n", "in": "path", "required": true, "schema": {"type": "integer"}},
                {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                {"name": "tags", "in": "query", "schema": {"type": "array", "items": {"type": "string"}}},
            ],
            "requestBody": {"required": true, "content": {"application/json": {"schema": object(json!({"x": {"type": "string"}}), &["x"])}}},
            "responses": {"204": {"description": "done"}},
        }},
        "/health": get("health", object(json!({"ok": {"type": "boolean"}}), &["ok"])),
    });
    let operations = import("one-call", paths, json!({})).expect("import").operations;
    // The path interpolates each percent-encoded path parameter; the query is a
    // map of the parameters in spec order; the body is sent as JSON.
    let add_part = "    root.internal.platform_empty(
        client, \"POST\", `/items/${root.internal.path_segment(id)}/parts/${root.internal.path_segment(string.from(n))}`,
        query = { \"limit\": limit, \"tags\": tags },
        body = root.internal.json_body(body),
    )\n";
    assert!(operations.contains(add_part), "{operations}");
    assert!(operations.contains("    root.internal.platform_json<HealthResponse>(client, \"GET\", \"/health\")\n"), "{operations}");
}

#[test]
fn header_parameters_are_sent_when_set() {
    let files = import(
        "reserved-parameter",
        json!({"/x": {"get": {
            "operationId": "getX",
            "parameters": [{"name": "path", "in": "header", "schema": {"type": "string"}}],
            "responses": {"204": {"description": "done"}},
        }}}),
        json!({}),
    )
    .expect("import");
    assert!(files.operations.contains("path: string? = null"), "{}", files.operations);
    assert!(files.operations.contains("        headers = { \"path\": path },\n"), "{}", files.operations);
}

#[test]
fn json_fields_decode_without_a_conversion() {
    // A renamed field makes the class implement FromJson.
    let files = import(
        "json-field",
        json!({"/x": get("getX", object(json!({"test": {}, "n": {"type": "integer"}}), &["n"]))}),
        json!({}),
    )
    .expect("import");
    assert!(files.types.contains("test_: baml.json.to<json?>(baml.json.field(object, \"test\")),"), "{}", files.types);
    assert!(files.types.contains("n: baml.json.to<int>(baml.json.field(object, \"n\")),"), "{}", files.types);
}
