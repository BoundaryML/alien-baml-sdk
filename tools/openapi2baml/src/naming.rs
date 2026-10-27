//! Identifiers: reserved words, sanitizing wire names, case conversion, and the
//! group/method names Alien's TypeScript SDKs use.

/// Words BAML 0.23.0 rejects as a class field name. Established by compiling
/// `class K { <word>: string }` for each candidate (lexer keywords plus
/// contextual keywords and builtin type names).
pub const RESERVED_FIELD_NAMES: &[&str] = &[
    "await", "break", "catch", "catch_all", "class", "continue", "defer", "else", "enum", "for",
    "function", "generator", "if", "implement", "implements", "in", "instanceof", "is", "let",
    "match", "retry_policy", "return", "spawn", "template_string", "test", "testset", "throw",
    "throws", "while",
];

/// Parameter names that get a trailing `_`: the 34 words BAML 0.23.0 rejects as
/// a function parameter name (found the same way, with `function f(<word>:
/// string)`), plus `baml`, `client`, `false`, `null` and `true`, which BAML
/// 0.22.2 also rejected.
pub const RESERVED_PARAM_NAMES: &[&str] = &[
    "await", "baml", "break", "catch", "catch_all", "class", "client", "continue", "defer", "else",
    "enum", "extends", "false", "for", "function", "generator", "if", "implement", "implements",
    "in", "instanceof", "interface", "is", "let", "match", "null", "requires", "retry_policy",
    "return", "root", "self", "spawn", "template_string", "test", "testset", "throw", "throws", "true",
    "while",
];

/// A BAML identifier for a wire name: invalid characters become `_`, a leading
/// digit gets a `_` prefix, and a reserved word gets a trailing `_`.
pub fn identifier(wire: &str, reserved: &[&str]) -> String {
    let mut out: String = wire
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if reserved.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// The field name of a property's wire name, as Alien's TypeScript SDK names
/// it: a wire name with separators is camelCased (`custom_domains` →
/// `customDomains`, `on-other` → `onOther`, `_internal` → `internal`), any
/// other is kept (`createdAt`); then made an identifier (`test` → `test_`).
pub fn field_name(wire: &str) -> String {
    identifier(&sdk_name(wire), RESERVED_FIELD_NAMES)
}

/// The argument name of a parameter's wire name, by the same rule as
/// [`field_name`] (`command_id` → `commandId`, `If-Match` → `ifMatch`); then
/// made an identifier (`in` → `in_`).
pub fn argument_name(wire: &str) -> String {
    identifier(&sdk_name(wire), RESERVED_PARAM_NAMES)
}

/// Alien's TypeScript SDK camelCases a wire name with separators and keeps any other.
fn sdk_name(wire: &str) -> String {
    if wire.chars().all(|c| c.is_ascii_alphanumeric()) {
        wire.to_string()
    } else {
        camel(wire)
    }
}

/// `get_command_status` → `GetCommandStatus`, `createSetupLink` → `CreateSetupLink`.
pub fn pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        } else {
            upper = true;
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// `get_command_status` → `getCommandStatus`; already-camel names are kept.
pub fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut chars = p.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert_eq!(identifier("test", RESERVED_FIELD_NAMES), "test_");
        assert_eq!(identifier("type", RESERVED_FIELD_NAMES), "type");
        assert_eq!(identifier("client", RESERVED_FIELD_NAMES), "client");
        assert_eq!(identifier("client", RESERVED_PARAM_NAMES), "client_");
        assert_eq!(identifier("x-amz-date", RESERVED_FIELD_NAMES), "x_amz_date");
        assert_eq!(identifier("1st", RESERVED_FIELD_NAMES), "_1st");
        assert_eq!(identifier("idOrName", RESERVED_PARAM_NAMES), "idOrName");
        assert_eq!(identifier("in", RESERVED_PARAM_NAMES), "in_");
        assert_eq!(identifier("root", RESERVED_PARAM_NAMES), "root_");
        assert_eq!(identifier("path", RESERVED_FIELD_NAMES), "path");
    }

    #[test]
    fn field_names() {
        assert_eq!(field_name("custom_domains"), "customDomains");
        assert_eq!(field_name("private_subnet_ids"), "privateSubnetIds");
        assert_eq!(field_name("on-other"), "onOther");
        assert_eq!(field_name("_internal"), "internal");
        assert_eq!(field_name("createdAt"), "createdAt");
        assert_eq!(field_name("Aws"), "Aws");
        assert_eq!(field_name("test"), "test_");
        assert_eq!(field_name("1st"), "_1st");
    }

    #[test]
    fn argument_names() {
        assert_eq!(argument_name("command_id"), "commandId");
        assert_eq!(argument_name("If-Match"), "ifMatch");
        assert_eq!(argument_name("if-match"), "ifMatch");
        assert_eq!(argument_name("idOrName"), "idOrName");
        assert_eq!(argument_name("path"), "path");
        assert_eq!(argument_name("client"), "client_");
    }

    #[test]
    fn cases() {
        assert_eq!(pascal("get_command_status"), "GetCommandStatus");
        assert_eq!(pascal("createSetupLink"), "CreateSetupLink");
        assert_eq!(pascal("APIError"), "APIError");
        assert_eq!(camel("get_command_status"), "getCommandStatus");
        assert_eq!(camel("whoami"), "whoami");
        assert_eq!(camel("createAPIKey"), "createAPIKey");
    }
}
