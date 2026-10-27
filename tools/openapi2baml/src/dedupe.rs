//! Merging structurally identical classes.
//!
//! Alien's Platform spec inlines the same object shapes under many paths, so
//! naive lowering produces many identical classes under different names (Alien's
//! Rust filter deduplicates them for the same reason). Classes whose fields
//! (names, wire names, types, required flags) are identical are merged until
//! nothing changes, so shapes nested in identical shapes merge too. The
//! surviving name is a pinned class's (a spec component, or an operation's own
//! request body or response), else any class's; among those, the one whose
//! schema is under `/components/`, then the shallowest, then the first by JSON
//! pointer. The key belongs to the schema, not to the operation selection or
//! the order of lowering: importing more operations renames a merged class
//! only when they bring an identical schema that sorts first.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    lower::{Def, Ty},
    ops::{Operation, Response},
};

fn signature(class: &crate::lower::Class) -> String {
    let fields = &class.fields;
    let mut out = String::from(if class.renamed { "renamed;" } else { ";" });
    if let Some(extra) = &class.extra {
        out.push_str(&format!("extra={};", extra.render(&|n| n.to_string())));
    }
    if let Some((wires, known)) = &class.open_tag {
        out.push_str(&format!("open={}:{};", wires.join("+"), known.join(",")));
    }
    for f in fields {
        out.push_str(&format!("{}|{}|{}|{};", f.name, f.wire, f.required, f.ty.render(&|n| n.to_string())));
    }
    out
}

fn rewrite(ty: &mut Ty, renames: &BTreeMap<String, String>) {
    match ty {
        Ty::Named(name) => {
            if let Some(to) = renames.get(name) {
                *name = to.clone();
            }
        }
        Ty::List(inner) | Ty::Map(inner) | Ty::Optional(inner) => rewrite(inner, renames),
        Ty::Union(members) => {
            members.iter_mut().for_each(|m| rewrite(m, renames));
            // Merging can make two members identical.
            let mut seen = Vec::new();
            members.retain(|m| if seen.contains(m) { false } else { seen.push(m.clone()); true });
            if members.len() == 1 {
                *ty = members.pop().expect("one member");
            }
        }
        _ => {}
    }
}

/// Returns how many classes were merged away.
pub fn dedupe(defs: &mut BTreeMap<String, Def>, ops: &mut [Operation]) -> usize {
    let mut merged = 0;
    loop {
        let mut groups: BTreeMap<String, Vec<&crate::lower::Class>> = BTreeMap::new();
        for def in defs.values() {
            if let Def::Class(c) = def {
                groups.entry(signature(c)).or_default().push(c);
            }
        }
        let mut renames = BTreeMap::new();
        for classes in groups.values().filter(|c| c.len() > 1) {
            let canonical = classes
                .iter()
                .min_by_key(|c| (!c.pinned, !c.origin.starts_with("/components/"), c.origin.matches('/').count(), &c.origin))
                .map(|c| c.name.clone())
                .expect("non-empty group");
            for class in classes {
                if !class.pinned && class.name != canonical {
                    renames.insert(class.name.clone(), canonical.clone());
                }
            }
        }
        if renames.is_empty() {
            return merged;
        }
        let mut encoded = BTreeSet::new();
        for from in renames.keys() {
            if let Some(Def::Class(c)) = defs.remove(from)
                && c.encoded
            {
                encoded.insert(renames[from].clone());
            }
        }
        for def in defs.values_mut() {
            match def {
                Def::Class(c) => {
                    if encoded.contains(&c.name) {
                        c.encoded = true;
                    }
                    c.fields.iter_mut().for_each(|f| rewrite(&mut f.ty, &renames));
                    if let Some(extra) = &mut c.extra {
                        rewrite(extra, &renames);
                    }
                }
                Def::Alias { ty, .. } => rewrite(ty, &renames),
            }
        }
        for op in ops.iter_mut() {
            op.args.iter_mut().for_each(|a| rewrite(&mut a.ty, &renames));
            match &mut op.response {
                Response::Json(ty) | Response::OptionalJson(ty) => rewrite(ty, &renames),
                Response::Empty => {}
            }
        }
        merged += renames.len();
    }
}
