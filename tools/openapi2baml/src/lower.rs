//! Lowering OpenAPI schemas to BAML types (DESIGN.md, "Schemas → BAML types").

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use crate::{
    Error,
    coverage::Coverage,
    naming,
    ops::{Operation, Response},
    spec::{Spec, schema_ref_name},
};

/// A BAML type expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    String,
    Int,
    Float,
    Bool,
    Json,
    Null,
    /// A string, integer or boolean literal.
    Literal(Value),
    /// A class or type alias of this service, by BAML name.
    Named(String),
    List(Box<Ty>),
    Map(Box<Ty>),
    Optional(Box<Ty>),
    Union(Vec<Ty>),
}

impl Ty {
    /// Renders the type; `qualify` turns a local type name into the spelling
    /// valid where it is written (bare, or `root.<service>.<Name>`).
    pub fn render(&self, qualify: &dyn Fn(&str) -> String) -> String {
        match self {
            Ty::String => "string".into(),
            Ty::Int => "int".into(),
            Ty::Float => "float".into(),
            Ty::Bool => "bool".into(),
            Ty::Json => "json".into(),
            Ty::Null => "null".into(),
            Ty::Literal(value) => value.to_string(),
            Ty::Named(name) => qualify(name),
            Ty::List(item) => match item.as_ref() {
                Ty::Union(_) => format!("({})[]", item.render(qualify)),
                _ => format!("{}[]", item.render(qualify)),
            },
            Ty::Map(value) => format!("map<string, {}>", value.render(qualify)),
            Ty::Optional(inner) => match inner.as_ref() {
                Ty::Union(_) => format!("({})?", inner.render(qualify)),
                _ => format!("{}?", inner.render(qualify)),
            },
            Ty::Union(members) => members.iter().map(|m| m.render(qualify)).collect::<Vec<_>>().join(" | "),
        }
    }

    pub fn is_optional(&self) -> bool {
        matches!(self, Ty::Optional(_) | Ty::Json | Ty::Null)
    }

    /// The type without its outer `?`.
    pub fn required(&self) -> &Ty {
        match self {
            Ty::Optional(inner) => inner,
            other => other,
        }
    }

    fn is_scalar(&self) -> bool {
        match self {
            Ty::String | Ty::Int | Ty::Float | Ty::Bool | Ty::Json | Ty::Literal(_) => true,
            Ty::Optional(inner) => inner.is_scalar(),
            _ => false,
        }
    }

    /// Named types this type mentions, directly or through containers.
    pub fn names(&self, out: &mut BTreeSet<String>) {
        match self {
            Ty::Named(name) => {
                out.insert(name.clone());
            }
            Ty::List(inner) | Ty::Map(inner) | Ty::Optional(inner) => inner.names(out),
            Ty::Union(members) => members.iter().for_each(|m| m.names(out)),
            _ => {}
        }
    }
}

pub fn optional(ty: Ty) -> Ty {
    match ty {
        Ty::Optional(_) | Ty::Json | Ty::Null => ty,
        other => Ty::Optional(Box::new(other)),
    }
}

/// The type with every literal widened to its base type (`"a"` → `string`,
/// `1` → `int`, `true` → `bool`).
pub fn widen(ty: Ty) -> Ty {
    match ty {
        Ty::Literal(Value::String(_)) => Ty::String,
        Ty::Literal(Value::Bool(_)) => Ty::Bool,
        Ty::Literal(Value::Number(n)) => if n.is_i64() { Ty::Int } else { Ty::Float },
        Ty::List(item) => Ty::List(Box::new(widen(*item))),
        Ty::Map(value) => Ty::Map(Box::new(widen(*value))),
        Ty::Optional(inner) => optional(widen(*inner)),
        Ty::Union(members) => union(members.into_iter().map(widen).collect()),
        other => other,
    }
}

/// The values of the literals in `ty` as doc text, in order, without repeats.
pub fn literal_values(ty: &Ty, out: &mut Vec<String>) {
    match ty {
        Ty::Literal(value) => {
            let text = value.as_str().map_or_else(|| value.to_string(), str::to_string);
            if !out.contains(&text) {
                out.push(text);
            }
        }
        Ty::List(inner) | Ty::Map(inner) | Ty::Optional(inner) => literal_values(inner, out),
        Ty::Union(members) => members.iter().for_each(|m| literal_values(m, out)),
        _ => {}
    }
}

/// Classes that are members of a union somewhere in `ty`.
fn union_members(ty: &Ty, out: &mut BTreeSet<String>) {
    match ty {
        Ty::Union(members) => {
            for member in members {
                if let Ty::Named(name) = member {
                    out.insert(name.clone());
                }
                union_members(member, out);
            }
        }
        Ty::List(inner) | Ty::Map(inner) | Ty::Optional(inner) => union_members(inner, out),
        _ => {}
    }
}

/// The doc line listing the values a type no longer spells out.
fn known_values_line(values: &[String]) -> String {
    format!("Known values: {}. Others may appear.", values.iter().map(|v| format!("`{v}`")).collect::<Vec<_>>().join(", "))
}

/// A flattened, de-duplicated union. `null` members make it optional; a `json`
/// member absorbs everything; a plain `string` absorbs string literals.
pub fn union(members: Vec<Ty>) -> Ty {
    let mut flat = Vec::new();
    let mut nullable = false;
    fn push(ty: Ty, flat: &mut Vec<Ty>, nullable: &mut bool) {
        match ty {
            Ty::Union(members) => members.into_iter().for_each(|m| push(m, flat, nullable)),
            Ty::Optional(inner) => {
                *nullable = true;
                push(*inner, flat, nullable);
            }
            Ty::Null => *nullable = true,
            other => {
                if !flat.contains(&other) {
                    flat.push(other);
                }
            }
        }
    }
    for member in members {
        push(member, &mut flat, &mut nullable);
    }
    if flat.contains(&Ty::Json) {
        return Ty::Json;
    }
    if flat.contains(&Ty::String) {
        flat.retain(|m| !matches!(m, Ty::Literal(Value::String(_))));
    }
    let ty = match flat.len() {
        0 => Ty::Null,
        1 => flat.pop().expect("one member"),
        _ => Ty::Union(flat),
    };
    if nullable { optional(ty) } else { ty }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnumPolicy {
    /// Every multi-value string enum decodes as `string`.
    All,
    /// Only the enums Alien's Rust filter opens (openapi_filter.rs).
    RustFilter,
    /// Every enum is a closed union of literals.
    None,
}

/// Keywords that constrain an untyped schema in ways the importer does not
/// implement: such a schema is typed `json` and counted in the coverage report.
const UNIMPLEMENTED_KEYWORDS: &[&str] = &[
    "not", "if", "then", "else", "patternProperties", "prefixItems", "contains", "dependentSchemas",
    "unevaluatedProperties", "unevaluatedItems", "$dynamicRef",
];

/// The fields Alien's Rust client opens (`OPEN_STRING_ENUM_SCHEMAS`).
const RUST_FILTER_OPEN: &[&str] = &[
    "/components/schemas/CapabilityMaterialization/properties/packages/items/properties/type",
    "/components/schemas/DeploymentLinkSetupResponse/properties/visiblePackageTypes/items",
];

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub wire: String,
    pub ty: Ty,
    pub required: bool,
    pub doc: String,
    /// The property's schema, to rewrite `doc` when a literal type is widened.
    pub property: Value,
}

