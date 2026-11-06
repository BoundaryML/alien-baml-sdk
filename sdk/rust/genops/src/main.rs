//! genops writes `src/operations.rs` (gitignored): one builder per generated
//! Platform and Manager operation, grouped as in Alien's TypeScript SDK
//! (`alien.deployments().list().project("web").limit(10).send().await?`); an
//! operation without a group is a method of the client itself
//! (`alien.accept_workspace_invitation(token)`). Required arguments are the
//! method's arguments, optional ones chained setters, and `send()` calls the
//! generated async function. It also re-exports the generated types (and only
//! the types) as `alien_sdk::platform` and `alien_sdk::manager`, so the
//! generated clients, whose `Debug` prints the credential (docs/gaps.md#b22),
//! stay private to the facade.
//!
//! It also writes `tests/support/operations.rs` (gitignored): the conformance
//! harness's table of every operation (`call.op` in a scenario,
//! conformance/README.md), each calling its builder.
//!
//! It reads the generated crate, so it holds no per-operation code, and
//! takes every name from it: the crate's own (it is the facade's one path
//! dependency, whatever `package_name` in baml/baml.toml calls it), and its
//! modules, functions and arguments as BAML's `language` naming spells them. The only names it makes
//! up are type names, joined from those (`deployment_groups` +
//! `create_token` → `DeploymentGroupsCreateToken`). scripts/generate.sh runs
//! it after `baml generate`, as it runs sdk/go/internal/genops:
//!
//!     cargo run -p genops    (from sdk/rust)

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Expr, ExprLit, FnArg, GenericArgument, Item, Lit, Pat, PathArguments, ReturnType, Type,
    TypeParamBound, visit::Visit, visit_mut::VisitMut,
};

/// The facade's manifest, which names the generated crate (`Generated::find`).
const MANIFEST: &str = "Cargo.toml";
const OUTPUT: &str = "src/operations.rs";
const TABLE: &str = "tests/support/operations.rs";

/// The suffix of the generated function each `send()` calls: the async
/// binding that takes per-call options (`list` → `list_with_options`; the
/// blocking twin is `list_sync_with_options`). The only place genops names a
/// generated binding.
const ASYNC_WITH_OPTIONS: &str = "_with_options";

/// One of the two generated APIs.
struct Api {
    /// BAML namespace and generated module: `platform`.
    path: &'static str,
    /// The generated client type, which the facade keeps private.
    client: &'static str,
    /// The facade client whose methods start each call.
    facade: &'static str,
    /// The facade client's own methods, which no group or operation may shadow.
    reserved: &'static [&'static str],
}

const APIS: [Api; 2] = [
    Api {
        path: "platform",
        client: "PlatformClient",
        facade: "Alien",
        reserved: &["new", "setup"],
    },
    Api {
        path: "manager",
        client: "ManagerClient",
        facade: "AlienManager",
        reserved: &["new", "with_timeout"],
    },
];

/// The builder's own methods and fields, which no argument may take.
const BUILDER_METHODS: [&str; 2] = ["send", "baml_options"];
const BUILDER_FIELDS: [&str; 2] = ["baml_client", "baml_options"];

/// An argument of a generated operation.
struct Param {
    ident: Ident,
    /// Its BAML name, which scenarios use: `deploymentGroup`.
    wire: String,
    /// The type the builder holds: the argument's type, or for an optional
    /// argument the `T` of its `OptionalArg<T>`.
    ty: Type,
    /// For an optional argument: what its setter takes (`impl Into<…>`), the
    /// `X` of `T = Option<X>`, or `T` itself.
    setter: Option<Setter>,
}

struct Setter {
    ty: Type,
    /// `T` is `Option<X>`: the setter wraps the value in `Some`.
    nullable: bool,
    doc: Vec<String>,
}

/// The generated crate.
struct Generated {
    /// Its name, as the facade's code writes it: `alien_api`.
    name: Ident,
    /// Its sources: `alien_api/src`.
    src: PathBuf,
}

