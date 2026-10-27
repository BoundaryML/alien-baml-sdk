// Ported decode rules (compat.baml). One testset per Alien regression test.

function newer_remote_sandbox() -> string {
    `{"enabled":true,"futureField":"value","baseImage":"ghcr.io/example/img@sha256:abc","maxLifetimeSeconds":3600,`
        + `"futureCloud":{"image":"registry.example.com/img@sha256:abc","maxLifetimeSeconds":3600},`
        + `"azure":{"catalogImage":"ubuntu","futureImage":"ghcr.io/example/img@sha256:def","idleSuspendSeconds":600}}`
}

function released_remote_sandbox() -> string {
    `{"enabled":true,"baseImage":"public.ecr.aws/docker/library/buildpack-deps@sha256:abc","maxLifetimeSeconds":3600,`
        + `"azure":{"catalogImage":"ubuntu","idleSuspendSeconds":600}}`
}

function project_json(id: string, sandbox: string) -> string {
    `{"id":"${id}","name":"my-app","createdAt":"2026-09-29T00:00:00Z","workspaceId":"ws_AAAAAAAAAAAAAAAAAAAAAAAA",`
        + `"projectCapabilities":{"schemaVersion":1,"capabilities":{"remoteSandbox":${sandbox}}}}`
}

/// The error code a call throws, or "no error".
function error_code_of_presence(json: string) -> string {
    { stored_secret_presence(json); "no error" } catch (e) {
        root.errors.AlienError => e.code,
    }
}

function error_code_of_role(json: string) -> string {
    { role_scope(json); "no error" } catch (e) {
        root.errors.AlienError => e.code,
    }
}

testset "unknown_response_fields" {
    test "a project with fields this client does not know decodes" {
        let page = decode_project_page(
            `{"items":[${project_json("prj_bbbbbbbbbbbbbbbbbbbbbbbbbbbb", released_remote_sandbox())},`
                + `${project_json("prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaa", newer_remote_sandbox())}],"nextCursor":null}`,
        );
        assert.equal(page.items.length(), 2);
        let newer = page.items[1].projectCapabilities?.capabilities.remoteSandbox;
        assert.equal(newer?.baseImage, "ghcr.io/example/img@sha256:abc");
        assert.equal(newer?.azure?.catalogImage, "ubuntu");
        assert.equal(newer?.azure?.idleSuspendSeconds, 600);
    }

    test "an unknown enum value is kept as a string" {
        let m = decode_materialization(
            `{"packages":[{"type":"sandbox-bundle","status":"ready"},{"type":"future-package-type","status":"ready"}]}`,
        );
        assert.equal(m.packages[1].type, "future-package-type");
    }
}

testset "optional_success_body" {
    test "202 with JSON decodes" {
        let accepted = root.internal.decode_optional_body<KeySetupAccepted>(
            `{"operationId":"op-test","physicalResourceId":"key-test"}`,
            202,
            "/v1/aws-key-setups",
        );
        assert.equal(accepted?.operationId, "op-test");
        assert.equal(accepted?.physicalResourceId, "key-test");
    }

    test "204 with an empty body is null" {
        assert.equal(root.internal.decode_optional_body<KeySetupAccepted>("", 204, "/v1/aws-key-setups"), null);
    }
}

testset "command_registry_unavailable" {
    test "a 503 API error keeps its code, message and retryable flag" {
        let e = root.internal.platform_http_error(
            503,
            "/v1/commands/bootstrap",
            `{"code":"SERVICE_UNAVAILABLE","message":"The API cannot serve the request right now.","retryable":true,`
                + `"internal":false,"httpStatusCode":503,"requestId":"00000000-0000-4000-8000-000000000000"}`,
        );
        assert.equal(e.code, "SERVICE_UNAVAILABLE");
        assert.equal(e.message, "The API cannot serve the request right now.");
        assert.equal(e.retryable, true);
        assert.equal(e.http_status, 503);
        assert.equal(e.context?.get("requestId"), "00000000-0000-4000-8000-000000000000");
    }

    test "a non-API error body becomes PLATFORM_HTTP_ERROR, retryable on 408, 429 and 5xx" {
        assert.equal(root.internal.platform_http_error(503, "/p", "upstream down").code, "PLATFORM_HTTP_ERROR");
        assert.equal(root.internal.platform_http_error(503, "/p", "upstream down").retryable, true);
        assert.equal(root.internal.platform_http_error(500, "/p", "").retryable, true);
        assert.equal(root.internal.platform_http_error(429, "/p", "").retryable, true);
        assert.equal(root.internal.platform_http_error(408, "/p", "").retryable, true);
        assert.equal(root.internal.platform_http_error(400, "/p", "").retryable, false);
        assert.equal(root.internal.platform_http_error(404, "/p", "{}").retryable, false);
        assert.equal(root.internal.platform_http_error(404, "/p", "{}").http_status, 404);
    }

    test "an API error without retryable is not retryable" {
        let e = root.internal.platform_http_error(500, "/p", `{"code":"X","message":"m","internal":false}`);
        assert.equal(e.code, "X");
        assert.equal(e.retryable, false);
    }
}