/// The field holding an open object's extra keys, named as in Speakeasy's SDKs.
pub const EXTRA_FIELD: &str = "additionalProperties";

/// The tag of every open union's `Other` member (`open_unions`), as Speakeasy's
/// forward-compatible unions spell it.
pub const UNKNOWN_TAG: &str = "UNKNOWN";

/// The `Other` member's field holding the member as the server sent it, as
/// Speakeasy names it.
pub const OTHER_RAW_FIELD: &str = "raw";

#[derive(Clone, Debug)]
pub struct Class {
    pub name: String,
    pub doc: String,
    pub fields: Vec<Field>,
    /// An object with both `properties` and `additionalProperties`: the type of
    /// the extra values, kept in a `map<string, T>` field named [`EXTRA_FIELD`].
    pub extra: Option<Ty>,
    /// Where the schema came from (JSON pointer), for diagnostics.
    pub origin: String,
    /// A field name differs from its wire name: needs FromJson and ToJson.
    pub renamed: bool,
    /// Reachable from a request body: needs ToJson to omit unset fields.
    pub encoded: bool,
    /// Keeps its name through deduplication: a spec component, or an
    /// operation's own request body or response.
    pub pinned: bool,
    /// The `Other` member of an open union (`open_unions`): the wire names of
    /// the union's tag fields, in the order decoding reads them (the first is
    /// this class's tag), and the values the union's other members own.
    pub open_tag: Option<(Vec<String>, Vec<String>)>,
}

/// A union member's value for the tag field `wire`: its string literal.
fn tag_value<'c>(class: &'c Class, wire: &str) -> Option<&'c str> {
    class.fields.iter().find(|f| f.wire == wire).and_then(|f| match &f.ty {
        Ty::Literal(Value::String(value)) => Some(value.as_str()),
        _ => None,
    })
}

/// The fields that tell the members of a union of classes apart, in the
/// order decoding reads them, or `None` when there are none.
///
/// Usually one: the first field of the first member that every member has as
/// a different string literal (`type`). Failing that, several, as for
/// `ExternalBinding` (`type`, then `service` where members share a `type`):
/// the first field of the first member that every member has as a string
/// literal, then, while some members agree on every field so far, the first
/// field of the first of them that all of them have as a string literal.
fn tag_fields(classes: &[&Class]) -> Option<Vec<String>> {
    let literal_in_all = |members: &[&Class], wire: &str| members.iter().all(|c| tag_value(c, wire).is_some());
    let distinct = classes[0].fields.iter().find(|field| {
        let values = classes.iter().map(|c| tag_value(c, &field.wire)).collect::<Option<Vec<&str>>>();
        values.is_some_and(|values| values.iter().collect::<BTreeSet<_>>().len() == values.len())
    });
    if let Some(field) = distinct {
        return Some(vec![field.wire.clone()]);
    }
    let mut fields = vec![classes[0].fields.iter().find(|f| literal_in_all(classes, &f.wire))?.wire.clone()];
    loop {
        let key = |class: &Class| fields.iter().map(|f| tag_value(class, f).map(str::to_owned)).collect::<Vec<_>>();
        let shared: Vec<&Class> =
            classes.iter().copied().filter(|c| classes.iter().filter(|other| key(other) == key(c)).count() > 1).collect();
        let Some(first) = shared.first() else { return Some(fields) };
        let next = first.fields.iter().find(|f| !fields.contains(&f.wire) && literal_in_all(&shared, &f.wire))?;
        fields.push(next.wire.clone());
    }
}