impl Generated {
    /// The facade's one path dependency, as `Cargo.toml` declares it. Its key is
    /// the name code writes, since the generated crate's lib is named as its
    /// package (BAML's `package_name`).
    fn find() -> Result<Generated, String> {
        let text = fs::read_to_string(MANIFEST).map_err(|e| format!("read {MANIFEST} (run from sdk/rust): {e}"))?;
        let manifest: toml::Table = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
        let dependencies = manifest.get("dependencies").and_then(toml::Value::as_table);
        let paths: Vec<(&String, &str)> = dependencies
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| Some((key, value.get("path")?.as_str()?)))
            .collect();
        let [(key, path)] = paths[..] else {
            return Err(format!(
                "{MANIFEST} must have exactly one path dependency, the generated crate; it has {:?}",
                paths.iter().map(|(key, _)| key).collect::<Vec<_>>()
            ));
        };
        let src = Path::new(path).join("src");
        if !src.join("lib.rs").is_file() {
            return Err(format!("no generated crate at {path} (run baml generate first)"));
        }
        Ok(Generated { name: Ident::new(&key.replace('-', "_"), Span::call_site()), src })
    }

    /// `alien_api::platform`, from `platform`.
    fn path(&self, module: impl std::fmt::Display) -> syn::Path {
        syn::parse_str(&format!("{}::{module}", self.name)).expect("a path")
    }
}