testset "stored_secret_presence" {
    test "absent and empty remain distinct" {
        assert.equal(stored_secret_presence("{}"), "null");
        assert.equal(stored_secret_presence(`{"storedSecretInputIds":[]}`), "empty");
        assert.equal(stored_secret_presence(`{"storedSecretInputIds":["credential"]}`), "ids:credential");
    }

    test "absent and empty survive re-encoding" {
        assert.equal(reencode_config(`{"inputValues":{"enableFeature":false}}`), `{"inputValues":{"enableFeature":false}}`);
        assert.equal(
            reencode_config(`{"storedSecretInputIds":[],"inputValues":{"enableFeature":false}}`),
            `{"storedSecretInputIds":[],"inputValues":{"enableFeature":false}}`,
        );
    }

    test "null decodes like omission and re-encodes as omission" {
        assert.equal(stored_secret_presence(`{"storedSecretInputIds":null}`), "null");
        assert.equal(reencode_config(`{"storedSecretInputIds":null}`), "{}");
    }

    test "malformed arrays are rejected" {
        assert.equal(error_code_of_presence(`{"storedSecretInputIds":[42]}`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_presence(`{"storedSecretInputIds":["credential",false]}`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_presence(`{"storedSecretInputIds":[null]}`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_presence(`{"storedSecretInputIds":"credential"}`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_presence(`{"storedSecretInputIds":{"credential":true}}`), "MALFORMED_RESPONSE");
    }
}

testset "stack_by_platform" {
    test "platform stacks decode without being dropped" {
        assert.equal(
            roundtrip_stack_by_platform(`{"aws":{"id":"example","resources":[]}}`),
            `{"aws":{"id":"example","resources":[]}}`,
        );
    }

    test "the test platform decodes into test_" {
        assert.equal(stack_for_test_platform(`{"test":{"id":"t","resources":[{"k":1}]}}`), `{"id":"t","resources":[{"k":1}]}`);
        assert.equal(stack_for_test_platform(`{"aws":{}}`), "null");
        // `test_` is not a wire name.
        assert.equal(stack_for_test_platform(`{"test_":{"id":"t"}}`), "null");
    }

    test "test_ encodes as test" {
        assert.equal(encode_test_platform_stack(`{"id":"t"}`), `{"test":{"id":"t"}}`);
    }

    test "every platform round-trips, unset ones stay omitted" {
        let all = `{"aws":{"a":1},"gcp":{"g":2},"azure":[1,2],"kubernetes":"k","machines":3,"local":true,"test":{"t":null}}`;
        assert.equal(roundtrip_stack_by_platform(all), all);
        assert.equal(roundtrip_stack_by_platform(`{"test":{"id":"t"},"gcp":null}`), `{"test":{"id":"t"}}`);
        assert.equal(roundtrip_stack_by_platform("{}"), "{}");
    }

    test "a non-object is malformed" {
        assert.equal({ roundtrip_stack_by_platform("[1]"); "no error" } catch (e) { root.errors.AlienError => e.code }, "MALFORMED_RESPONSE");
        assert.equal({ roundtrip_stack_by_platform(`"aws"`); "no error" } catch (e) { root.errors.AlienError => e.code }, "MALFORMED_RESPONSE");
    }
}

testset "field_names" {
    test "snake_case wire names decode into camelCase fields and encode back" {
        let s = `{"network":{"private_subnet_ids":["subnet-a"],"public_subnet_ids":["subnet-b"],`
            + `"security_group_ids":["sg-1"],"type":"byo-vpc-aws","vpc_id":"vpc-1"}}`;
        assert.equal(aws_network_of(s), "vpc-1 subnet-a");
        assert.equal(roundtrip_stack_settings(s), s);
        let pools = `{"compute":{"pools":{"general":{"failure_domains":{"spread":2},"machines":3,"mode":"fixed"}}}}`;
        assert.equal(roundtrip_stack_settings(pools), pools);
        let gcp = `{"network":{"network_name":"net","region":"us-central1","subnet_name":"sub","type":"byo-vpc-gcp"}}`;
        assert.equal(roundtrip_stack_settings(gcp), gcp);
    }

    test "the Manager's stack settings keep their wire names" {
        let s = `{"network":{"availability_zones":2,"cidr":"10.0.0.0/16","type":"create"}}`;
        assert.equal(roundtrip_manager_stack_settings(s), s);
    }

    test "a response's snake_case and kebab-case names round-trip" {
        let flags = `{"custom_domains":true,"private_managers":false,"operations_custom_plugins":false,"sso_saml":true,"audit_logs":false,"airgapped":false}`;
        assert.equal(feature_flags_of(flags), "customDomains=true ssoSaml=true");
        assert.equal(roundtrip_feature_flags(flags), flags);
        let counts = `{"updated":1,"updating":2,"failed":3,"pending":4,"waiting-on-owner":5,"waiting-on-inputs":6,"pinned-other":7,"superseded":8,"on-other":9}`;
        assert.equal(roundtrip_rollout_counts(counts), counts);
    }

    test "the field name is not a wire name" {
        let camel = `{"customDomains":true,"privateManagers":false,"operationsCustomPlugins":false,"ssoSaml":true,"auditLogs":false,"airgapped":false}`;
        assert.equal({ roundtrip_feature_flags(camel); "no error" } catch (e) { root.errors.AlienError => e.code }, "MALFORMED_RESPONSE");
    }
}

testset "event_state" {
    test "every event state round-trips" {
        for (let state in [
            `"none"`,
            `"started"`,
            `"success"`,
            `{"failed":{}}`,
            `{"failed":{"error":{"code":"UPDATE_FAILED","message":"candidate did not become ready","internal":false,"retryable":true}}}`,
        ]) {
            let event = `{"id":"event_aaaaaaaaaaaaaaaaaaaaaaaaaaaa","deploymentId":"dep_aaaaaaaaaaaaaaaaaaaaaaaaaaaa",`
                + `"state":${state},"createdAt":"2026-10-10T06:26:48.506Z"}`;
            assert.equal(roundtrip_event(event), event);
        }
    }

    test "a failed state with a null error round-trips as failed" {
        assert.equal(
            roundtrip_event(`{"id":"e","state":{"failed":{"error":null}},"createdAt":"t"}`),
            `{"id":"e","state":{"failed":{}},"createdAt":"t"}`,
        );
    }
}

testset "roles" {
    test "role scopes round-trip through their typed variants" {
        assert.equal(role_scope(`"workspace.member"`), `workspace "workspace.member"`);
        assert.equal(role_scope(`"project.developer"`), `project "project.developer"`);
        assert.equal(role_scope(`"deployment.manager"`), `deployment "deployment.manager"`);
        assert.equal(role_scope(`"deployment-group.deployer"`), `deployment-group "deployment-group.deployer"`);
        assert.equal(role_scope(`"manager.runtime"`), `manager "manager.runtime"`);
    }

    test "unknown roles are rejected" {
        assert.equal(error_code_of_role(`"workspace.unknown"`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_role(`"unknown.runtime"`), "MALFORMED_RESPONSE");
        assert.equal(error_code_of_role("{}"), "MALFORMED_RESPONSE");
    }
}

testset "expires_in" {
    test "expiresIn accepts integral, fractional and null numbers" {
        assert.equal(decode_remote_access(`{"expiresIn":300}`).expiresIn, 300.0);
        assert.equal(decode_remote_access(`{"expiresIn":299.5}`).expiresIn, 299.5);
        assert.equal(decode_remote_access(`{"expiresIn":null}`).expiresIn, null);
        assert.equal(decode_remote_access("{}").expiresIn, null);
    }
}
