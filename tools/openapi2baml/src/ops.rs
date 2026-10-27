//! Operations → BAML functions (DESIGN.md, "Operations → BAML functions").

use std::collections::BTreeSet;

use serde_json::Value;

use crate::{
    Error,
    config::{Naming, ServiceConfig},
    lower::{Ctx, Def, Lowerer, Ty, doc_of, literal_values, union, widen},
    naming,
    spec::Spec,
};

const METHODS: &[&str] = &["get", "put", "post", "delete", "options", "head", "patch", "trace"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Path,
    Query,
    Header,
}

/// One argument of the generated function, after the client.
#[derive(Clone, Debug)]
pub struct Arg {
    /// Wire name (`None` for the request body).
    pub wire: Option<String>,
    /// Position among the spec's parameters: query parameters are sent in this order.
    pub index: usize,
    pub name: String,
    pub location: Option<Location>,
    /// The type without the optional marker.
    pub ty: Ty,
    /// Required arguments are positional; optional ones default to `null`.
    pub required: bool,
    pub doc: String,
}

#[derive(Clone, Debug)]
pub enum Response {
    Json(Ty),
    OptionalJson(Ty),
    Empty,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub id: String,
    pub method: String,
    pub path: String,
    pub group: Option<String>,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub deprecated: bool,
    /// Required arguments first (path parameters in path order, then other
    /// parameters in spec order, then the body), then optional ones.
    pub args: Vec<Arg>,
    pub workspace: bool,
    pub response: Response,
}

impl Operation {
    /// The parameters at `location`, in spec order.
    pub fn params(&self, location: Location) -> Vec<&Arg> {
        let mut params: Vec<&Arg> = self.args.iter().filter(|a| a.location == Some(location)).collect();
        params.sort_by_key(|a| a.index);
        params
    }

    pub fn body(&self) -> Option<&Arg> {
        self.args.iter().find(|a| a.location.is_none())
    }
}

/// Every operation of the spec: (operationId, method, path, operation object).
pub fn all_operations(spec: &Spec) -> Result<Vec<(String, String, String, Value)>, Error> {
    let paths = spec.doc.get("paths").and_then(Value::as_object).ok_or_else(|| Error::new("spec has no paths"))?;
    let mut out = Vec::new();
    for (path, item) in paths {
        let item = spec.deref(item)?;
        let shared = item.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(*method) else { continue };
            let id = op
                .get("operationId")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new(format!("{} {path}: missing operationId", method.to_uppercase())))?;
            // Path-item parameters apply unless the operation overrides them.
            let mut op = op.clone();
            if !shared.is_empty() {
                let own = op.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut merged = own.clone();
                for parameter in &shared {
                    let resolved = spec.deref(parameter)?;
                    let key = (resolved.get("name"), resolved.get("in"));
                    let overridden = own.iter().any(|p| {
                        spec.deref(p).is_ok_and(|r| (r.get("name"), r.get("in")) == key)
                    });
                    if !overridden {
                        merged.push(parameter.clone());
                    }
                }
                op["parameters"] = Value::Array(merged);
            }
            // Speakeasy leaves these out of the SDK on purpose.
            if op.get("x-speakeasy-ignore") == Some(&Value::Bool(true)) {
                continue;
            }
            out.push((id.to_string(), method.to_uppercase(), path.clone(), op));
        }
    }
    Ok(out)
}

/// The (group, method) Alien's TypeScript SDK exposes an operation under.
pub fn sdk_name(service: &ServiceConfig, id: &str, method: &str, path: &str, op: &Value) -> (Option<String>, String) {
    let first_tag = op.get("tags").and_then(Value::as_array).and_then(|t| t.first()).and_then(Value::as_str).map(naming::camel);
    match service.naming {
        Naming::SpeakeasyGroup => {
            let group = op.get("x-speakeasy-group").and_then(Value::as_str).map(str::to_string).or(first_tag);
            let name = op
                .get("x-speakeasy-name-override")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| naming::camel(id));
            let name = service
                .method_renames
                .iter()
                .find(|r| r.method.eq_ignore_ascii_case(method) && r.path == path)
                .map_or(name, |r| r.name.clone());
            (group, name)
        }
        Naming::FirstTag => (first_tag, naming::camel(id)),
    }
}

/// The operation's parameters (resolved), minus a client-global `workspace`.
fn parameters<'a>(spec: &'a Spec, service: &ServiceConfig, op: &'a Value) -> Result<Vec<&'a Value>, Error> {
    let mut out = Vec::new();
    for parameter in op.get("parameters").and_then(Value::as_array).into_iter().flatten() {
        let parameter = spec.deref(parameter)?;
        let global = service.workspace_global
            && parameter.get("in") == Some(&Value::from("query"))
            && parameter.get("name") == Some(&Value::from("workspace"));
        if !global {
            out.push(parameter);
        }
    }
    Ok(out)
}

