// Decode rules ported from Alien's Platform Rust client regression tests
// (client-sdks/platform/rust/tests/*.rs in alienplatform/alien). Each rule is
// ported to a small BAML shape (stack_by_platform: the imported class), not to
// the operation the Rust test used.
//
// Test support only: the language facades do not re-export this namespace.
// The conformance scenarios call these functions through the generated SDKs
// so every rule also runs in all four host languages.

// ---------------------------------------------------------------------------
// unknown_response_fields.rs: unknown nested fields are ignored; an unknown
// enum value is accepted as a string (open enums).

class AzureSandbox {
    catalogImage: string?,
    idleSuspendSeconds: int?,
}

class RemoteSandbox {
    enabled: bool,
    baseImage: string?,
    maxLifetimeSeconds: int?,
    azure: AzureSandbox?,
}

class CapabilitySet {
    remoteSandbox: RemoteSandbox?,
}

class ProjectCapabilities {
    schemaVersion: int,
    capabilities: CapabilitySet,
}

class Project {
    id: string,
    name: string,
    createdAt: string,
    workspaceId: string,
    projectCapabilities: ProjectCapabilities?,
}

class ListProjectsResponse {
    items: Project[],
    nextCursor: string?,
}

class CapabilityPackage {
    /// Open enum: a type this client does not know is kept as-is.
    type: string,
    status: string,
}

class CapabilityMaterialization {
    packages: CapabilityPackage[],
}

function decode_project_page(json: string) -> ListProjectsResponse throws root.errors.AlienError {
    root.internal.decode_body<ListProjectsResponse>(json, 200, "compat")
}

function decode_materialization(json: string) -> CapabilityMaterialization throws root.errors.AlienError {
    root.internal.decode_body<CapabilityMaterialization>(json, 200, "compat")
}

// ---------------------------------------------------------------------------
// optional_success_body.rs: a success may be 202 with JSON or 204 with an
// empty body. Exercised over HTTP against the stub (POST /v1/aws-key-setups).

class KeySetupAccepted {
    operationId: string,
    physicalResourceId: string?,
}

class KeySetupAction {
    action: string,
}

/// `null` for a 204 (or any 2xx with an empty body); the decoded body otherwise.
function accept_key_setup(
    client: root.platform.PlatformClient,
    request: KeySetupAction,
) -> KeySetupAccepted? throws root.errors.AlienError {
    root.internal.platform_optional_json<KeySetupAccepted>(client, "POST", "/v1/aws-key-setups", body = root.internal.json_body(request))
}

// ---------------------------------------------------------------------------
// stored_secret_presence.rs: absent, empty and malformed `storedSecretInputIds`.

class DeploymentConfig {
    storedSecretInputIds: string[]?,
    inputValues: map<string, json>?,

    implements baml.ToJson {
        function to_json(self) -> baml.json.json throws baml.json.SerializationError {
            let out: map<string, baml.json.json> = {};
            if (self.storedSecretInputIds != null) { out["storedSecretInputIds"] = baml.json.from(self.storedSecretInputIds); }
            if (self.inputValues != null) { out["inputValues"] = baml.json.from(self.inputValues); }
            out
        }
    }
}

/// How a typed decode sees `storedSecretInputIds`: "null" (absent or null),
/// "empty" or "ids:<comma-joined>".
function stored_secret_presence(json: string) -> string throws root.errors.AlienError {
    let config = root.internal.decode_body<DeploymentConfig>(json, 200, "compat");
    match (config.storedSecretInputIds) {
        null => "null",
        let ids: string[] => if (ids.length() == 0) { "empty" } else { `ids:${ids.join(",")}` },
    }
}

/// Decodes a config and encodes it again the way request bodies are encoded
/// (unset optional fields omitted).
function reencode_config(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<DeploymentConfig>(json, 200, "compat"))
}

// ---------------------------------------------------------------------------
// stack_by_platform.rs: per-platform stacks are arbitrary JSON and must
// survive decoding untouched. Checked on the imported `platform.StackByPlatform`,
// whose wire field `test` is a BAML reserved word: the importer names it
// `test_` and maps it to and from `"test"` (docs/gaps.md#b14).

function roundtrip_stack_by_platform(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<root.platform.StackByPlatform>(json, 200, "compat"))
}

/// The `test` platform's stack as decoded into `test_`, as JSON text ("null" when unset).
function stack_for_test_platform(json: string) -> string throws root.errors.AlienError {
    let stacks = root.internal.decode_body<root.platform.StackByPlatform>(json, 200, "compat");
    baml.json.stringify(stacks.test_ ?? null) catch (e) {
        _ => throw root.internal.malformed_response(200, "compat", "", e.message),
    }
}

/// Encodes a `platform.StackByPlatform` built in code, with only the `test` platform set.
function encode_test_platform_stack(stack: string) -> string throws root.errors.AlienError {
    let value = baml.json.parse(stack) catch (e) {
        _ => throw root.internal.malformed_response(200, "compat", "", e.message),
    };
    root.internal.encode_body(root.platform.StackByPlatform { test_: value })
}

// ---------------------------------------------------------------------------
// Field names: a wire name with separators gets the camelCase name Alien's
// TypeScript SDK gives it (`vpc_id` → `vpcId`), mapped to and from the wire
// name like `test_` above, so the JSON on the wire does not change
// (docs/api-shape.md, "Field names").