/// How decoding picks a member of an open union by its tag fields.
pub enum TagTree<'c> {
    /// Decode as this member.
    Member(&'c Class),
    /// Read the tag field `wire` and go by its value. Any other value is the
    /// union's `Other` member.
    Read { wire: String, arms: Vec<(String, TagTree<'c>)> },
}

/// How decoding picks each of `classes` by reading the tag `fields` in
/// order, or `None` when they do not tell every member apart. A member is
/// picked only when every tag field it has matches, so a known `type` with
/// a `service` no member has is the `Other` member. A member may lack a
/// later tag field when the earlier ones already tell it apart
/// (`ExternalBinding`'s `ai` has no `service`).
pub fn tag_tree<'c>(classes: &[&'c Class], fields: &[String]) -> Option<TagTree<'c>> {
    if let [only] = classes
        && fields.iter().all(|f| tag_value(only, f).is_none())
    {
        return Some(TagTree::Member(only));
    }
    let (wire, rest) = fields.split_first()?;
    let mut groups: Vec<(&str, Vec<&Class>)> = Vec::new();
    for &class in classes {
        let value = tag_value(class, wire)?;
        match groups.iter_mut().find(|(v, _)| *v == value) {
            Some((_, group)) => group.push(class),
            None => groups.push((value, vec![class])),
        }
    }
    let arms = groups
        .into_iter()
        .map(|(value, group)| Some((value.to_string(), tag_tree(&group, rest)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(TagTree::Read { wire: wire.clone(), arms })
}

#[derive(Clone, Debug)]
pub enum Def {
    Class(Class),
    Alias { name: String, doc: String, ty: Ty },
}

impl Def {
    pub fn name(&self) -> &str {
        match self {
            Def::Class(c) => &c.name,
            Def::Alias { name, .. } => name,
        }
    }
}

enum State {
    InProgress { referenced: bool },
    Done(Ty),
}

pub struct Lowerer<'a> {
    spec: &'a Spec,
    policy: EnumPolicy,
    pub defs: BTreeMap<String, Def>,
    components: HashMap<String, State>,
    pub coverage: &'a mut Coverage,
}

/// Where a schema appears: the name an inline object would get, and its JSON
/// pointer.
#[derive(Clone)]
pub struct Ctx {
    pub name: String,
    pub pointer: String,
    /// The class lowered here keeps its name through deduplication.
    pinned: bool,
}

impl Ctx {
    pub fn new(name: impl Into<String>, pointer: impl Into<String>) -> Ctx {
        let pointer = pointer.into();
        Ctx { name: name.into(), pinned: is_pinned(&pointer), pointer }
    }
    /// A nested position; with an empty suffix it keeps the parent's name (and
    /// whether that name is pinned).
    fn child(&self, suffix: &str, pointer: &str) -> Ctx {
        Ctx {
            name: format!("{}{suffix}", self.name),
            pointer: format!("{}/{pointer}", self.pointer),
            pinned: suffix.is_empty() && self.pinned,
        }
    }
}

fn is_annotation_only(schema: &Value) -> bool {
    schema.as_object().is_some_and(|o| {
        o.keys().all(|k| {
            matches!(
                k.as_str(),
                "description" | "title" | "example" | "examples" | "deprecated" | "readOnly" | "writeOnly" | "default"
            )
        })
    })
}

/// A schema without documentation-only keys, for comparing shapes.
fn strip_annotations(schema: &Value) -> Value {
    match schema {
        Value::Object(o) => Value::Object(
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "description" | "title" | "example" | "examples"))
                .map(|(k, v)| (k.clone(), strip_annotations(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_annotations).collect()),
        other => other.clone(),
    }
}

fn json_pointer_escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

pub fn doc_of(schema: &Value) -> String {
    schema.get("description").and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// A property's description, else one on a documentation-only `allOf` part,
/// else the one on its only non-null `oneOf`/`anyOf` member.
fn property_doc(property: &Value) -> String {
    let own = doc_of(property);
    if !own.is_empty() {
        return own;
    }
    if let Some(parts) = property.get("allOf").and_then(Value::as_array)
        && let Some(doc) = parts.iter().map(doc_of).find(|d| !d.is_empty())
    {
        return doc;
    }
    let members = property.get("oneOf").or_else(|| property.get("anyOf")).and_then(Value::as_array);
    let non_null: Vec<&Value> = members.into_iter().flatten().filter(|m| !is_null_schema(m)).collect();
    match non_null.as_slice() {
        [only] => doc_of(only),
        _ => String::new(),
    }
}

/// `{type: "null"}`, `{enum: [null]}` and similar members of a nullable union.
fn is_null_schema(schema: &Value) -> bool {
    schema.get("type") == Some(&Value::from("null"))
        || schema.get("enum").and_then(Value::as_array).is_some_and(|v| !v.is_empty() && v.iter().all(Value::is_null))
}

/// Whether a class with this pointer keeps its name through deduplication: a
/// component, or an operation's own request body or response.
fn is_pinned(pointer: &str) -> bool {
    let segments: Vec<&str> = pointer.split('/').collect();
    matches!(segments.as_slice(), ["", "components", "schemas", _])
        || matches!(segments.as_slice(), ["", "paths", _, _, "requestBody"])
        || matches!(segments.as_slice(), ["", "paths", _, _, "responses", _])
}

/// Under `open_enums: "rust-filter"`, fails unless every field Alien's Rust
/// filter opens is still an enum in `spec`, as that filter does. A spec
/// without the field's component (the Manager's) is fine.
pub fn check_policy(spec: &Spec, policy: EnumPolicy) -> Result<(), Error> {
    if policy != EnumPolicy::RustFilter {
        return Ok(());
    }
    for pointer in RUST_FILTER_OPEN {
        let component = pointer.split('/').take(4).collect::<Vec<_>>().join("/");
        if spec.doc.pointer(&component).is_some()
            && spec.doc.pointer(pointer).and_then(|schema| schema.get("enum")).is_none()
        {
            return Err(Error::new(format!(
                "open_enums \"rust-filter\": {pointer} is missing or not an enum; update RUST_FILTER_OPEN from Alien's openapi_filter.rs"
            )));
        }
    }
    Ok(())
}

impl<'a> Lowerer<'a> {
    pub fn new(spec: &'a Spec, policy: EnumPolicy, coverage: &'a mut Coverage) -> Self {
        Lowerer { spec, policy, defs: BTreeMap::new(), components: HashMap::new(), coverage }
    }

    /// The BAML name of a component schema.
    pub fn component_name(name: &str) -> String {
        let ident = naming::identifier(name, &[]);
        let mut chars = ident.chars();
        match chars.next() {
            Some(first) if first.is_ascii_lowercase() => first.to_ascii_uppercase().to_string() + chars.as_str(),
            _ => ident,
        }
    }

    pub fn lower_ref(&mut self, reference: &str) -> Result<Ty, Error> {
        let component = schema_ref_name(reference)
            .ok_or_else(|| Error::new(format!("unsupported schema $ref {reference}")))?;
        let name = Self::component_name(&component);
        match self.components.get_mut(&component) {
            Some(State::Done(ty)) => return Ok(ty.clone()),
            Some(State::InProgress { referenced, .. }) => {
                *referenced = true;
                return Ok(Ty::Named(name));
            }
            None => {}
        }
        let schema = self
            .spec
            .schemas()
            .get(&component)
            .ok_or_else(|| Error::new(format!("unresolvable $ref {reference}")))?
            .clone();
        self.components.insert(component.clone(), State::InProgress { referenced: false });
        let pointer = format!("/components/schemas/{}", json_pointer_escape(&component));
        let ty = self.lower(&schema, &Ctx::new(name.clone(), pointer))?;
        let referenced = matches!(self.components.get(&component), Some(State::InProgress { referenced: true, .. }));
        let done = match &ty {
            // An object component became a class under its own name.
            Ty::Named(n) if *n == name => ty.clone(),
            Ty::Optional(inner) if matches!(inner.as_ref(), Ty::Named(n) if *n == name) => ty.clone(),
            // IDs, open enums and other scalars are inlined where used.
            t if t.is_scalar() && !referenced => ty.clone(),
            _ => {
                self.define(Def::Alias { name: name.clone(), doc: doc_of(&schema), ty })?;
                Ty::Named(name)
            }
        };
        self.components.insert(component, State::Done(done.clone()));
        Ok(done)
    }

    pub fn lower(&mut self, schema: &Value, ctx: &Ctx) -> Result<Ty, Error> {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let ty = self.lower_ref(reference)?;
            return Ok(if schema.get("nullable") == Some(&Value::Bool(true)) { optional(ty) } else { ty });
        }
        let types: Vec<&str> = match schema.get("type") {
            Some(Value::String(t)) => vec![t.as_str()],
            Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let enum_has_null = schema.get("enum").and_then(Value::as_array).is_some_and(|v| v.contains(&Value::Null));
        let nullable =
            schema.get("nullable") == Some(&Value::Bool(true)) || types.contains(&"null") || enum_has_null;
        let non_null: Vec<&str> = types.iter().copied().filter(|t| *t != "null").collect();

        let base = if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
            self.lower_all_of(schema, parts, ctx)?
        } else if let Some(members) =
            schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array)
        {
            self.lower_union(schema, members, ctx)?
        } else if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            self.lower_enum(values, ctx)
        } else if let Some(value) = schema.get("const") {
            match value {
                Value::String(_) | Value::Bool(_) | Value::Number(_) => Ty::Literal(value.clone()),
                _ => self.fallback(ctx, "object or array const"),
            }
        } else if non_null.len() > 1 {
            let members = non_null
                .iter()
                .map(|t| self.lower_typed(schema, t, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            union(members)
        } else if let Some(t) = non_null.first() {
            self.lower_typed(schema, t, ctx)?
        } else if !types.is_empty() {
            Ty::Null
        } else if schema.get("properties").is_some() || schema.get("additionalProperties").is_some() {
            self.lower_object(schema, ctx)?
        } else if schema.get("items").is_some() {
            self.lower_typed(schema, "array", ctx)?
        } else if let Some(keyword) = UNIMPLEMENTED_KEYWORDS.iter().find(|k| schema.get(**k).is_some()) {
            self.fallback(ctx, &format!("`{keyword}` is not implemented"))
        } else {
            // `{}`, `{nullable: true}` and other untyped schemas.
            Ty::Json
        };
        Ok(if nullable { optional(base) } else { base })
    }

    fn lower_typed(&mut self, schema: &Value, t: &str, ctx: &Ctx) -> Result<Ty, Error> {
        Ok(match t {
            "string" => Ty::String,
            "integer" => Ty::Int,
            "number" => Ty::Float,
            "boolean" => Ty::Bool,
            "null" => Ty::Null,
            "array" => match schema.get("items") {
                Some(items) => Ty::List(Box::new(self.lower(items, &ctx.child("Item", "items"))?)),
                None => Ty::List(Box::new(Ty::Json)),
            },
            "object" => self.lower_object(schema, ctx)?,
            other => self.fallback(ctx, &format!("unknown type `{other}`")),
        })
    }

    fn lower_object(&mut self, schema: &Value, ctx: &Ctx) -> Result<Ty, Error> {
        let properties = schema.get("properties").and_then(Value::as_object);
        let additional = schema.get("additionalProperties");
        if properties.is_none_or(|p| p.is_empty()) {
            return Ok(match additional {
                Some(Value::Object(o)) if !o.is_empty() => Ty::Map(Box::new(
                    self.lower(additional.expect("checked"), &ctx.child("Value", "additionalProperties"))?,
                )),
                Some(Value::Bool(true)) | Some(Value::Object(_)) => Ty::Map(Box::new(Ty::Json)),
                _ => Ty::Json,
            });
        }
        let extra = match additional {
            Some(Value::Object(o)) if !o.is_empty() => Some(self.lower(
                additional.expect("checked"),
                &ctx.child("Value", "additionalProperties"),
            )?),
            Some(Value::Bool(true)) | Some(Value::Object(_)) => Some(Ty::Json),
            _ => None,
        };
        let required: BTreeSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut fields = Vec::new();
        for (wire, property) in properties.expect("checked") {
            let child = ctx.child(&naming::pascal(wire), &format!("properties/{}", json_pointer_escape(wire)));
            let ty = self.lower(property, &child)?;
            let is_required = required.contains(wire.as_str());
            let ty_for_doc = ty.clone();
            fields.push(Field {
                name: naming::field_name(wire),
                wire: wire.clone(),
                // `json` admits null, but BAML still requires the key: an
                // optional property must be `json?` to tolerate its absence.
                ty: if is_required { ty } else if ty == Ty::Json { Ty::Optional(Box::new(Ty::Json)) } else { optional(ty) },
                required: is_required,
                doc: self.field_doc(property, &ty_for_doc, &[]),
                property: property.clone(),
            });
        }
        if extra.is_some() {
            if fields.iter().any(|f| f.name == EXTRA_FIELD) {
                return Err(Error::new(format!(
                    "{}: a property named `{EXTRA_FIELD}` collides with the field for additional properties",
                    ctx.pointer
                )));
            }
            self.coverage.count("objects keeping additional properties");
        }
        self.class(ctx, doc_of(schema), fields, extra)
    }

    fn field_doc(&self, property: &Value, ty: &Ty, widened: &[String]) -> String {
        let mut doc = property_doc(property);
        self.annotate(&mut doc, property, ty, widened);
        doc
    }

    /// Appends what the BAML type no longer says: the known values of an
    /// enum decoded as an open `string` (also as list items or map values) or
    /// of the `widened` literals, and the default the server applies when the
    /// value is left out.
    pub fn annotate(&self, doc: &mut String, schema: &Value, ty: &Ty, widened: &[String]) {
        let mut lines = Vec::new();
        if let Some(values) = self.known_values(schema, ty).or_else(|| (!widened.is_empty()).then(|| widened.to_vec())) {
            lines.push(known_values_line(&values));
        }
        if let Some(default) = self.resolve(schema).get("default").or_else(|| schema.get("default")) {
            lines.push(format!("Default: `{default}` (applied by the server when left out)."));
        }
        for line in lines {
            if !doc.is_empty() {
                doc.push_str("\n\n");
            }
            doc.push_str(&line);
        }
    }

    /// Follows `$ref`s, single-schema `allOf` wrappers and nullable unions.
    fn resolve<'s>(&'s self, mut schema: &'s Value) -> &'s Value {
        for _ in 0..8 {
            if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
                match self.spec.pointer(reference) {
                    Ok(target) => schema = target,
                    Err(_) => break,
                }
            } else if let Some([only]) = schema.get("allOf").and_then(Value::as_array).map(Vec::as_slice) {
                schema = only;
            } else if let Some(members) = schema.get("oneOf").or_else(|| schema.get("anyOf")).and_then(Value::as_array)
                && let [only] = members.iter().filter(|m| !is_null_schema(m)).collect::<Vec<_>>().as_slice()
            {
                schema = only;
            } else {
                break;
            }
        }
        schema
    }

    /// The values of a multi-value string enum that `ty` decodes as an open
    /// `string`, directly or as list items or map values.
    fn known_values(&self, schema: &Value, ty: &Ty) -> Option<Vec<String>> {
        let schema = self.resolve(schema);
        match ty.required() {
            Ty::String => {
                let values: Vec<String> =
                    schema.get("enum")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
                (values.len() > 1).then_some(values)
            }
            Ty::List(item) => self.known_values(schema.get("items")?, item),
            Ty::Map(value) => self.known_values(schema.get("additionalProperties")?, value),
            _ => None,
        }
    }

    /// Registers a class; an identical class under the same name is reused,
    /// a different one is an error.
    fn class(&mut self, ctx: &Ctx, doc: String, fields: Vec<Field>, extra: Option<Ty>) -> Result<Ty, Error> {
        let mut seen = BTreeMap::new();
        for field in &fields {
            if let Some(previous) = seen.insert(field.name.clone(), field.wire.clone()) {
                return Err(Error::new(format!(
                    "{}: fields `{previous}` and `{}` both become `{}`",
                    ctx.pointer, field.wire, field.name
                )));
            }
        }
        let renamed = fields.iter().any(|f| f.name != f.wire);
        let pinned = ctx.pinned;
        let class =
            Class { name: ctx.name.clone(), doc, fields, extra, origin: ctx.pointer.clone(), renamed, encoded: false, pinned, open_tag: None };
        if renamed {
            for field in class.fields.iter().filter(|f| f.name != f.wire) {
                self.coverage.rename(&class.name, &field.wire, &field.name, &ctx.pointer);
            }
        }
        self.define(Def::Class(class))?;
        Ok(Ty::Named(ctx.name.clone()))
    }

    fn define(&mut self, def: Def) -> Result<(), Error> {
        let name = def.name().to_string();
        if let Some(existing) = self.defs.get(&name) {
            let same = match (existing, &def) {
                (Def::Class(a), Def::Class(b)) => {
                    a.extra == b.extra
                        && a.fields.len() == b.fields.len()
                        && a.fields
                            .iter()
                            .zip(&b.fields)
                            .all(|(x, y)| x.name == y.name && x.wire == y.wire && x.ty == y.ty && x.required == y.required)
                }
                (Def::Alias { ty: a, .. }, Def::Alias { ty: b, .. }) => a == b,
                _ => false,
            };
            if !same {
                let origin = |d: &Def| match d {
                    Def::Class(c) => c.origin.clone(),
                    Def::Alias { .. } => "a type alias".into(),
                };
                return Err(Error::new(format!(
                    "two different types are both named `{name}` ({} and {})",
                    origin(existing),
                    origin(&def)
                )));
            }
            // The same class lowered at several places keeps the one with the
            // smallest pointer (its doc and its key in deduplication), whatever
            // order the operations are lowered in.
            if let (Some(Def::Class(existing)), Def::Class(mut class)) = (self.defs.get_mut(&name), def)
                && class.origin < existing.origin
            {
                class.encoded |= existing.encoded;
                class.pinned |= existing.pinned;
                *existing = class;
            }
            return Ok(());
        }
        self.defs.insert(name, def);
        Ok(())
    }

    fn lower_all_of(&mut self, schema: &Value, parts: &[Value], ctx: &Ctx) -> Result<Ty, Error> {
        let parts: Vec<&Value> = parts.iter().filter(|p| !is_annotation_only(p)).collect();
        if let [only] = parts.as_slice() {
            return self.lower(only, ctx);
        }
        // `allOf: [oneOf/anyOf: [A, B], {…}]` is `A ∧ {…} | B ∧ {…}`, also when
        // the union part is a `$ref` (`allOf: [$ref: StorageBinding, {…}]`).
        let mut union_parts = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let part = self.spec.deref(part)?;
            if part.get("properties").is_none() && (part.get("oneOf").is_some() || part.get("anyOf").is_some()) {
                union_parts.push((i, part));
            }
        }
        if let [(index, union_part)] = union_parts.as_slice() {
            let index = *index;
            let members = union_part.get("oneOf").or_else(|| union_part.get("anyOf")).and_then(Value::as_array).expect("union part");
            let mut lowered = Vec::new();
            for (i, member) in members.iter().enumerate() {
                if member.get("enum").and_then(Value::as_array).is_some_and(|v| v.iter().all(Value::is_null)) {
                    lowered.push(Ty::Null);
                    continue;
                }
                let mut combined: Vec<Value> = parts.iter().map(|p| (*p).clone()).collect();
                combined[index] = member.clone();
                let mut synthetic = serde_json::Map::new();
                synthetic.insert("allOf".into(), Value::Array(combined));
                if let Some(description) = schema.get("description") {
                    synthetic.insert("description".into(), description.clone());
                }
                let suffix = Self::member_suffix(member, None).unwrap_or_else(|| (i + 1).to_string());
                lowered.push(self.lower(&Value::Object(synthetic), &ctx.child(&suffix, &format!("allOf/{index}/{i}")))?);
            }
            self.coverage.count("allOf distributed over a union");
            return Ok(union(lowered));
        }
        // Several parts: merge their properties into one class.
        let mut properties = serde_json::Map::new();
        let mut required = Vec::<Value>::new();
        let mut stack: Vec<&Value> = parts.clone();
        stack.reverse();
        while let Some(part) = stack.pop() {
            let part = self.spec.deref(part)?;
            if let Some(nested) = part.get("allOf").and_then(Value::as_array) {
                nested.iter().rev().filter(|p| !is_annotation_only(p)).for_each(|p| stack.push(p));
                continue;
            }
            let object_like = part.get("properties").is_some() || part.get("type") == Some(&Value::from("object"));
            if !object_like {
                return Ok(self.fallback(ctx, "allOf with a part that is not an object"));
            }
            if part.get("additionalProperties").is_some_and(|a| *a != Value::Bool(false)) {
                return Err(Error::new(format!(
                    "{}: merging an allOf part with additionalProperties is not implemented",
                    ctx.pointer
                )));
            }
            for (key, value) in part.get("properties").and_then(Value::as_object).into_iter().flatten() {
                match properties.get(key) {
                    Some(existing) if strip_annotations(existing) != strip_annotations(value) => {
                        return Err(Error::new(format!("{}: allOf parts disagree on property `{key}`", ctx.pointer)));
                    }
                    Some(_) => {}
                    None => {
                        properties.insert(key.clone(), value.clone());
                    }
                }
            }
            for name in part.get("required").and_then(Value::as_array).into_iter().flatten() {
                if !required.contains(name) {
                    required.push(name.clone());
                }
            }
        }
        let mut merged = serde_json::Map::new();
        merged.insert("type".into(), "object".into());
        merged.insert("properties".into(), Value::Object(properties));
        merged.insert("required".into(), Value::Array(required));
        if let Some(description) = schema.get("description") {
            merged.insert("description".into(), description.clone());
        }
        self.coverage.count("allOf merged into one class");
        self.lower_object(&Value::Object(merged), ctx)
    }

    fn lower_union(&mut self, schema: &Value, members: &[Value], ctx: &Ctx) -> Result<Ty, Error> {
        let kind = if schema.get("oneOf").is_some() { "oneOf" } else { "anyOf" };
        let tag = schema.get("discriminator").and_then(|d| d.get("propertyName")).and_then(Value::as_str);
        let non_null = members.iter().filter(|m| !is_null_schema(m)).count();
        let mut lowered = Vec::new();
        for (index, member) in members.iter().enumerate() {
            // A nullable wrapper's one real member keeps the parent's name.
            let suffix = if non_null == 1 {
                String::new()
            } else {
                Self::member_suffix(member, tag).unwrap_or_else(|| (index + 1).to_string())
            };
            lowered.push(self.lower(member, &ctx.child(&suffix, &format!("{kind}/{index}")))?);
        }
        if let Some(discriminator) = schema.get("discriminator") {
            self.apply_discriminator(discriminator, ctx)?;
        }
        let ty = union(lowered);
        if let Ty::Union(members) = &ty
            && members.iter().any(|m| matches!(m, Ty::List(_) | Ty::Map(_)))
            && members.iter().any(|m| matches!(m, Ty::Named(_)))
        {
            self.coverage.count("unions mixing containers and classes");
        }
        Ok(ty)
    }

    /// A readable suffix for an inline union member: the value of its tag
    /// property (the discriminator, else any single-value property, looking
    /// into `allOf` parts too), else the key of an externally tagged variant
    /// (`{"Aws": {…}}`).
    fn member_suffix(member: &Value, tag: Option<&str>) -> Option<String> {
        if member.get("$ref").is_some() {
            return None;
        }
        let mut properties: Vec<(&String, &Value)> = Vec::new();
        for part in std::iter::once(member).chain(member.get("allOf").and_then(Value::as_array).into_iter().flatten()) {
            properties.extend(part.get("properties").and_then(Value::as_object).into_iter().flatten());
        }
        let literal = |property: &Value| match property.get("enum").and_then(Value::as_array).map(Vec::as_slice) {
            Some([Value::String(value)]) => Some(value.clone()),
            _ => property.get("const").and_then(Value::as_str).map(str::to_string),
        };
        let tagged = properties
            .iter()
            .filter(|(key, _)| tag.is_none_or(|t| t == key.as_str()))
            .find_map(|(_, property)| literal(property));
        if let Some(value) = tagged.or_else(|| properties.iter().find_map(|(_, property)| literal(property))) {
            return Some(naming::pascal(&value));
        }
        let required = member.get("required").and_then(Value::as_array);
        match properties.as_slice() {
            [(key, property)]
                if (property.get("$ref").is_some() || property.get("properties").is_some())
                    && required.is_some_and(|r| r.iter().any(|k| k.as_str() == Some(key.as_str()))) =>
            {
                Some(naming::pascal(key))
            }
            _ => None,
        }
    }

    /// Pins each mapped class's discriminator field to its literal value.
    fn apply_discriminator(&mut self, discriminator: &Value, ctx: &Ctx) -> Result<(), Error> {
        let property = discriminator.get("propertyName").and_then(Value::as_str).unwrap_or_default();
        let Some(mapping) = discriminator.get("mapping").and_then(Value::as_object) else {
            self.coverage.note(&ctx.pointer, "discriminator without mapping: members decode by shape");
            return Ok(());
        };
        for (value, reference) in mapping {
            let Some(name) = reference.as_str().and_then(schema_ref_name).map(|n| Self::component_name(&n)) else {
                continue;
            };
            if let Some(Def::Class(class)) = self.defs.get_mut(&name)
                && let Some(field) = class.fields.iter_mut().find(|f| f.wire == property)
            {
                field.ty = Ty::Literal(Value::String(value.clone()));
                field.required = true;
            }
        }
        Ok(())
    }

    fn lower_enum(&mut self, values: &[Value], ctx: &Ctx) -> Ty {
        let strings: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
        let others: Vec<&Value> = values.iter().filter(|v| !v.is_null() && !v.is_string()).collect();
        if !others.is_empty() {
            return match (strings.is_empty(), others.as_slice()) {
                (true, [one]) => Ty::Literal((*one).clone()),
                (true, _) if others.iter().all(|v| v.is_i64()) => Ty::Int,
                (true, _) if others.iter().all(|v| v.is_boolean()) => Ty::Bool,
                _ => self.fallback(ctx, "enum mixing strings and other values"),
            };
        }
        match strings.as_slice() {
            [] => Ty::Null,
            [one] => Ty::Literal(Value::String((*one).to_string())),
            many => {
                let open = match self.policy {
                    EnumPolicy::All => true,
                    EnumPolicy::RustFilter => RUST_FILTER_OPEN.contains(&ctx.pointer.as_str()),
                    EnumPolicy::None => false,
                };
                if open {
                    self.coverage.count("open enums (decoded as string)");
                    Ty::String
                } else {
                    self.coverage.count("closed enums (union of literals)");
                    Ty::Union(many.iter().map(|v| Ty::Literal(Value::String((*v).to_string()))).collect())
                }
            }
        }
    }

    /// `open_unions`: a union whose members are all classes with a common
    /// literal tag field (for example `type`), or several that tell them apart
    /// together ([`tag_fields`]), gets one more member, last:
    /// `<Owner><Field>Other`, whose first tag field is the literal
    /// [`UNKNOWN_TAG`] and whose `raw` holds the member as the server sent it.
    /// Its tag never matches a server's, so emit decodes an open union by its
    /// tags (DESIGN.md, "Open unions"): known tags as their own class, any
    /// others as the `Other` class, instead of failing the whole response.
    /// Request types get it too: they share classes with responses, and it
    /// lets a caller send a member this SDK does not know yet. Runs after
    /// [`Self::widen_untagged_literals`].
    pub fn open_tagged_unions(&mut self, ops: &mut [Operation]) -> Result<(), Error> {
        // Where each opened union is, its opened type and its `Other` class.
        enum At {
            Field(String, usize),
            Alias(String),
            Response(usize),
        }
        let mut opened = Vec::new();
        for (name, def) in &self.defs {
            match def {
                Def::Class(class) => {
                    for (index, field) in class.fields.iter().enumerate() {
                        let other = format!("{}{}Other", class.name, naming::pascal(&field.wire));
                        let origin = format!("{}/properties/{}", class.origin, json_pointer_escape(&field.wire));
                        if let Some(open) = self.open_union(&field.ty, &other, &origin) {
                            opened.push((At::Field(name.clone(), index), open?));
                        }
                    }
                }
                Def::Alias { ty, .. } => {
                    let origin = format!("/components/schemas/{}", json_pointer_escape(name));
                    if let Some(open) = self.open_union(ty, &format!("{name}Other"), &origin) {
                        opened.push((At::Alias(name.clone()), open?));
                    }
                }
            }
        }
        for (index, op) in ops.iter().enumerate() {
            if let Response::Json(ty) | Response::OptionalJson(ty) = &op.response {
                let origin = format!("/paths/{}/{}/responses", op.path.replace('~', "~0").replace('/', "~1"), op.method.to_lowercase());
                if let Some(open) = self.open_union(ty, &format!("{}ResponseOther", naming::pascal(&op.id)), &origin) {
                    opened.push((At::Response(index), open?));
                }
            }
        }
        for (at, (ty, other)) in opened {
            match at {
                At::Field(owner, index) => {
                    if let Some(Def::Class(class)) = self.defs.get_mut(&owner) {
                        class.fields[index].ty = ty;
                    }
                }
                At::Alias(name) => {
                    if let Some(Def::Alias { ty: aliased, .. }) = self.defs.get_mut(&name) {
                        *aliased = ty;
                    }
                }
                At::Response(index) => {
                    if let Response::Json(response) | Response::OptionalJson(response) = &mut ops[index].response {
                        *response = ty;
                    }
                }
            }
            self.coverage.count("unions open to new members (an `Other` member)");
            self.define(Def::Class(other))?;
        }
        Ok(())
    }

    /// `ty` with an `Other` member named `name` added to its tagged union of
    /// classes (also inside `?`, lists and maps), and that class.
    fn open_union(&self, ty: &Ty, name: &str, origin: &str) -> Option<Result<(Ty, Class), Error>> {
        match ty {
            Ty::Optional(inner) => self.open_union(inner, name, origin).map(|r| r.map(|(t, c)| (optional(t), c))),
            Ty::List(inner) => self.open_union(inner, name, origin).map(|r| r.map(|(t, c)| (Ty::List(Box::new(t)), c))),
            Ty::Map(inner) => self.open_union(inner, name, origin).map(|r| r.map(|(t, c)| (Ty::Map(Box::new(t)), c))),
            Ty::Union(members) => {
                let classes = members
                    .iter()
                    .map(|m| match m {
                        Ty::Named(n) => match self.defs.get(n) {
                            Some(Def::Class(class)) => Some(class),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect::<Option<Vec<&Class>>>()?;
                let fields = tag_fields(&classes)?;
                tag_tree(&classes, &fields)?;
                let wire = fields[0].clone();
                if classes.iter().any(|c| tag_value(c, &wire) == Some(UNKNOWN_TAG)) {
                    // The sentinel would no longer tell the Other member apart.
                    return Some(Err(Error::new(format!("{origin}: a member's `{wire}` is {UNKNOWN_TAG:?}, the tag of the union's Other member"))));
                }
                let known =
                    classes.iter().map(|c| fields.iter().filter_map(|f| tag_value(c, f)).collect::<Vec<_>>().join("/")).collect();
                // With several tag fields, the docs say how a member is told apart.
                let (raw_doc, by, known_doc) = match &fields[1..] {
                    [] => (String::new(), String::new(), format!("A known `{wire}`")),
                    later => (
                        later.iter().map(|f| format!(", its `{f}` `{OTHER_RAW_FIELD}[{f:?}]`")).collect(),
                        format!(
                            " Members are told apart by `{wire}`{}: this is any combination no other member has, such as a new `{wire}` or a known `{wire}` with a new `{}`.",
                            later.iter().map(|f| format!(", then `{f}`")).collect::<String>(),
                            later[0]
                        ),
                        "A known combination".to_string(),
                    ),
                };
                let tag = Field {
                    name: naming::field_name(&wire),
                    wire: wire.clone(),
                    ty: Ty::Literal(Value::String(UNKNOWN_TAG.to_string())),
                    required: true,
                    doc: format!("Always `{UNKNOWN_TAG:?}`. The server's `{wire}` is `{OTHER_RAW_FIELD}[{wire:?}]`{raw_doc}."),
                    property: Value::Null,
                };
                let raw = Field {
                    name: OTHER_RAW_FIELD.to_string(),
                    wire: OTHER_RAW_FIELD.to_string(),
                    ty: Ty::Map(Box::new(Ty::Json)),
                    required: true,
                    doc: "The member as the server sent it, every key included.".to_string(),
                    property: Value::Null,
                };
                let other = Class {
                    name: name.to_string(),
                    doc: format!(
                        "A member of this union that this SDK does not know, for example one a newer server added.{by} Its `{wire}` is always `{UNKNOWN_TAG:?}`, so code that switches on `{wire}` handles it like any other member; `{OTHER_RAW_FIELD}` holds the member as the server sent it. {known_doc} never decodes as this class."
                    ),
                    renamed: tag.name != tag.wire,
                    fields: vec![tag, raw],
                    extra: None,
                    origin: origin.to_string(),
                    encoded: false,
                    pinned: false,
                    open_tag: Some((fields, known)),
                };
                let mut members = members.clone();
                members.push(Ty::Named(other.name.clone()));
                Some(Ok((Ty::Union(members), other)))
            }
            _ => None,
        }
    }

    /// `ty` with every alias of a closed enum (`open_enums` "none" or
    /// "rust-filter") replaced by its union of literals, also as list items.
    pub fn inline_enum_aliases(&self, ty: Ty) -> Ty {
        match ty {
            Ty::Named(name) => match self.defs.get(&name) {
                Some(Def::Alias { ty, .. })
                    if matches!(ty.required(), Ty::Union(members) if members.iter().all(|m| matches!(m, Ty::Literal(_)))) =>
                {
                    ty.clone()
                }
                _ => Ty::Named(name),
            },
            Ty::List(item) => Ty::List(Box::new(self.inline_enum_aliases(*item))),
            Ty::Optional(inner) => optional(self.inline_enum_aliases(*inner)),
            other => other,
        }
    }

    fn fallback(&mut self, ctx: &Ctx, reason: &str) -> Ty {
        self.coverage.json_fallback(&ctx.pointer, reason);
        Ty::Json
    }

    /// Marks every class reachable from `ty` as encoded (needs ToJson).
    pub fn mark_encoded(&mut self, ty: &Ty) {
        let mut pending = BTreeSet::new();
        ty.names(&mut pending);
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop_first() {
            if !visited.insert(name.clone()) {
                continue;
            }
            match self.defs.get_mut(&name) {
                Some(Def::Class(class)) => {
                    class.encoded = true;
                    for field in &class.fields {
                        field.ty.names(&mut pending);
                    }
                    if let Some(extra) = &class.extra {
                        extra.names(&mut pending);
                    }
                }
                Some(Def::Alias { ty, .. }) => ty.names(&mut pending),
                None => {}
            }
        }
    }

    /// Single-value enums and consts are literals as lowered. Under
    /// `open_enums: "all"`, a literal field of a class that is a member of a
    /// union stays: it is that member's tag, so the union decodes
    /// unambiguously. Every other literal is widened to its base type, as open
    /// enums are, and its values are listed in the doc comment. The other
    /// policies keep every enum closed, single-value ones included, so this
    /// does nothing for them. Runs once every selected operation is lowered.
    pub fn widen_untagged_literals(&mut self, ops: &mut [Operation]) {
        if self.policy != EnumPolicy::All {
            return;
        }
        let mut members = BTreeSet::new();
        for def in self.defs.values() {
            match def {
                Def::Class(class) => {
                    class.fields.iter().for_each(|f| union_members(&f.ty, &mut members));
                    class.extra.iter().for_each(|extra| union_members(extra, &mut members));
                }
                Def::Alias { ty, .. } => union_members(ty, &mut members),
            }
        }
        for op in ops.iter() {
            op.args.iter().for_each(|a| union_members(&a.ty, &mut members));
            if let Response::Json(ty) | Response::OptionalJson(ty) = &op.response {
                union_members(ty, &mut members);
            }
        }
        let mut defs = std::mem::take(&mut self.defs);
        for def in defs.values_mut() {
            match def {
                Def::Class(class) => {
                    let tagged = members.contains(&class.name);
                    for field in &mut class.fields {
                        let mut values = Vec::new();
                        literal_values(&field.ty, &mut values);
                        if values.is_empty() || (tagged && matches!(field.ty.required(), Ty::Literal(_))) {
                            continue;
                        }
                        field.ty = widen(field.ty.clone());
                        field.doc = self.field_doc(&field.property, field.ty.required(), &values);
                        self.coverage.count("literals widened (not a union member's tag)");
                    }
                    class.extra = class.extra.take().map(widen);
                }
                Def::Alias { doc, ty, .. } => {
                    let mut values = Vec::new();
                    literal_values(ty, &mut values);
                    if !values.is_empty() {
                        *ty = widen(ty.clone());
                        if !doc.is_empty() {
                            doc.push_str("\n\n");
                        }
                        doc.push_str(&known_values_line(&values));
                        self.coverage.count("literals widened (not a union member's tag)");
                    }
                }
            }
        }
        self.defs = defs;
        for op in ops.iter_mut() {
            op.args.iter_mut().for_each(|a| a.ty = widen(a.ty.clone()));
            if let Response::Json(ty) | Response::OptionalJson(ty) = &mut op.response {
                *ty = widen(ty.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(ty: &Ty) -> String {
        ty.render(&|n| n.to_string())
    }

    #[test]
    fn unions_normalize() {
        assert_eq!(render(&union(vec![Ty::String, Ty::Literal("a".into())])), "string");
        assert_eq!(render(&union(vec![Ty::Null, Ty::Named("A".into())])), "A?");
        assert_eq!(render(&union(vec![Ty::Named("A".into()), Ty::Optional(Box::new(Ty::Named("B".into())))])), "(A | B)?");
        assert_eq!(render(&union(vec![Ty::Json, Ty::Int])), "json");
        assert_eq!(render(&Ty::List(Box::new(union(vec![Ty::Int, Ty::String])))), "(int | string)[]");
        assert_eq!(render(&Ty::List(Box::new(Ty::Optional(Box::new(Ty::Int))))), "int?[]");
    }
}