fn is_required(parameter: &Value) -> bool {
    parameter.get("in") == Some(&Value::from("path")) || parameter.get("required") == Some(&Value::Bool(true))
}

pub fn lower_operation(
    lowerer: &mut Lowerer,
    service: &ServiceConfig,
    spec: &Spec,
    (id, method, path, op): &(String, String, String, Value),
) -> Result<Operation, Error> {
    let (group, name) = sdk_name(service, id, method, path, op);
    let op_name = naming::pascal(id);
    let pointer = format!("/paths/{}/{}", path.replace('~', "~0").replace('/', "~1"), method.to_lowercase());
    let workspace = service.workspace_global
        && op.get("parameters").and_then(Value::as_array).into_iter().flatten().any(|p| {
            spec.deref(p).is_ok_and(|p| p.get("in") == Some(&Value::from("query")) && p.get("name") == Some(&Value::from("workspace")))
        });

    // Parameters.
    let parameters = parameters(spec, service, op)?;
    let mut args = Vec::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let wire = parameter.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let location = match parameter.get("in").and_then(Value::as_str) {
            Some("path") => Location::Path,
            Some("query") => Location::Query,
            Some("header") => Location::Header,
            other => return Err(Error::new(format!("{pointer}: unsupported parameter location {other:?}"))),
        };
        // Only each location's default serialization is implemented: form
        // style, exploded, in a query; simple style elsewhere.
        let (style, explode) = if location == Location::Query { ("form", true) } else { ("simple", false) };
        if parameter.get("style").is_some_and(|s| s != style)
            || parameter.get("explode").is_some_and(|e| *e != Value::Bool(explode))
            || parameter.get("allowReserved") == Some(&Value::Bool(true))
        {
            return Err(Error::new(format!(
                "{pointer}: parameter `{wire}` sets a serialization other than style {style}, explode {explode}, allowReserved false"
            )));
        }
        let schema = parameter
            .get("schema")
            .cloned()
            .ok_or_else(|| Error::new(format!("{pointer}: parameter `{wire}` has no schema")))?;
        let ctx = Ctx::new(format!("{op_name}{}", naming::pascal(&wire)), format!("{pointer}/parameters/{index}/schema"));
        // A parameter is only serialized, and `"a"[]` is not a `string[]` in
        // BAML: literal types, closed enums included, are widened to their
        // base type.
        let lowered = lowerer.lower(&schema, &ctx)?;
        let lowered = lowerer.inline_enum_aliases(lowered);
        let mut widened = Vec::new();
        literal_values(&lowered, &mut widened);
        let ty = widen(lowered);
        check_param_type(&ty, location, &pointer, &wire)?;
        let mut doc = doc_of(parameter);
        if doc.is_empty() {
            doc = doc_of(&schema);
        }
        lowerer.annotate(&mut doc, &schema, &ty, &widened);
        args.push(Arg {
            name: naming::argument_name(&wire),
            wire: Some(wire),
            index,
            location: Some(location),
            ty: ty.required().clone(),
            required: is_required(parameter),
            doc,
        });
    }
    // One header parameter at most: there is no helper merging several.
    if args.iter().filter(|a| a.location == Some(Location::Header)).count() > 1 {
        return Err(Error::new(format!("{pointer}: more than one header parameter is not implemented")));
    }
    // Every path template variable is a path parameter, and the reverse.
    let variables = path_variables(path).map_err(|e| Error::new(format!("{pointer}: {e}")))?;
    for arg in args.iter().filter(|a| a.location == Some(Location::Path)) {
        let wire = arg.wire.as_deref().unwrap_or_default();
        if !variables.contains(&wire) {
            return Err(Error::new(format!("{pointer}: path parameter `{wire}` is not in the path")));
        }
    }
    for variable in &variables {
        if !args.iter().any(|a| a.location == Some(Location::Path) && a.wire.as_deref() == Some(*variable)) {
            return Err(Error::new(format!("{pointer}: `{{{variable}}}` in the path is not a path parameter")));
        }
    }
    // Path parameters in the order the path template uses them.
    let position = |a: &Arg| match (a.location, &a.wire) {
        (Some(Location::Path), Some(wire)) => path.find(&format!("{{{wire}}}")).unwrap_or(usize::MAX),
        _ => usize::MAX,
    };
    args.sort_by_key(|a| (!a.required, position(a)));

    // Body: `<Op>Request` when it is the only argument, else `<Op>RequestBody`,
    // as in Alien's TypeScript SDK.
    if let Some(request_body) = op.get("requestBody") {
        let request_body = spec.deref(request_body)?;
        let schema = request_body
            .get("content")
            .and_then(Value::as_object)
            .and_then(|c| c.get("application/json"))
            .and_then(|m| m.get("schema"))
            .ok_or_else(|| Error::new(format!("{pointer}: request body is not application/json")))?;
        let class_name = if parameters.is_empty() { format!("{op_name}Request") } else { format!("{op_name}RequestBody") };
        let ty = lowerer.lower(schema, &Ctx::new(class_name, format!("{pointer}/requestBody")))?;
        lowerer.mark_encoded(&ty);
        let required = request_body.get("required") == Some(&Value::Bool(true));
        // `internal.json_body` sends no body for `null`.
        if required && admits_null(lowerer, &ty) {
            return Err(Error::new(format!("{pointer}: a required request body that may be null is not implemented")));
        }
        let body = Arg {
            wire: None,
            index: usize::MAX,
            name: "body".into(),
            location: None,
            ty: ty.required().clone(),
            required,
            doc: doc_of(request_body),
        };
        let at = args.iter().position(|a| !a.required).unwrap_or(args.len());
        args.insert(if required { at } else { args.len() }, body);
    }
    let mut names = BTreeSet::new();
    for arg in &args {
        if !names.insert(arg.name.as_str()) {
            return Err(Error::new(format!("{pointer}: two arguments become `{}`", arg.name)));
        }
    }

    // Responses.
    let mut json = Vec::new();
    let mut empty = false;
    for (status, response) in op.get("responses").and_then(Value::as_object).into_iter().flatten() {
        if !status.starts_with('2') {
            continue;
        }
        let response = spec.deref(response)?;
        match response.get("content").and_then(Value::as_object) {
            None => empty = true,
            Some(content) if content.is_empty() => empty = true,
            Some(content) => {
                let schema = content
                    .get("application/json")
                    .and_then(|m| m.get("schema"))
                    .ok_or_else(|| Error::new(format!("{pointer}: {status} response is not application/json")))?;
                let suffix = if json.is_empty() { String::new() } else { status.clone() };
                let ctx = Ctx::new(format!("{op_name}Response{suffix}"), format!("{pointer}/responses/{status}"));
                json.push(lowerer.lower(schema, &ctx)?);
            }
        }
    }
    let response = match (json.is_empty(), empty) {
        (true, _) => Response::Empty,
        (false, false) => Response::Json(union(json)),
        (false, true) => Response::OptionalJson(union(json)),
    };

    Ok(Operation {
        id: id.clone(),
        method: method.clone(),
        path: path.clone(),
        group,
        name,
        summary: op.get("summary").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
        description: doc_of(op),
        deprecated: op.get("deprecated") == Some(&Value::Bool(true)),
        args,
        workspace,
        response,
    })
}

