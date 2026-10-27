// Test helpers for ns_setup/setup_tests.baml.

function shared_fixture_input() -> root.setup.CloudFormationLaunch {
    root.setup.CloudFormationLaunch {
        templateUrl: "https://alien-setup-fixture.s3.us-east-1.amazonaws.com/stacks/example-setup.yaml?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=EXAMPLEFIXTURECREDENTIAL%2F20261014%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20261014T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Signature=0f1x2t3u4r5e6s7i8g9n0a1t2u3r4e5n6o7t8a9r0e1a2l3s4i5g6n7a8t9u0r1e",
        region: "eu-west-1",
        stackName: "Example-Setup-01",
        parameters: {
            "Zeta": "last",
            "Alpha": "a&b=c",
            "Mid": "1+1 = 2",
            "Pct": "100%",
            "Name": "Café Zürich 東京",
            "Empty": "",
        },
    }
}

function launch(templateUrl: string, region: string, stackName: string, parameters: map<string, string>) -> string throws root.errors.AlienError {
    root.setup.cloudformation_launch_url(root.setup.CloudFormationLaunch {
        templateUrl: templateUrl,
        region: region,
        stackName: stackName,
        parameters: parameters,
    })
}

/// The error code `launch` throws for this input, or "no error".
function launch_error(templateUrl: string, region: string, stackName: string, parameters: map<string, string>) -> string {
    { launch(templateUrl, region, stackName, parameters); "no error" } catch (e) {
        root.errors.AlienError => if (e.retryable) { `${e.code} retryable` } else { e.code },
    }
}
