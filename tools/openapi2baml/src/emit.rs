//! BAML text for the lowered types and operations.
//!
//! Multi-line pieces of BAML are strings whose lines after the first are
//! indented relative to the first; [`lines`] and [`block`] nest them.

use std::{collections::BTreeMap, fmt::Write as _};

use crate::{
    Error,
    config::ClientKind,
    lower::{Class, Def, EXTRA_FIELD, Field, OTHER_RAW_FIELD, TagTree, Ty, UNKNOWN_TAG, optional, tag_tree},
    ops::{Location, Operation, Response},
};

const INDENT: &str = "    ";

/// Lines that fit within this many columns are kept on one line.
const WIDTH: usize = 100;

/// `text` indented by one level, every line.
fn nest(text: &str) -> String {
    format!("{INDENT}{}", text.replace('\n', &format!("\n{INDENT}")))
}

/// `open`, the items one per line indented by one level, then `close`.
fn lines(open: &str, items: &[String], close: &str) -> String {
    let mut out = format!("{open}\n");
    for item in items {
        out.push_str(&nest(item));
        out.push('\n');
    }
    out.push_str(close);
    out
}

/// `head { … }`, one item per line.
fn block(head: &str, items: &[String]) -> String {
    lines(&format!("{head} {{"), items, "}")
}

/// `match (subject) { … }`, one arm per item.
fn match_expr(subject: &str, arms: &[String]) -> String {
    block(&format!("match ({subject})"), arms)
}

/// The `_` arm that throws a `baml.json.DecodeError` saying `message`.
fn decode_error_arm(message: &str) -> String {
    format!("_ => throw baml.json.DecodeError {{ message: {}, path: \"\" }},", quote(message))
}

/// `"text"` as a BAML string literal.
fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("string")
}

/// `text` as part of a backtick string literal: `\`, `` ` `` and `${` escaped.
fn template_text(text: &str) -> String {
    text.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
}

/// A service's types by name, for decoding open unions (`open_unions`). BAML
/// picks a union's member by shape, and an open union's `Other` member, whose
/// tag is the literal [`UNKNOWN_TAG`], fits no value the server sends. So a
/// class with a field holding an open union, and an operation returning one,
/// decode it themselves: by its tags, known tags as their own class and any
/// others as the `Other` member (DESIGN.md, "Open unions").
pub struct Index<'a> {
    defs: BTreeMap<&'a str, &'a Def>,
}

impl<'a> Index<'a> {
    pub fn new(defs: &[&'a Def]) -> Self {
        Self { defs: defs.iter().map(|d| (d.name(), *d)).collect() }
    }

    fn class(&self, name: &str) -> Option<&'a Class> {
        match self.defs.get(name) {
            Some(Def::Class(class)) => Some(class),
            _ => None,
        }
    }

