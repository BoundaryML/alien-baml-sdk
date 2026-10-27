// Setup-link helpers. The Platform API already returns ready-made launch links
// (`getProjectTemplateUrls`, `createManager`); these build a customized AWS
// CloudFormation quick-create link client-side from a server-provided template URL.

/// Input for `cloudformation_launch_url`.
class CloudFormationLaunch {
    /// `https://` URL of the CloudFormation template (for example the
    /// `templateUrl` from `getProjectTemplateUrls`). Used verbatim, percent-encoded.
    templateUrl: string,
    /// Commercial AWS region, for example `us-east-1`: `^[a-z]{2}-[a-z]+-[0-9]+$`, not `cn-*`.
    region: string,
    /// Stack name: a letter, then up to 127 letters, digits or hyphens.
    stackName: string,
    /// Stack parameters by key. Keys are ASCII alphanumeric.
    parameters: map<string, string>,
}

/// Builds an AWS CloudFormation quick-create console link:
///
/// `https://{region}.console.aws.amazon.com/cloudformation/home?region={region}#/stacks/create/review?templateURL=…&stackName=…&param_<Key>=…`
///
/// Every value is percent-encoded (RFC 3986 unreserved characters kept) and
/// parameters are sorted by key, so the same input always gives the same link.
/// Throws a non-retryable `AlienError` (`SETUP_*`) for invalid input.
function cloudformation_launch_url(input: CloudFormationLaunch) -> string throws root.errors.AlienError {
    if (!input.templateUrl.to_lower_case().starts_with("https://") || input.templateUrl.length() <= 8) {
        throw root.internal.setup_error(
            "SETUP_INVALID_TEMPLATE_URL",
            "templateUrl must be an https:// URL",
            { "field": "templateUrl" },
        );
    }
    if (!root.internal.is_commercial_aws_region(input.region)) {
        throw root.internal.setup_error(
            "SETUP_REGION_UNSUPPORTED",
            `region '${input.region}' is not a commercial AWS region`,
            { "field": "region", "region": input.region },
        );
    }
    if (!root.internal.is_valid_stack_name(input.stackName)) {
        throw root.internal.setup_error(
            "SETUP_INVALID_STACK_NAME",
            "stackName must start with a letter and contain only letters, digits and hyphens (at most 128 characters)",
            { "field": "stackName" },
        );
    }
    let keys = input.parameters.keys();
    keys.sort();
    for (let key in keys) {
        if (key.length() == 0 || !key.is_ascii_alphanumeric()) {
            throw root.internal.setup_error(
                "SETUP_INVALID_PARAMETER",
                `parameter key '${key}' must be ASCII alphanumeric`,
                { "field": "parameters", "key": key },
            );
        }
    }

    let query = [
        `templateURL=${root.internal.percent_encode(input.templateUrl)}`,
        `stackName=${root.internal.percent_encode(input.stackName)}`,
    ];
    for (let key in keys) {
        query.push(`param_${key}=${root.internal.percent_encode(input.parameters[key])}`);
    }
    let region = input.region;
    `https://${region}.console.aws.amazon.com/cloudformation/home?region=${region}#/stacks/create/review?${query.join("&")}`
}

/// Fetches the project's AWS template URL (`platform.projects.getTemplateUrls`) and
/// builds a quick-create link for it with `cloudformation_launch_url`.
/// Throws `SETUP_TEMPLATE_UNAVAILABLE` when the project has no AWS template.
function launch_url_for_project(
    client: root.platform.PlatformClient,
    project: string,
    region: string,
    stackName: string,
    parameters: map<string, string>,
) -> string throws root.errors.AlienError {
    let urls = root.platform.projects.getTemplateUrls(client, project);
    match (urls.aws) {
        let aws: root.platform.GetProjectTemplateUrlsResponseAws => cloudformation_launch_url(CloudFormationLaunch {
            templateUrl: aws.templateUrl,
            region: region,
            stackName: stackName,
            parameters: parameters,
        }),
        null => throw root.internal.setup_error(
            "SETUP_TEMPLATE_UNAVAILABLE",
            `project '${project}' has no AWS template URLs`,
            { "project": project },
        ),
    }
}