/// Whether a value of `ty` may be `null`, through type aliases.
fn admits_null(lowerer: &Lowerer, ty: &Ty) -> bool {
    match ty {
        Ty::Named(name) => matches!(lowerer.defs.get(name), Some(Def::Alias { ty, .. }) if admits_null(lowerer, ty)),
        ty => ty.is_optional(),
    }
}

/// The variables of a path template (`/v1/{a}/x/{b}` → `a`, `b`).
fn path_variables(path: &str) -> Result<Vec<&str>, String> {
    let mut variables = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find(['{', '}']) {
        let end = match rest[start..].find('}') {
            Some(end) if rest.as_bytes()[start] == b'{' => start + end,
            _ => return Err(format!("malformed path template `{path}`")),
        };
        let name = &rest[start + 1..end];
        if name.is_empty() || name.contains('{') {
            return Err(format!("malformed path template `{path}`"));
        }
        variables.push(name);
        rest = &rest[end + 1..];
    }
    Ok(variables)
}

/// Parameter types the runtime helpers can serialize: scalars in a path;
/// strings, integers, numbers and string lists (form style, exploded) in a
/// query; strings in a header. Neither spec has a boolean or integer-list
/// query parameter, so there is no helper for them.
fn check_param_type(ty: &Ty, location: Location, pointer: &str, wire: &str) -> Result<(), Error> {
    let ok = match (location, ty.required()) {
        (Location::Path, t) => matches!(t, Ty::String | Ty::Int | Ty::Float | Ty::Bool),
        (Location::Query, Ty::List(item)) => **item == Ty::String,
        (Location::Query, t) => matches!(t, Ty::String | Ty::Int | Ty::Float),
        (Location::Header, t) => *t == Ty::String,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{pointer}: {location:?} parameter `{wire}` has a type the importer cannot serialize: {}",
            ty.render(&|n| n.to_string())
        )))
    }
}

/// Fails when two operations get the same name, or a function the name of a group.
pub fn check_names(ops: &[Operation]) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for op in ops {
        let key = (op.group.clone(), op.name.clone());
        if !seen.insert(key) {
            return Err(Error::new(format!(
                "two operations become `{}.{}` (one is {})",
                op.group.as_deref().unwrap_or("<root>"),
                op.name,
                op.id
            )));
        }
    }
    let groups: BTreeSet<&str> = ops.iter().filter_map(|o| o.group.as_deref()).collect();
    for op in ops.iter().filter(|o| o.group.is_none()) {
        if groups.contains(op.name.as_str()) {
            return Err(Error::new(format!("function `{}` collides with the group of the same name", op.name)));
        }
    }
    Ok(())
}
