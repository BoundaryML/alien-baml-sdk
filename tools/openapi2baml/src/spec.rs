//! Loading a spec, merging compatibility overlays, and resolving `$ref`s.

use std::path::Path;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::Error;

pub struct Spec {
    pub doc: Value,
    /// `sha256:<hex>` of the spec file as read, for the output banner.
    pub digest: String,
    /// (path, digest) of each overlay merged in, for the output banner.
    pub overlays: Vec<(String, String)>,
}

impl Spec {
    pub fn load(root: &Path, path: &str, overlays: &[String]) -> Result<Spec, Error> {
        let bytes = std::fs::read(root.join(path)).map_err(|e| Error::new(format!("read {path}: {e}")))?;
        let digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let mut doc: Value =
            serde_json::from_slice(&bytes).map_err(|e| Error::new(format!("parse {path}: {e}")))?;
        let mut merged = Vec::new();
        for overlay in overlays {
            let text = std::fs::read(root.join(overlay)).map_err(|e| Error::new(format!("read {overlay}: {e}")))?;
            let overlay_doc: Value =
                serde_json::from_slice(&text).map_err(|e| Error::new(format!("parse {overlay}: {e}")))?;
            apply_schema_overlay(&mut doc, &overlay_doc, overlay)?;
            merged.push((overlay.clone(), format!("sha256:{:x}", Sha256::digest(&text))));
        }
        Ok(Spec { doc, digest, overlays: merged })
    }

    pub fn schemas(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.doc
            .pointer("/components/schemas")
            .and_then(Value::as_object)
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    /// Follows `$ref`s of any component kind (parameters, request bodies,
    /// responses) until a non-reference object.
    pub fn deref<'a>(&'a self, mut value: &'a Value) -> Result<&'a Value, Error> {
        for _ in 0..32 {
            match value.get("$ref").and_then(Value::as_str) {
                Some(reference) => value = self.pointer(reference)?,
                None => return Ok(value),
            }
        }
        Err(Error::new("$ref chain longer than 32 links"))
    }

    pub fn pointer(&self, reference: &str) -> Result<&Value, Error> {
        let pointer = reference
            .strip_prefix('#')
            .ok_or_else(|| Error::new(format!("only local $refs are supported: {reference}")))?;
        self.doc
            .pointer(pointer)
            .ok_or_else(|| Error::new(format!("unresolvable $ref {reference}")))
    }
}

/// The component name of a `#/components/schemas/<name>` reference.
pub fn schema_ref_name(reference: &str) -> Option<String> {
    reference
        .strip_prefix("#/components/schemas/")
        .map(|name| name.replace("~1", "/").replace("~0", "~"))
}

/// Applies an OpenAPI overlay of `update` actions (Alien's
/// `sdk-compatibility-overlay.json`). Updates are merged in and only add what
/// is absent: the overlay re-adds schemas, properties and paths the server API
/// removed. A value that differs from what the spec has is an error.
fn apply_schema_overlay(doc: &mut Value, overlay: &Value, name: &str) -> Result<(), Error> {
    let actions = overlay
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::new(format!("{name}: overlay without actions")))?;
    for action in actions {
        let target = action.get("target").and_then(Value::as_str).unwrap_or_default();
        let update = action
            .get("update")
            .ok_or_else(|| Error::new(format!("{name}: only `update` actions are supported ({target})")))?;
        let mut node = &mut *doc;
        for segment in bracket_path(target).ok_or_else(|| Error::new(format!("{name}: unsupported target {target}")))? {
            node = node
                .get_mut(&segment)
                .ok_or_else(|| Error::new(format!("{name}: target {target} does not exist")))?;
        }
        add_absent(node, update, target).map_err(|at| Error::new(format!("{name}: {target} conflicts with the spec at {at}")))?;
    }
    Ok(())
}

/// `$['paths']['/v1/x']` → `["paths", "/v1/x"]`.
fn bracket_path(target: &str) -> Option<Vec<String>> {
    let mut rest = target.strip_prefix('$')?;
    let mut segments = Vec::new();
    while !rest.is_empty() {
        let inner = rest.strip_prefix("['")?;
        let end = inner.find("']")?;
        segments.push(inner[..end].to_string());
        rest = &inner[end + 2..];
    }
    Some(segments)
}

/// Adds the keys of `update` that `target` lacks, recursively; returns where a
/// present value differs.
fn add_absent(target: &mut Value, update: &Value, at: &str) -> Result<(), String> {
    if let (Value::Object(target), Value::Object(update)) = (&mut *target, update) {
        for (key, value) in update {
            match target.get_mut(key) {
                Some(existing) => add_absent(existing, value, &format!("{at}['{key}']"))?,
                None => {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        return Ok(());
    }
    if target == update { Ok(()) } else { Err(at.to_string()) }
}