    /// The `Other` member of a union with these members, if it is open.
    fn other(&self, members: &[Ty]) -> Option<&'a Class> {
        members.iter().find_map(|m| match m {
            Ty::Named(name) => self.class(name).filter(|c| c.open_tag.is_some()),
            _ => None,
        })
    }

    /// Whether decoding `ty` meets an open union before it meets a class (a
    /// class decodes its own fields).
    fn has_open(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Named(name) => match self.defs.get(name.as_str()) {
                Some(Def::Alias { ty, .. }) => self.has_open(ty),
                _ => false,
            },
            Ty::Optional(inner) | Ty::List(inner) | Ty::Map(inner) => self.has_open(inner),
            Ty::Union(members) => self.other(members).is_some() || members.iter().any(|m| self.has_open(m)),
            _ => false,
        }
    }

    /// A BAML expression decoding the JSON expression `value` as `ty`, open
    /// unions by their tags: one `match` on `value`, with a `null` arm when
    /// `ty` is optional.
    fn decode(&self, ty: &Ty, value: &str, qualify: &dyn Fn(&str) -> String) -> String {
        if !self.has_open(ty) {
            return match ty {
                Ty::Json => value.to_string(),
                ty => format!("baml.json.to<{}>({value})", ty.render(qualify)),
            };
        }
        let mut ty = ty;
        let mut arms = Vec::new();
        loop {
            match ty {
                Ty::Named(name) => match self.defs.get(name.as_str()) {
                    Some(Def::Alias { ty: aliased, .. }) => ty = aliased,
                    _ => unreachable!("has_open is false for a class"),
                },
                Ty::Optional(inner) => {
                    if arms.is_empty() {
                        arms.push("null => null,".to_string());
                    }
                    ty = inner;
                }
                _ => break,
            }
        }
        match ty {
            Ty::Union(members) => {
                let member = self.select_member(members, "member", qualify);
                arms.push(format!("let member: map<string, baml.json.json> => {member},"));
                arms.push(decode_error_arm("expected a JSON object"));
            }
            Ty::List(item) => {
                let item = self.decode(item, "item", qualify);
                arms.push(lines("let items: baml.json.json[] => items.map((item) -> {", &[item], "}),"));
                arms.push(decode_error_arm("expected a JSON array"));
            }
            Ty::Map(item) => {
                let item = self.decode(item, "entries[key]", qualify);
                let entries = [
                    format!("let out: {} = {{}};", ty.render(qualify)),
                    block("for (let key in entries.keys())", &[format!("out[key] = {item};")]),
                    "out".to_string(),
                ];
                arms.push(lines("let entries: map<string, baml.json.json> => {", &entries, "},"));
                arms.push(decode_error_arm("expected a JSON object"));
            }
            _ => unreachable!("has_open is false for {}", ty.render(qualify)),
        }
        match_expr(value, &arms)
    }

    /// A BAML expression picking the member of the open union `members` that
    /// the JSON object `object` is, by its tag fields.
    fn select_member(&self, members: &[Ty], object: &str, qualify: &dyn Fn(&str) -> String) -> String {
        let other = self.other(members).unwrap_or_else(|| {
            unreachable!("open_tagged_unions opens a union only where it holds classes: {}", Ty::Union(members.to_vec()).render(qualify))
        });
        let (wires, _) = other.open_tag.as_ref().expect("an Other member");
        let classes = members
            .iter()
            .filter_map(|member| {
                let Ty::Named(name) = member else { unreachable!("an open union holds classes") };
                (name != &other.name).then(|| self.class(name).expect("an open union holds classes"))
            })
            .collect::<Vec<_>>();
        let tree = tag_tree(&classes, wires).expect("open_tagged_unions opens a union only where its tags tell the members apart");
        let other = format!("{} {{ {}: {}, {OTHER_RAW_FIELD}: {object} }}", qualify(&other.name), other.fields[0].name, quote(UNKNOWN_TAG));
        select(&tree, object, &other, qualify)
    }
}

/// A BAML expression picking a member of an open union for the JSON object
/// `object` by its tag fields (`tree`): a `match` on each tag read, with
/// `other` for tags no member has.
fn select(tree: &TagTree, object: &str, other: &str, qualify: &dyn Fn(&str) -> String) -> String {
    match tree {
        TagTree::Member(class) => format!("baml.json.to<{}>({object})", qualify(&class.name)),
        TagTree::Read { wire, arms } => {
            let mut lines: Vec<String> =
                arms.iter().map(|(value, next)| format!("{} => {},", quote(value), select(next, object, other, qualify))).collect();
            lines.push(format!("_ => {other},"));
            match_expr(&format!("baml.json.to<string>(baml.json.field({object}, {}))", quote(wire)), &lines)
        }
    }
}

pub fn banner(spec_path: &str, digest: &str, overlays: &[(String, String)]) -> String {
    let mut out = format!("// Generated by tools/openapi2baml from {spec_path} ({digest})");
    for (path, digest) in overlays {
        let _ = write!(out, "\n// with overlay {path} ({digest})");
    }
    out.push_str(".\n// Do not edit: change the importer or its config, then run scripts/import.sh.\n");
    out
}

fn doc(out: &mut String, indent: &str, text: &str) {
    for line in text.trim().lines() {
        let line = line.trim_end();
        if line.is_empty() {
            let _ = writeln!(out, "{indent}///");
        } else {
            let _ = writeln!(out, "{indent}/// {line}");
        }
    }
}

