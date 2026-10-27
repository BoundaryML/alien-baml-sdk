//! The importer's config file (`tools/openapi2baml/config.json`).

use serde::Deserialize;

use crate::lower::EnumPolicy;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub open_enums: EnumPolicy,
    /// Give every tagged union in a response an `Other` member for members a
    /// newer server adds (DESIGN.md, "Open unions").
    #[serde(default)]
    pub open_unions: bool,
    /// Where the coverage report goes, repository-relative.
    pub coverage: String,
    pub services: Vec<ServiceConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// BAML namespace (`ns_<name>`).
    pub name: String,
    pub spec: String,
    #[serde(default)]
    pub overlays: Vec<String>,
    /// `"all"` or a list of operation IDs.
    pub operations: Selection,
    /// Output directory, repository-relative. The importer owns it entirely.
    pub out: String,
    pub client: ClientKind,
    pub naming: Naming,
    /// The `workspace` query parameter is a client-level global.
    #[serde(default)]
    pub workspace_global: bool,
    #[serde(default)]
    pub method_renames: Vec<MethodRename>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Selection {
    Keyword(String),
    Only(Vec<String>),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClientKind {
    Platform,
    Manager,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Naming {
    /// `x-speakeasy-group` / `x-speakeasy-name-override` (Platform).
    SpeakeasyGroup,
    /// First tag / camelCase(operationId) (Manager).
    FirstTag,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodRename {
    pub method: String,
    pub path: String,
    pub name: String,
}