struct Operation {
    api: &'static Api,
    /// The generated module, as a path: `alien_api::platform::deployments`.
    module: syn::Path,
    /// The generated client type its function takes: `alien_api::platform::PlatformClient`.
    client: syn::Path,
    /// The generated group module, `None` for an operation without a group.
    group: Option<Ident>,
    /// The operation's generated name: `list`.
    method: Ident,
    /// The generated function `send()` calls: `list_with_options`.
    function: Ident,
    /// The BAML path without `user.`: `platform.deployments.list`.
    key: String,
    doc: Vec<String>,
    params: Vec<Param>,
    result: Type,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("genops: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let generated = Generated::find()?;
    let mut apis = Vec::new();
    for api in &APIS {
        apis.push(read_api(&generated, api)?);
    }
    let mut counts = String::new();
    for (api, items) in APIS.iter().zip(&apis) {
        let _ = write!(counts, " {} {} operations;", api.path, items.ops.len());
    }
    let header = |rest: &str| {
        format!("// Code generated by genops from {}. DO NOT EDIT.\n{rest}", generated.src.display())
    };
    write_rust(OUTPUT, &header(OUTPUT_HEADER), facade(&apis)?)?;
    let ops: Vec<&Operation> = apis.iter().flat_map(|items| &items.ops).collect();
    write_rust(TABLE, &header(TABLE_HEADER), table(&ops))?;
    eprintln!("genops: wrote {OUTPUT} and {TABLE}:{}", counts.trim_end_matches(';'));
    Ok(())
}

fn write_rust(path: &str, header: &str, tokens: TokenStream) -> Result<(), String> {
    let file: syn::File =
        syn::parse2(tokens).map_err(|error| format!("{path}: generated code does not parse: {error}"))?;
    let source = format!("{header}\n{}", prettyplease::unparse(&file));
    fs::write(path, source).map_err(|error| format!("write {path}: {error}"))
}

/// After the line `run` writes first: `// Code generated by genops from …`.
const OUTPUT_HEADER: &str = "\
// scripts/generate.sh writes it (gitignored); genops/src/main.rs explains it.
";

const TABLE_HEADER: &str = "\
// Included by tests/conformance.rs, which defines the helpers it calls.
";

// ---------------------------------------------------------------------------
// Reading the generated crate.

/// The types an API's module declares (`types`) and its group modules
/// declare (the unions in operation signatures), and its operations.
struct ApiItems {
    /// The generated module, as a path: `alien_api::platform`.
    module: syn::Path,
    types: Vec<Ident>,
    group_types: BTreeMap<String, (Ident, Vec<Ident>)>,
    ops: Vec<Operation>,
}

fn read_api(generated: &Generated, api: &'static Api) -> Result<ApiItems, String> {
    let root = generated.src.join(api.path);
    let file = parse(&root.join("mod.rs"))?;
    let mut types = Vec::new();
    let mut group_types: BTreeMap<String, (Ident, Vec<Ident>)> = BTreeMap::new();
    let mut ops = Vec::new();
    let module = generated.path(api.path);
    let client = generated.path(format_args!("{}::{}", api.path, api.client));
    for item in &file.items {
        match item {
            Item::Struct(s) if is_pub(&s.vis) => types.push(s.ident.clone()),
            Item::Enum(e) if is_pub(&e.vis) => types.push(e.ident.clone()),
            Item::Type(t) if is_pub(&t.vis) => types.push(t.ident.clone()),
            Item::Mod(m) if is_pub(&m.vis) && m.content.is_none() => {
                let path = root.join(unraw(&m.ident)).join("mod.rs");
                let group = parse(&path)?;
                let mut group_module = module.clone();
                group_module.segments.push(m.ident.clone().into());
                for item in &group.items {
                    match item {
                        Item::Fn(f) => {
                            if let Some(op) = read_operation(generated, api, &group_module, &client, Some(&m.ident), f)
                                .map_err(|e| format!("{}::{}: {e}", m.ident, f.sig.ident))?
                            {
                                ops.push(op);
                            }
                        }
                        Item::Struct(syn::ItemStruct { vis, ident, .. })
                        | Item::Enum(syn::ItemEnum { vis, ident, .. })
                        | Item::Type(syn::ItemType { vis, ident, .. })
                            if is_pub(vis) =>
                        {
                            group_types
                                .entry(m.ident.to_string())
                                .or_insert_with(|| (m.ident.clone(), Vec::new()))
                                .1
                                .push(ident.clone());
                        }
                        Item::Mod(_) => {
                            return Err(format!("{}: a group module has a module", path.display()));
                        }
                        _ => {}
                    }
                }
            }
            Item::Fn(f) => {
                if let Some(op) = read_operation(generated, api, &module, &client, None, f)
                    .map_err(|e| format!("{}::{}: {e}", api.path, f.sig.ident))?
                {
                    ops.push(op);
                }
            }
            _ => {}
        }
    }
    let Some(position) = types.iter().position(|t| t == api.client) else {
        return Err(format!("no {} in {}", api.client, root.display()));
    };
    types.remove(position);
    if ops.is_empty() {
        return Err(format!("no operations in {}", root.display()));
    }
    types.sort_by_key(|t| t.to_string());
    for (_, group) in group_types.values_mut() {
        group.sort_by_key(|t| t.to_string());
    }
    ops.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(ApiItems { module, types, group_types, ops })
}

fn parse(path: &Path) -> Result<syn::File, String> {
    let source = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    syn::parse_file(&source).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// An identifier's text without `r#`: `r#type` → `type`.
fn unraw(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("r#").map(str::to_string).unwrap_or(s)
}

/// The operation a generated function is the per-call-options async binding
/// of, or `None` for any other function (the other bindings, helpers).
fn read_operation(
    generated: &Generated,
    api: &'static Api,
    module: &syn::Path,
    api_client: &syn::Path,
    group: Option<&Ident>,
    f: &syn::ItemFn,
) -> Result<Option<Operation>, String> {
    let name = unraw(&f.sig.ident);
    let Some(method) = name.strip_suffix(ASYNC_WITH_OPTIONS) else {
        return Ok(None);
    };
    if f.sig.asyncness.is_none() || !is_pub(&f.vis) {
        return Ok(None);
    }
    let Some(fqn) = invoked(&f.block) else {
        return Err("calls no BAML function".into());
    };
    let Some(key) = fqn.strip_prefix("user.") else {
        return Err(format!("calls {fqn}, not a function of this project"));
    };
    if !key.starts_with(&format!("{}.", api.path)) {
        return Ok(None);
    }
    let inputs: Vec<&syn::PatType> = f
        .sig
        .inputs
        .iter()
        .map(|input| match input {
            FnArg::Typed(t) => Ok(t),
            FnArg::Receiver(_) => Err("takes self".to_string()),
        })
        .collect::<Result<_, _>>()?;
    let [client, args @ .., _options] = inputs.as_slice() else {
        return Err("takes neither a client nor options".into());
    };
    if !client.ty.to_token_stream().to_string().contains(api.client) {
        return Err(format!("its first argument is not a {}", api.client));
    }
    let wires = argument_names(&f.block);
    let docs = doc_lines(&f.attrs);
    let arg_docs = argument_docs(&docs);
    let mut params = Vec::new();
    for arg in args {
        let Pat::Ident(pat) = arg.pat.as_ref() else {
            return Err("an argument is not an identifier".into());
        };
        let ident = pat.ident.clone();
        let wire = wires
            .get(&ident.to_string())
            .cloned()
            .ok_or_else(|| format!("argument {ident} is not sent"))?;
        let (ty, optional) = argument_type(&arg.ty)?;
        let setter = optional.then(|| {
            let (ty, nullable) = match option_inner(&ty) {
                Some(inner) => (inner.clone(), true),
                None => (ty.clone(), false),
            };
            Setter {
                ty,
                nullable,
                doc: arg_docs.get(&wire).cloned().unwrap_or_default(),
            }
        });
        params.push(Param { ident, wire, ty: qualify(generated, ty), setter: setter.map(|s| Setter { ty: qualify(generated, s.ty), ..s }) });
    }
    let result = match &f.sig.output {
        ReturnType::Type(_, ty) => first_generic(ty).ok_or("returns no Result")?,
        ReturnType::Default => return Err("returns nothing".into()),
    };
    Ok(Some(Operation {
        api,
        module: module.clone(),
        client: api_client.clone(),
        group: group.cloned(),
        method: ident(method),
        function: f.sig.ident.clone(),
        key: key.to_string(),
        doc: operation_doc(&docs),
        params,
        result: qualify(generated, result),
    }))
}

/// The BAML function a generated binding calls: the string literal its call
/// to `invoke_with_options` starts with.
fn invoked(block: &syn::Block) -> Option<String> {
    struct Find(Option<String>);
    impl Visit<'_> for Find {
        fn visit_expr_call(&mut self, call: &syn::ExprCall) {
            if let Expr::Path(path) = call.func.as_ref()
                && path.path.segments.last().is_some_and(|s| s.ident == "invoke_with_options")
                && let Some(Expr::Lit(ExprLit { lit: Lit::Str(s), .. })) = call.args.first()
            {
                self.0.get_or_insert(s.value());
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut find = Find(None);
    find.visit_block(block);
    find.0
}

/// Each argument's BAML name, from the calls that send it:
/// `arguments.value("deploymentGroup", &deployment_group, …)` or `.optional(…)`.
fn argument_names(block: &syn::Block) -> BTreeMap<String, String> {
    struct Find(BTreeMap<String, String>);
    impl Visit<'_> for Find {
        fn visit_expr_method_call(&mut self, call: &syn::ExprMethodCall) {
            if (call.method == "value" || call.method == "optional")
                && let (Some(Expr::Lit(ExprLit { lit: Lit::Str(wire), .. })), Some(value)) =
                    (call.args.first(), call.args.iter().nth(1))
            {
                struct Idents(Vec<String>);
                impl Visit<'_> for Idents {
                    fn visit_path(&mut self, path: &syn::Path) {
                        if let Some(ident) = path.get_ident() {
                            self.0.push(ident.to_string());
                        }
                        syn::visit::visit_path(self, path);
                    }
                }
                let mut idents = Idents(Vec::new());
                idents.visit_expr(value);
                for name in idents.0 {
                    self.0.entry(name).or_insert_with(|| wire.value());
                }
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut find = Find(BTreeMap::new());
    find.visit_block(block);
    find.0
}

/// An argument's type and whether it is optional:
/// `impl Into<OptionalArg<T>>` → `(T, true)`, `impl Borrow<X>` and
/// `impl Into<X>` → `(X, false)`, any other type → itself.
fn argument_type(ty: &Type) -> Result<(Type, bool), String> {
    let Type::ImplTrait(imp) = ty else {
        return Ok((ty.clone(), false));
    };
    let [TypeParamBound::Trait(bound)] = imp.bounds.iter().collect::<Vec<_>>().as_slice() else {
        return Err(format!("unexpected argument type {}", ty.to_token_stream()));
    };
    let last = bound.path.segments.last().expect("a path");
    if last.ident != "Into" && last.ident != "Borrow" {
        return Err(format!("unexpected argument type {}", ty.to_token_stream()));
    }
    let inner = generic(&last.arguments, 0).ok_or("no type argument")?;
    if last.ident == "Into"
        && let Type::Path(p) = &inner
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "OptionalArg"
    {
        return Ok((generic(&seg.arguments, 0).ok_or("no type argument")?, true));
    }
    Ok((inner, false))
}

fn generic(args: &PathArguments, i: usize) -> Option<Type> {
    match args {
        PathArguments::AngleBracketed(a) => a
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(t) => Some(t.clone()),
                _ => None,
            })
            .nth(i),
        _ => None,
    }
}

/// `T` of `Result<T, …>`.
fn first_generic(ty: &Type) -> Option<Type> {
    match ty {
        Type::Path(p) => generic(&p.path.segments.last()?.arguments, 0),
        _ => None,
    }
}

/// `X` of `Option<X>`.
fn option_inner(ty: &Type) -> Option<Type> {
    match ty {
        Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option") => {
            generic(&p.path.segments.last()?.arguments, 0)
        }
        _ => None,
    }
}

/// A generated type as the facade names it: `crate::…` → `alien_api::…`.
fn qualify(generated: &Generated, mut ty: Type) -> Type {
    struct Qualify<'a>(&'a Ident);
    impl VisitMut for Qualify<'_> {
        fn visit_path_mut(&mut self, path: &mut syn::Path) {
            if path.segments.first().is_some_and(|s| s.ident == "crate") {
                path.segments[0].ident = self.0.clone();
                path.leading_colon = None;
            }
            syn::visit_mut::visit_path_mut(self, path);
        }
    }
    Qualify(&generated.name).visit_type_mut(&mut ty);
    ty
}