pub fn client(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::Platform => {
            "/// How to reach the Platform API.\n\
             class PlatformClient {\n    \
             /// Base URL, for example `https://api.alien.dev`. Trailing slashes are ignored; a\n    \
             /// query string stays at the end of each request URL.\n    \
             base_url: string,\n    \
             /// API key, sent as `Authorization: Bearer <api_key>`.\n    \
             api_key: string,\n    \
             /// Workspace name or ID, sent as the `workspace` query parameter on operations\n    \
             /// that accept it. Optional for workspace-scoped API keys.\n    \
             workspace: string?,\n    \
             /// Limit in milliseconds for each HTTP request, from connecting to reading the whole\n    \
             /// response. `null` is BAML's default of 5 minutes; zero or negative is no limit. A\n    \
             /// request that runs out fails with `COMMUNICATION_ERROR`. A per-call timeout also\n    \
             /// applies, to the whole call: the shorter one ends it.\n    \
             timeout_ms: int?,\n\
             }\n"
        }
        ClientKind::Manager => {
            "/// How to reach a Manager.\n\
             class ManagerClient {\n    \
             /// Base URL of the Manager, for example the `managerUrl` from `bootstrapCommands`.\n    \
             /// Trailing slashes are ignored; a query string stays at the end of each request URL.\n    \
             base_url: string,\n    \
             /// Bearer token, sent as `Authorization: Bearer <token>` on every operation when set,\n    \
             /// as `@alienplatform/commands` does.\n    \
             token: string?,\n    \
             /// Limit in milliseconds for each HTTP request, from connecting to reading the whole\n    \
             /// response. `null` is BAML's default of 5 minutes; zero or negative is no limit. A\n    \
             /// request that runs out fails with `COMMUNICATION_ERROR`. A per-call timeout also\n    \
             /// applies, to the whole call: the shorter one ends it.\n    \
             timeout_ms: int?,\n\
             }\n"
        }
    }
}

pub fn types(defs: &[&Def], index: &Index) -> String {
    let mut out = String::new();
    let local = |n: &str| n.to_string();
    for def in defs {
        out.push('\n');
        match def {
            Def::Alias { name, doc: text, ty } => {
                doc(&mut out, "", text);
                let _ = writeln!(out, "type {name} = {}", ty.render(&local));
            }
            Def::Class(class) => class_text(&mut out, class, index),
        }
    }
    out
}

fn class_text(out: &mut String, class: &Class, index: &Index) {
    let local = |n: &str| n.to_string();
    doc(out, "", &class.doc);
    let _ = writeln!(out, "class {} {{", class.name);
    for field in &class.fields {
        let mut text = field.doc.clone();
        if field.name != field.wire {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            let _ = write!(text, "Wire name: `{}`.", field.wire);
        }
        doc(out, INDENT, &text);
        let _ = writeln!(out, "    {}: {},", field.name, field.ty.render(&local));
    }
    if let Some(extra) = &class.extra {
        doc(out, INDENT, "Keys the schema does not list, with their values.");
        let _ = writeln!(out, "    {EXTRA_FIELD}: map<string, {}>,", extra.render(&local));
    }
    let mut implementations = Vec::new();
    if class.open_tag.is_some() {
        // An open union's Other member: decoded by the classes and operations
        // that hold its union (Index), encoded as the member it holds.
        implementations.push(to_json(&[format!("baml.json.from(self.{OTHER_RAW_FIELD})")]));
    } else {
        if class.renamed || class.extra.is_some() || class.fields.iter().any(|f| index.has_open(&f.ty)) {
            implementations.push(from_json(class, index));
        }
        // Default encoding writes every field under its BAML name, nulls
        // included, and honors nested ToJson overrides; override only where
        // that is wrong.
        if class.renamed || class.extra.is_some() || (class.encoded && class.fields.iter().any(omittable)) {
            implementations.push(to_json(&to_json_body(class)));
        }
    }
    for implementation in implementations {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", nest(&implementation));
    }
    let _ = writeln!(out, "}}");
}