/// Decodes Platform stack settings and encodes them again as a request body.
function roundtrip_stack_settings(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<root.platform.StackSettings>(json, 200, "compat"))
}

/// Decodes Manager stack settings and encodes them again as a request body.
function roundtrip_manager_stack_settings(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<root.manager.StackSettings>(json, 200, "compat"))
}

/// The AWS network's VPC as decoded into `vpcId`, and its first private subnet.
function aws_network_of(json: string) -> string throws root.errors.AlienError {
    match (root.internal.decode_body<root.platform.StackSettings>(json, 200, "compat").network) {
        let aws: root.platform.StackSettingsNetworkByoVpcAws => `${aws.vpcId} ${aws.privateSubnetIds[0]}`,
        _ => "not an AWS network",
    }
}

/// Decodes a feature-flag response and encodes it again.
function roundtrip_feature_flags(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<root.platform.BillingFeatureFlags>(json, 200, "compat"))
}

/// `customDomains` and `ssoSaml` as decoded.
function feature_flags_of(json: string) -> string throws root.errors.AlienError {
    let flags = root.internal.decode_body<root.platform.BillingFeatureFlags>(json, 200, "compat");
    `customDomains=${flags.customDomains} ssoSaml=${flags.ssoSaml}`
}

/// Decodes rollout counts (kebab-case wire names) and encodes them again.
function roundtrip_rollout_counts(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<root.platform.ReleaseRolloutStateCounts>(json, 200, "compat"))
}

// ---------------------------------------------------------------------------
// event_state.rs: every EventState variant survives a decode/encode round trip.
// Wire form: "none" | "started" | "success" | {"failed": {"error": AlienError | null}}

class EventError {
    code: string,
    message: string,
    internal: bool,
    retryable: bool?,
    hint: string?,
    httpStatusCode: int?,
    context: json?,
    source: json?,

    implements baml.ToJson {
        function to_json(self) -> baml.json.json throws baml.json.SerializationError {
            let out: map<string, baml.json.json> = {};
            out["code"] = baml.json.from(self.code);
            out["message"] = baml.json.from(self.message);
            out["internal"] = baml.json.from(self.internal);
            if (self.retryable != null) { out["retryable"] = baml.json.from(self.retryable); }
            if (self.hint != null) { out["hint"] = baml.json.from(self.hint); }
            if (self.httpStatusCode != null) { out["httpStatusCode"] = baml.json.from(self.httpStatusCode); }
            if (self.context != null) { out["context"] = baml.json.from(self.context); }
            if (self.source != null) { out["source"] = baml.json.from(self.source); }
            out
        }
    }
}

class EventFailure {
    error: EventError?,

    implements baml.ToJson {
        function to_json(self) -> baml.json.json throws baml.json.SerializationError {
            let out: map<string, baml.json.json> = {};
            if (self.error != null) { out["error"] = baml.json.from(self.error); }
            out
        }
    }
}

class EventFailed {
    failed: EventFailure,
}

type EventState = "none" | "started" | "success" | EventFailed

class Event {
    id: string,
    deploymentId: string?,
    state: EventState,
    createdAt: string,

    implements baml.ToJson {
        function to_json(self) -> baml.json.json throws baml.json.SerializationError {
            let out: map<string, baml.json.json> = {};
            out["id"] = baml.json.from(self.id);
            if (self.deploymentId != null) { out["deploymentId"] = baml.json.from(self.deploymentId); }
            out["state"] = baml.json.from(self.state);
            out["createdAt"] = baml.json.from(self.createdAt);
            out
        }
    }
}

function roundtrip_event(json: string) -> string throws root.errors.AlienError {
    root.internal.encode_body(root.internal.decode_body<Event>(json, 200, "compat"))
}

// ---------------------------------------------------------------------------
// roles.rs: a role decodes to its scope and encodes back to the same string;
// unknown roles are rejected (roles are a closed set).

type WorkspaceRole = "workspace.viewer" | "workspace.member" | "workspace.admin"
type ProjectRole = "project.viewer" | "project.developer" | "project.admin" | "project.encryption" | "project.ai-gateway" | "project.sandbox-gateway" | "project.remote-bindings"
type DeploymentRole = "deployment.viewer" | "deployment.manager" | "deployment.dynamic-container-manager" | "deployment.telemetry-writer"
type DeploymentGroupRole = "deployment-group.deployer"
type ManagerRole = "manager.runtime"
type Role = WorkspaceRole | ProjectRole | DeploymentRole | DeploymentGroupRole | ManagerRole

/// "<scope> <re-encoded JSON>" for a JSON role value.
function role_scope(json: string) -> string throws root.errors.AlienError {
    let role = root.internal.decode_body<Role>(json, 200, "compat");
    let scope = match (role) {
        let r: WorkspaceRole => "workspace",
        let r: ProjectRole => "project",
        let r: DeploymentRole => "deployment",
        let r: DeploymentGroupRole => "deployment-group",
        let r: ManagerRole => "manager",
    };
    `${scope} ${root.internal.encode_body(role)}`
}

// ---------------------------------------------------------------------------
// Probe finding: remote access responses carry `expiresIn` as a JSON number
// that may be fractional or null, so it is `float?`, not `int`.

class RemoteAccess {
    expiresIn: float?,
}

function decode_remote_access(json: string) -> RemoteAccess throws root.errors.AlienError {
    root.internal.decode_body<RemoteAccess>(json, 200, "compat")
}
