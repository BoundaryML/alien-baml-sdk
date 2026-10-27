// Tests for the setup-link helper. Helpers live in
// ns_internal/setup_testing.baml so they stay out of the public namespace.

testset "launch_url" {
    // conformance/fixtures/setup_launch_url.json: every language must produce this exact string.
    test "shared fixture" {
        assert.equal(
            cloudformation_launch_url(root.internal.shared_fixture_input()),
            "https://eu-west-1.console.aws.amazon.com/cloudformation/home?region=eu-west-1#/stacks/create/review?templateURL=https%3A%2F%2Falien-setup-fixture.s3.us-east-1.amazonaws.com%2Fstacks%2Fexample-setup.yaml%3FX-Amz-Algorithm%3DAWS4-HMAC-SHA256%26X-Amz-Credential%3DEXAMPLEFIXTURECREDENTIAL%252F20261014%252Fus-east-1%252Fs3%252Faws4_request%26X-Amz-Date%3D20261014T000000Z%26X-Amz-Expires%3D3600%26X-Amz-SignedHeaders%3Dhost%26X-Amz-Signature%3D0f1x2t3u4r5e6s7i8g9n0a1t2u3r4e5n6o7t8a9r0e1a2l3s4i5g6n7a8t9u0r1e&stackName=Example-Setup-01&param_Alpha=a%26b%3Dc&param_Empty=&param_Mid=1%2B1%20%3D%202&param_Name=Caf%C3%A9%20Z%C3%BCrich%20%E6%9D%B1%E4%BA%AC&param_Pct=100%25&param_Zeta=last",
        );
    }

    test "minimal link" {
        assert.equal(
            root.internal.launch("https://example.com/t.yaml", "us-east-1", "S", {}),
            "https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1#/stacks/create/review?templateURL=https%3A%2F%2Fexample.com%2Ft.yaml&stackName=S",
        );
    }

    test "encodes reserved characters in values" {
        let url = root.internal.launch("https://example.com/t.yaml", "us-east-1", "S", {
            "Amp": "a&b",
            "Eq": "a=b",
            "Plus": "a+b",
            "Pct": "100%",
            "Space": "a b",
            "Reserved": "/?#[]@!*'()",
            "Unreserved": "-._~",
        });
        assert.equal(
            url,
            "https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1#/stacks/create/review?templateURL=https%3A%2F%2Fexample.com%2Ft.yaml&stackName=S"
                + "&param_Amp=a%26b&param_Eq=a%3Db&param_Pct=100%25&param_Plus=a%2Bb"
                + "&param_Reserved=%2F%3F%23%5B%5D%40%21%2A%27%28%29&param_Space=a%20b&param_Unreserved=-._~",
        );
    }

    test "encodes non-ASCII as UTF-8 bytes" {
        assert.equal(
            root.internal.launch("https://example.com/t.yaml", "us-east-1", "S", { "Name": "café 東京 😀" }),
            "https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1#/stacks/create/review?templateURL=https%3A%2F%2Fexample.com%2Ft.yaml&stackName=S&param_Name=caf%C3%A9%20%E6%9D%B1%E4%BA%AC%20%F0%9F%98%80",
        );
    }

    test "presigned S3 template URL survives intact" {
        // The X-Amz-Signature and the already-encoded %2F in X-Amz-Credential
        // must be encoded once more, not decoded or dropped.
        let url = root.internal.launch(
            "https://bucket.s3.us-east-1.amazonaws.com/t.yaml?X-Amz-Credential=EXAMPLE%2F20261014&X-Amz-Signature=abc123def456",
            "us-east-1",
            "S",
            {},
        );
        assert.equal(
            url,
            "https://us-east-1.console.aws.amazon.com/cloudformation/home?region=us-east-1#/stacks/create/review?templateURL=https%3A%2F%2Fbucket.s3.us-east-1.amazonaws.com%2Ft.yaml%3FX-Amz-Credential%3DEXAMPLE%252F20261014%26X-Amz-Signature%3Dabc123def456&stackName=S",
        );
    }

    test "parameters are sorted by key" {
        let a = root.internal.launch("https://example.com/t.yaml", "us-east-1", "S", { "B": "2", "A": "1", "C": "3" });
        let b = root.internal.launch("https://example.com/t.yaml", "us-east-1", "S", { "C": "3", "A": "1", "B": "2" });
        assert.equal(a, b);
        assert.equal(a.ends_with("&param_A=1&param_B=2&param_C=3"), true);
    }

    test "output is deterministic" {
        assert.equal(cloudformation_launch_url(root.internal.shared_fixture_input()), cloudformation_launch_url(root.internal.shared_fixture_input()));
    }

    test "rejects non-https template URLs" {
        assert.equal(root.internal.launch_error("http://example.com/t.yaml", "us-east-1", "S", {}), "SETUP_INVALID_TEMPLATE_URL");
        assert.equal(root.internal.launch_error("s3://bucket/t.yaml", "us-east-1", "S", {}), "SETUP_INVALID_TEMPLATE_URL");
        assert.equal(root.internal.launch_error("https://", "us-east-1", "S", {}), "SETUP_INVALID_TEMPLATE_URL");
        assert.equal(root.internal.launch_error("", "us-east-1", "S", {}), "SETUP_INVALID_TEMPLATE_URL");
        assert.equal(root.internal.launch_error("HTTPS://example.com/t.yaml", "us-east-1", "S", {}), "no error");
    }

    test "rejects regions outside commercial AWS" {
        // China: commercial shape, different console host, rejected by name.
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "cn-north-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "cn-northwest-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        // GovCloud, ISO, European Sovereign Cloud: outside the shape.
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-gov-west-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-iso-east-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "eusc-de-east-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        // Not a region at all.
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "US-EAST-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1.evil.example", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "use-east-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-١", "S", {}), "SETUP_REGION_UNSUPPORTED");
    }

    test "accepts commercial regions, including ones AWS has not launched yet" {
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", {}), "no error");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "ap-southeast-7", "S", {}), "no error");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "mx-central-1", "S", {}), "no error");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "sa-west-12", "S", {}), "no error");
    }

    test "rejects invalid stack names" {
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "1stack", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "-stack", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "my_stack", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "my stack", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "stäck", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", `a${"b".repeat(128)}`, {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", `a${"b".repeat(127)}`, {}), "no error");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "My-Stack-2", {}), "no error");
    }

    test "rejects non-alphanumeric parameter keys" {
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "": "x" }), "SETUP_INVALID_PARAMETER");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "a-b": "x" }), "SETUP_INVALID_PARAMETER");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "a b": "x" }), "SETUP_INVALID_PARAMETER");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "a&b": "x" }), "SETUP_INVALID_PARAMETER");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "Kéy": "x" }), "SETUP_INVALID_PARAMETER");
        assert.equal(root.internal.launch_error("https://example.com/t.yaml", "us-east-1", "S", { "Key1": "x" }), "no error");
    }

    test "validation errors are never retryable" {
        // launch_error appends " retryable" when the error is retryable.
        assert.equal(root.internal.launch_error("http://x", "us-east-1", "S", {}), "SETUP_INVALID_TEMPLATE_URL");
        assert.equal(root.internal.launch_error("https://x", "cn-north-1", "S", {}), "SETUP_REGION_UNSUPPORTED");
        assert.equal(root.internal.launch_error("https://x", "us-east-1", "1", {}), "SETUP_INVALID_STACK_NAME");
        assert.equal(root.internal.launch_error("https://x", "us-east-1", "S", { "-": "" }), "SETUP_INVALID_PARAMETER");
    }
}