/// `implements baml.FromJson`: every field from its wire name, open unions by
/// their tags, and the keys the schema does not list into [`EXTRA_FIELD`].
fn from_json(class: &Class, index: &Index) -> String {
    let local = |n: &str| n.to_string();
    let mut fields: Vec<String> = class
        .fields
        .iter()
        .map(|f| format!("{}: {},", f.name, index.decode(&f.ty, &format!("baml.json.field(object, {})", quote(&f.wire)), &local)))
        .collect();
    let object = "let object: map<string, baml.json.json> =>";
    let arm = match &class.extra {
        None => format!("{object} {},", block(&class.name, &fields)),
        Some(extra) => {
            fields.push(format!("{EXTRA_FIELD}: extra,"));
            let value = match extra {
                Ty::Json => "object[key]".to_string(),
                ty => format!("baml.json.to<{}>(object[key])", ty.render(&local)),
            };
            let statements = [
                format!("let extra: map<string, {}> = {{}};", extra.render(&local)),
                unlisted_keys("object.keys()", &class.fields, &format!("extra[key] = {value};")),
                block(&class.name, &fields),
            ];
            lines(&format!("{object} {{"), &statements, "},")
        }
    };
    let body = match_expr("j", &[arm, decode_error_arm(&format!("{} must be a JSON object", class.name))]);
    block(
        "implements baml.FromJson",
        &[block("function from_json(j: baml.json.json) -> Self throws baml.json.DecodeError", &[body])],
    )
}

/// The statements of `ToJson.to_json`: every field under its wire name, an
/// unset optional field left out, then the extra keys.
fn to_json_body(class: &Class) -> Vec<String> {
    let mut lines = vec!["let out: map<string, baml.json.json> = {};".to_string()];
    for field in &class.fields {
        let set = format!("out[{}] = baml.json.from(self.{});", quote(&field.wire), field.name);
        lines.push(if omittable(field) { format!("if (self.{} != null) {{ {set} }}", field.name) } else { set });
    }
    if class.extra.is_some() {
        let keys = format!("self.{EXTRA_FIELD}.keys()");
        lines.push(unlisted_keys(&keys, &class.fields, &format!("out[key] = baml.json.from(self.{EXTRA_FIELD}[key]);")));
    }
    lines.push("out".to_string());
    lines
}

/// `implements baml.ToJson` with these statements.
fn to_json(body: &[String]) -> String {
    block(
        "implements baml.ToJson",
        &[block("function to_json(self) -> baml.json.json throws baml.json.SerializationError", body)],
    )
}

/// A loop running `statement` for every key in `keys` that is not the wire
/// name of one of `fields`.
fn unlisted_keys(keys: &str, fields: &[Field], statement: &str) -> String {
    let body = if fields.is_empty() {
        statement.to_string()
    } else {
        let listed = fields.iter().map(|f| quote(&f.wire)).collect::<Vec<_>>().join(" | ");
        match_expr("key", &[format!("{listed} => {{}},"), lines("_ => {", &[statement.to_string()], "},")])
    };
    block(&format!("for (let key in {keys})"), &[body])
}

/// An optional field the request encoding leaves out when unset.
fn omittable(field: &Field) -> bool {
    !field.required && field.ty.is_optional()
}

