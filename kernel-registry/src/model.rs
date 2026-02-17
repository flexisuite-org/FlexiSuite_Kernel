use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Composition,
    Component,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DistManifest {
    pub schema_version: String,
    pub id: String,
    pub version: String, // Added this field
    pub kind: Kind,
    pub name: String,
    #[serde(default)]
    pub protected: bool,
    pub composition_root: String,
    pub routes: Vec<Route>,
    pub dependencies: Dependencies,
    #[serde(default)]
    pub configuration: HashMap<String, serde_json::Value>,
    pub security: Security,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub path: String,
    pub component: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dependencies {
    pub components: HashMap<String, ComponentDependency>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDependency {
    pub source: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Security {
    /// SHA-384 digest of the manifest payload with the entire `security`
    /// section excluded from the hashed input.
    pub manifest_digest: String,
    pub manifest_signature: String,
    pub manifest_signature_kid: String,
    pub trust_root_version: String,
}
