//! Imports OpenAPI 3.0/3.1 specifications into BAML. See DESIGN.md.

pub mod config;
pub mod coverage;
pub mod dedupe;
pub mod emit;
pub mod lower;
pub mod naming;
pub mod ops;
pub mod spec;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
};

use config::{Config, Selection};
use coverage::Coverage;
use lower::{Def, Lowerer};
use ops::{Operation, Response};

#[derive(Debug)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Every file the importer owns, by repository-relative path.
pub struct Output {
    pub files: BTreeMap<String, String>,
    /// Directories whose `.baml` files are all generated.
    pub owned_dirs: Vec<String>,
}

pub struct ServiceReport {
    pub name: String,
    pub spec: String,
    pub digest: String,
    pub operations: usize,
    pub total_operations: usize,
    pub classes: usize,
    pub aliases: usize,
    pub coverage: Coverage,
}

pub fn run(root: &Path, config: &Config) -> Result<Output, Error> {
    let mut files = BTreeMap::new();
    let mut reports = Vec::new();
    let mut owned_dirs = Vec::new();
    for service in &config.services {
        let spec = spec::Spec::load(root, &service.spec, &service.overlays)?;
        lower::check_policy(&spec, config.open_enums)?;
        let mut coverage = Coverage::default();
        let all = ops::all_operations(&spec)?;
        let selected: Vec<_> = match &service.operations {
            Selection::Keyword(k) if k == "all" => all.iter().collect(),
            Selection::Keyword(k) => return Err(Error::new(format!("{}: operations must be \"all\" or a list, got {k:?}", service.name))),
            Selection::Only(ids) => {
                let mut picked = Vec::new();
                for id in ids {
                    let op = all
                        .iter()
                        .find(|(op_id, ..)| op_id == id)
                        .ok_or_else(|| Error::new(format!("{}: no operation `{id}` in {}", service.name, service.spec)))?;
                    picked.push(op);
                }
                picked
            }
        };
        let mut lowerer = Lowerer::new(&spec, config.open_enums, &mut coverage);
        let mut operations = Vec::new();
        for op in &selected {
            operations.push(ops::lower_operation(&mut lowerer, service, &spec, op)?);
        }
        ops::check_names(&operations)?;
        lowerer.widen_untagged_literals(&mut operations);
        if config.open_unions {
            lowerer.open_tagged_unions(&mut operations)?;
        }
        drop_unreachable(&mut lowerer.defs, &operations);
        let merged = dedupe::dedupe(&mut lowerer.defs, &mut operations);
        lowerer.coverage.counts.insert("identical classes merged away".into(), merged);
        operations.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
        let defs: Vec<&Def> = lowerer.defs.values().collect();
        let classes = defs.iter().filter(|d| matches!(d, Def::Class(_))).count();
        let aliases = defs.len() - classes;
        let index = emit::Index::new(&defs);
        let types_text = emit::types(&defs, &index);
        let mut groups: BTreeMap<Option<String>, String> = BTreeMap::new();
        for op in &operations {
            groups.entry(op.group.clone()).or_default().push_str(&emit::operation(&service.name, service.client, op, &index)?);
        }
        drop(index);
        drop(defs);
        drop(lowerer);

        let head = emit::banner(&service.spec, &spec.digest, &spec.overlays);
        let out = service.out.trim_end_matches('/').to_string();
        owned_dirs.push(out.clone());
        files.insert(format!("{out}/client.baml"), format!("{head}\n{}", emit::client(service.client)));
        files.insert(format!("{out}/types.baml"), format!("{head}{types_text}"));
        for (group, text) in groups {
            let path = match &group {
                None => format!("{out}/operations.baml"),
                Some(g) => format!("{out}/ns_{g}/operations.baml"),
            };
            files.insert(path, format!("{head}{text}"));
        }
        reports.push(ServiceReport {
            name: service.name.clone(),
            spec: service.spec.clone(),
            digest: spec.digest.clone(),
            operations: operations.len(),
            total_operations: all.len(),
            classes,
            aliases,
            coverage,
        });
    }
    files.insert(config.coverage.clone(), coverage::render(&reports));
    Ok(Output { files, owned_dirs })
}

/// Removes the classes and aliases that no operation's arguments or response
/// reach, such as the members of a union typed `json`.
fn drop_unreachable(defs: &mut BTreeMap<String, Def>, operations: &[Operation]) {
    let mut pending = BTreeSet::new();
    for op in operations {
        op.args.iter().for_each(|a| a.ty.names(&mut pending));
        if let Response::Json(ty) | Response::OptionalJson(ty) = &op.response {
            ty.names(&mut pending);
        }
    }
    let mut reached = BTreeSet::new();
    while let Some(name) = pending.pop_first() {
        if !reached.insert(name.clone()) {
            continue;
        }
        match defs.get(&name) {
            Some(Def::Class(class)) => {
                class.fields.iter().for_each(|f| f.ty.names(&mut pending));
                class.extra.iter().for_each(|extra| extra.names(&mut pending));
            }
            Some(Def::Alias { ty, .. }) => ty.names(&mut pending),
            None => {}
        }
    }
    defs.retain(|name, _| reached.contains(name));
}