/// One function per operation: its doc comment, its signature, and one call
/// to the `root.internal` helper that sends the request and decodes the
/// response (`baml/baml_src/ns_internal/http.baml`).
pub fn operation(service: &str, kind: ClientKind, op: &Operation, index: &Index) -> Result<String, Error> {
    // An operation without a group is in its service's own namespace.
    let qualify = |n: &str| match op.group {
        None => n.to_string(),
        Some(_) => format!("root.{service}.{n}"),
    };
    let mut out = String::from("\n");
    doc(&mut out, "", &operation_doc(op));

    let client_type = match kind {
        ClientKind::Platform => "PlatformClient",
        ClientKind::Manager => "ManagerClient",
    };
    let mut arguments = vec![format!("client: {}", qualify(client_type))];
    for arg in &op.args {
        if arg.required {
            arguments.push(format!("{}: {}", arg.name, arg.ty.render(&qualify)));
        } else {
            arguments.push(format!("{}: {} = null", arg.name, optional(arg.ty.clone()).render(&qualify)));
        }
    }
    let returns = match &op.response {
        Response::Json(ty) => ty.render(&qualify),
        Response::OptionalJson(ty) => optional(ty.clone()).render(&qualify),
        Response::Empty => "null".to_string(),
    };
    let _ = writeln!(out, "function {}({}) -> {returns} throws root.errors.AlienError {{", op.name, arguments.join(", "));

    // Named arguments of the helper, in the order of an HTTP request.
    let mut named = Vec::new();
    let mut query = Vec::new();
    if op.workspace {
        query.push("\"workspace\": client.workspace".to_string());
    }
    for arg in op.params(Location::Query) {
        // ops::check_param_type admits only the types of `internal.QueryValue`.
        query.push(format!("{}: {}", quote(arg.wire.as_deref().expect("wire")), arg.name));
    }
    if !query.is_empty() {
        named.push(format!("query = {}", map_literal(&query, "query = ".len() + 2 * INDENT.len())));
    }
    // ops::lower_operation admits at most one header parameter.
    for arg in op.params(Location::Header) {
        named.push(format!("headers = {{ {}: {} }}", quote(&arg.wire.as_deref().expect("wire").to_lowercase()), arg.name));
    }
    if let Some(arg) = op.body() {
        named.push(format!("body = root.internal.json_body({})", arg.name));
    }
    let helper = match &op.response {
        // An open union decodes by its tag (Index), from the body as JSON.
        Response::Json(ty) if index.has_open(ty) => {
            let decode = block("(response) ->", &[index.decode(ty, "response", &qualify)]);
            named.push(format!("decode = {decode}"));
            format!("decoded<{}>", ty.render(&qualify))
        }
        Response::OptionalJson(ty) if index.has_open(ty) => {
            return Err(Error::new(format!(
                "{} {} (operation {}): a response that may be empty and holds an open union is not implemented",
                op.method, op.path, op.id
            )));
        }
        Response::Json(ty) => format!("json<{}>", ty.render(&qualify)),
        Response::OptionalJson(ty) => format!("optional_json<{}>", ty.render(&qualify)),
        Response::Empty => "empty".to_string(),
    };
    let prefix = match kind {
        ClientKind::Platform => "platform",
        ClientKind::Manager => "manager",
    };
    let call = format!("root.internal.{prefix}_{helper}");
    let target = format!("client, {}, {}", quote(&op.method), path_expr(op));
    let body = if named.is_empty() {
        format!("{call}({target})")
    } else {
        let arguments: Vec<String> = std::iter::once(target).chain(named).map(|a| format!("{a},")).collect();
        lines(&format!("{call}("), &arguments, ")")
    };
    let _ = writeln!(out, "{}", nest(&body));
    let _ = writeln!(out, "}}");
    Ok(out)
}

/// The operation's summary, description, method and path, and its documented
/// arguments.
fn operation_doc(op: &Operation) -> String {
    let mut paragraphs = Vec::new();
    if !op.summary.is_empty() {
        paragraphs.push(op.summary.clone());
    }
    if !op.description.is_empty() && op.description != op.summary {
        paragraphs.push(op.description.clone());
    }
    let deprecated = if op.deprecated { " Deprecated." } else { "" };
    paragraphs.push(format!("`{} {}` (operation `{}`).{deprecated}", op.method, op.path, op.id));
    let arguments: Vec<String> =
        op.args.iter().filter(|a| !a.doc.is_empty()).map(|a| format!("- `{}`: {}", a.name, a.doc.replace('\n', "\n  "))).collect();
    if !arguments.is_empty() {
        paragraphs.push(arguments.join("\n"));
    }
    paragraphs.join("\n\n")
}

/// The operation's path: a string literal, or a backtick string with each
/// path parameter percent-encoded (`root.internal.path_segment`).
fn path_expr(op: &Operation) -> String {
    let mut text = String::new();
    let mut rest = op.path.as_str();
    while let Some(start) = rest.find('{') {
        // ops::lower_operation checked the template against the path parameters.
        let end = rest[start..].find('}').map(|e| start + e).expect("closed path template");
        let wire = &rest[start + 1..end];
        let arg = op.params(Location::Path).into_iter().find(|a| a.wire.as_deref() == Some(wire)).expect("path parameter");
        let value = match arg.ty {
            Ty::String => arg.name.clone(),
            _ => format!("string.from({})", arg.name),
        };
        let _ = write!(text, "{}${{root.internal.path_segment({value})}}", template_text(&rest[..start]));
        rest = &rest[end + 1..];
    }
    if text.is_empty() { quote(rest) } else { format!("`{text}{}`", template_text(rest)) }
}

/// A map literal of these `key: value` entries: on one line when it fits in
/// [`WIDTH`] after `used` columns, else one entry per line.
fn map_literal(entries: &[String], used: usize) -> String {
    let inline = format!("{{ {} }}", entries.join(", "));
    if used + inline.len() < WIDTH {
        inline
    } else {
        lines("{", &entries.iter().map(|e| format!("{e},")).collect::<Vec<_>>(), "}")
    }
}
