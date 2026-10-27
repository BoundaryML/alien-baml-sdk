/// A non-retryable setup validation error.
function setup_error(code: string, message: string, context: map<string, string>) -> root.errors.AlienError {
    root.errors.AlienError {
        code: code,
        message: message,
        retryable: false,
        http_status: null,
        context: context,
    }
}

/// A commercial-partition AWS region, whose console lives at
/// `{region}.console.aws.amazon.com`: `^[a-z]{2}-[a-z]+-[0-9]+$`, and not a China
/// region (`cn-*`). The shape alone already rules out GovCloud (`us-gov-west-1`),
/// ISO and European Sovereign Cloud regions; China regions share it, so they are
/// rejected by name. Regions AWS adds later pass without a code change. Digits are
/// `[0-9]` rather than `\d`, which BAML's regex engine matches against any
/// Unicode digit.
function is_commercial_aws_region(region: string) -> bool {
    let shape = baml.regex.new("^[a-z]{2}-[a-z]+-[0-9]+$") catch (e) {
        baml.regex.Error => baml.sys.panic(`constant region pattern failed to compile: ${e.message}`),
    };
    shape.has_match(region) && !region.starts_with("cn-")
}

/// `^[A-Za-z][A-Za-z0-9-]{0,127}$`
function is_valid_stack_name(name: string) -> bool {
    let chars = name.chars();
    if (chars.length() == 0 || chars.length() > 128 || !chars[0].is_ascii_alphabetic()) {
        return false;
    }
    for (let c in chars) {
        if (!c.is_ascii_alphanumeric() && c != "-") {
            return false;
        }
    }
    true
}