/// The text of `#[doc = "…"]` attributes, one line each, without the space
/// `///` adds.
fn doc_lines(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(nv) if nv.path.is_ident("doc") => match &nv.value {
                Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) => {
                    let line = s.value();
                    Some(line.strip_prefix(' ').map(str::to_string).unwrap_or(line))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The operation's doc without the generator's notes, which describe the
/// generated function (by-value arguments, its `# Errors` section).
fn operation_doc(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in lines {
        if line.starts_with("# Errors") {
            break;
        }
        if line.starts_with("Arguments are passed to the BAML runtime by value") {
            skipping = true;
        }
        if skipping {
            skipping = !line.is_empty();
            continue;
        }
        out.push(line.clone());
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Each argument's description, by BAML name, from the doc's argument list
/// (`` - `limit`: Maximum number… ``, continued on indented lines).
fn argument_docs(lines: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in lines {
        if let Some(rest) = line.strip_prefix("- `")
            && let Some((name, text)) = rest.split_once("`:")
        {
            current = Some(name.to_string());
            out.insert(name.to_string(), vec![text.trim().to_string()]);
        } else if let Some(name) = &current
            && (line.starts_with("  ") || line.is_empty())
        {
            out.get_mut(name).expect("current").push(line.trim().to_string());
        } else {
            current = None;
        }
    }
    for doc in out.values_mut() {
        while doc.last().is_some_and(|l| l.is_empty()) {
            doc.pop();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Names.

fn ident(name: &str) -> Ident {
    syn::parse_str::<Ident>(name).unwrap_or_else(|_| Ident::new_raw(name, Span::call_site()))
}

/// A generated snake_case name joined into a type name: `deployment_groups`
/// → `DeploymentGroups`.
fn pascal(ident: &Ident) -> String {
    unraw(ident)
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect()
}

impl Operation {
    /// The builder type: `DeploymentsList`, or `AcceptWorkspaceInvitation`
    /// without a group.
    fn builder(&self) -> Ident {
        let group = self.group.as_ref().map(pascal).unwrap_or_default();
        format_ident!("{group}{}", pascal(&self.method))
    }

    /// The call as Rust code writes it, for docs: `alien.deployments().list()`.
    fn call(&self) -> String {
        let receiver = self.api.facade.to_lowercase().replace("alienmanager", "manager");
        match &self.group {
            Some(group) => format!("{receiver}.{}().{}()", unraw(group), unraw(&self.method)),
            None => format!("{receiver}.{}()", unraw(&self.method)),
        }
    }

    fn required(&self) -> impl Iterator<Item = &Param> {
        self.params.iter().filter(|p| p.setter.is_none())
    }

    fn optional(&self) -> impl Iterator<Item = (&Param, &Setter)> {
        self.params.iter().filter_map(|p| p.setter.as_ref().map(|s| (p, s)))
    }
}

/// Records the names one Rust namespace declares, to catch two that clash.
#[derive(Default)]
struct Names(BTreeMap<String, String>);

impl Names {
    fn claim(&mut self, name: impl ToString, what: impl ToString) -> Result<(), String> {
        let (name, what) = (name.to_string(), what.to_string());
        if let Some(prev) = self.0.insert(name.clone(), what.clone()) {
            return Err(format!("{prev} and {what} are both named {name}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// src/operations.rs.

fn docs(lines: &[String]) -> TokenStream {
    let lines = lines.iter().map(|l| format!(" {l}"));
    quote! { #(#[doc = #lines])* }
}

fn facade(apis: &[ApiItems]) -> Result<TokenStream, String> {
    let mut modules = Vec::new();
    let mut type_modules = Vec::new();
    for (api, api_items) in APIS.iter().zip(apis) {
        let (module, types, ops) = (&api_items.module, &api_items.types, &api_items.ops);
        let path = format_ident!("{}", api.path);
        let client = format_ident!("{}", api.client);
        let facade = format_ident!("{}", api.facade);

        // The facade client's methods, and the types this module declares.
        let mut methods = Names::default();
        for name in api.reserved {
            methods.claim(name, format!("{}::{name}", api.facade))?;
        }
        let mut type_names = Names::default();
        let mut groups: BTreeMap<String, (Ident, Vec<&Operation>)> = BTreeMap::new();
        let mut ungrouped = Vec::new();
        for op in ops {
            match &op.group {
                Some(g) => groups.entry(g.to_string()).or_insert_with(|| (g.clone(), Vec::new())).1.push(op),
                None => ungrouped.push(op),
            }
        }

        let mut items = Vec::new();
        let mut facade_methods = Vec::new();
        for (group, group_ops) in groups.values() {
            methods.claim(unraw(group), format!("group {}.{group}", api.path))?;
            let service = format_ident!("{}", pascal(group));
            type_names.claim(&service, format!("group {}.{group}", api.path))?;
            let doc = format!(
                " The {} API's `{}` operations: `{}.{}()`.",
                pascal(&format_ident!("{}", api.path)),
                group_ops[0].key.split('.').nth(1).expect("a group"),
                api.facade.to_lowercase().replace("alienmanager", "manager"),
                unraw(group),
            );
            let group_doc = format!(" The `{}` operations.", group_ops[0].key.split('.').nth(1).expect("a group"));
            facade_methods.push(quote! {
                #[doc = #group_doc]
                pub fn #group(&self) -> #service<'_> {
                    #service { client: &self.client }
                }
            });
            let mut group_methods = Names::default();
            let mut starts = Vec::new();
            for op in group_ops {
                group_methods.claim(unraw(&op.method), &op.key)?;
                let (start, builder) = builder(op, quote! { self.client }, &mut type_names)?;
                starts.push(start);
                items.push(builder);
            }
            let name = service.to_string();
            items.push(quote! {
                #[doc = #doc]
                #[derive(Clone, Copy)]
                pub struct #service<'a> {
                    client: &'a #module::#client,
                }

                impl<'a> #service<'a> {
                    #(#starts)*
                }

                impl ::std::fmt::Debug for #service<'_> {
                    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                        f.debug_struct(#name).finish_non_exhaustive()
                    }
                }
            });
        }
        for op in &ungrouped {
            methods.claim(unraw(&op.method), &op.key)?;
            let (start, builder) = builder(op, quote! { &self.client }, &mut type_names)?;
            facade_methods.push(start);
            items.push(builder);
        }

        let module_doc = format!(
            " The {} API's operations: the groups and builders that [`{}`](crate::{}) methods return.",
            pascal(&path),
            api.facade,
            api.facade,
        );
        modules.push(quote! {
            #[doc = #module_doc]
            pub mod #path {
                use crate::{BamlOptions, Error};


                impl crate::#facade {
                    #(#facade_methods)*
                }

                #(#items)*
            }
        });

        let types_doc = format!(
            " Every {} API request and response type, as generated. Call operations through [`{}`](crate::{}).",
            pascal(&path),
            api.facade,
            api.facade,
        );
        let group_types = api_items.group_types.values().map(|(group, types)| {
            let doc = format!(" Types the `{}` operations declare.", unraw(group));
            quote! {
                #[doc = #doc]
                pub mod #group {
                    pub use #module::#group::{#(#types),*};
                }
            }
        });
        type_modules.push(quote! {
            #[doc = #types_doc]
            pub mod #path {
                pub use #module::{#(#types),*};
                #(#group_types)*
            }
        });
    }
    Ok(quote! {
        #(#modules)*

        /// The generated types, re-exported by the crate as `platform` and `manager`.
        pub(crate) mod types {
            #(#type_modules)*
        }
    })
}

/// An operation's method on its group or facade client (`start`), and its
/// builder type with its setters and `send`.
fn builder(
    op: &Operation,
    client: TokenStream,
    type_names: &mut Names,
) -> Result<(TokenStream, TokenStream), String> {
    let name = op.builder();
    type_names.claim(&name, &op.key)?;
    let method = &op.method;
    let module = &op.module;
    let function = &op.function;
    let api_client = &op.client;
    let result = &op.result;

    let mut members = Names::default();
    for m in BUILDER_METHODS {
        members.claim(m, format!("{name}::{m}"))?;
    }
    for f in BUILDER_FIELDS {
        if op.params.iter().any(|p| p.ident == f) {
            return Err(format!("{}: an argument is named {f}, a builder field", op.key));
        }
    }
    for (p, _) in op.optional() {
        members.claim(unraw(&p.ident), format!("{} argument {}", op.key, p.wire))?;
    }

    let required: Vec<&Param> = op.required().collect();
    let req_idents: Vec<&Ident> = required.iter().map(|p| &p.ident).collect();
    let req_types: Vec<&Type> = required.iter().map(|p| &p.ty).collect();
    let optional: Vec<(&Param, &Setter)> = op.optional().collect();
    let opt_idents: Vec<&Ident> = optional.iter().map(|(p, _)| &p.ident).collect();
    let opt_types: Vec<&Type> = optional.iter().map(|(p, _)| &p.ty).collect();
    let args: Vec<TokenStream> = op
        .params
        .iter()
        .map(|p| {
            let ident = &p.ident;
            quote! { self.#ident }
        })
        .collect();

    let setters = optional.iter().map(|(p, s)| {
        let ident = &p.ident;
        let ty = &s.ty;
        let value = if s.nullable {
            quote! { ::std::option::Option::Some(::std::convert::Into::into(#ident)) }
        } else {
            quote! { ::std::convert::Into::into(#ident) }
        };
        let doc = if s.doc.is_empty() {
            vec![format!("Sets `{}`.", p.wire)]
        } else {
            s.doc.clone()
        };
        let doc = docs(&doc);
        quote! {
            #doc
            pub fn #ident(mut self, #ident: impl ::std::convert::Into<#ty>) -> Self {
                self.#ident = ::baml_bridge::OptionalArg::Set(#value);
                self
            }
        }
    });

    let mut start_doc = op.doc.clone();
    start_doc.push(String::new());
    start_doc.push(if optional.is_empty() {
        format!("Returns a [`{name}`]: call `.send().await`.")
    } else {
        format!("Returns a [`{name}`]: set optional arguments on it, then call `.send().await`.")
    });
    let start_doc = docs(&start_doc);
    let start = quote! {
        #start_doc
        pub fn #method(&self, #(#req_idents: impl ::std::convert::Into<#req_types>),*) -> #name<'_> {
            #name {
                baml_client: #client,
                #(#req_idents: ::std::convert::Into::into(#req_idents),)*
                #(#opt_idents: ::baml_bridge::OptionalArg::Unset,)*
                baml_options: ::std::option::Option::None,
            }
        }
    };
    // A group method returns a builder borrowing the client, not the group.
    let start = if op.group.is_some() {
        syn::parse2::<syn::ImplItemFn>(start)
            .map(|mut f| {
                f.sig.output = syn::parse_quote! { -> #name<'a> };
                f.to_token_stream()
            })
            .map_err(|e| e.to_string())?
    } else {
        start
    };

    let struct_doc = format!(
        " `{}`: the {} operation. Its `Debug` output shows no argument and no credential.",
        op.call(),
        op.key,
    );
    let debug_name = name.to_string();
    let builder = quote! {
        #[doc = #struct_doc]
        #[must_use = "the request is sent by `.send().await`"]
        pub struct #name<'a> {
            baml_client: &'a #api_client,
            #(#req_idents: #req_types,)*
            #(#opt_idents: ::baml_bridge::OptionalArg<#opt_types>,)*
            baml_options: ::std::option::Option<BamlOptions>,
        }

        impl #name<'_> {
            #(#setters)*

            /// Per-call options, for example a timeout for the whole call:
            /// `.baml_options(BamlOptions::new().timeout_ms(10_000))`.
            pub fn baml_options(mut self, options: BamlOptions) -> Self {
                self.baml_options = ::std::option::Option::Some(options);
                self
            }

            /// Makes the call.
            pub async fn send(self) -> ::std::result::Result<#result, Error> {
                ::std::result::Result::Ok(#module::#function(
                    self.baml_client,
                    #(#args,)*
                    self.baml_options.unwrap_or_default(),
                ).await?)
            }
        }

        impl ::std::fmt::Debug for #name<'_> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(#debug_name).finish_non_exhaustive()
            }
        }
    };
    Ok((start, builder))
}

// ---------------------------------------------------------------------------
// tests/support/operations.rs.

fn table(ops: &[&Operation]) -> TokenStream {
    let keys: Vec<&str> = ops.iter().map(|op| op.key.as_str()).collect();
    let rows = ops.iter().map(|op| {
        let key = &op.key;
        let client = match op.api.path {
            "platform" => quote! { alien(&a["client"]) },
            _ => quote! { alien_manager(&a["client"]) },
        };
        let target = match &op.group {
            Some(group) => quote! { client.#group() },
            None => quote! { client },
        };
        let method = &op.method;
        let required = op.required().map(|p| {
            let (ty, wire) = (&p.ty, &p.wire);
            quote! { arg::<#ty>(a, #wire).await? }
        });
        let optional = op.optional().map(|(p, s)| {
            let (ty, wire, ident) = (&s.ty, &p.wire, &p.ident);
            quote! {
                if let ::std::option::Option::Some(value) = opt::<#ty>(a, #wire).await? {
                    builder = builder.#ident(value);
                }
            }
        });
        quote! {
            #key => Box::pin(async move {
                let client = #client;
                #[allow(unused_mut)]
                let mut builder = #target.#method(#(#required),*);
                #(#optional)*
                Ok(to_json(&builder.send().await?))
            }).await,
        }
    });
    quote! {
        /// Every generated operation, by the BAML path a scenario's `call.op` names.
        const OPERATIONS: &[&str] = &[#(#keys),*];

        /// Calls an operation through its builder with the scenario's arguments.
        /// Each row's future is boxed: one future holding all of them
        /// overflows the test thread's stack.
        async fn operation(op: &str, a: &Value) -> Result<Value, Error> {
            match op {
                #(#rows)*
                other => Err(harness(format!("no Rust binding for operation {other}"))),
            }
        }
    }
}
