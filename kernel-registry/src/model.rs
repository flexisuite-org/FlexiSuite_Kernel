use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DistManifest {
    pub schema_version: String,
    pub id: String,
    pub version: String, // Added this field
    pub kind: String, // "composition" | "component"
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
pub struct Route {
    pub path: String,
    pub component: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Dependencies {
    pub components: HashMap<String, ComponentDependency>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ComponentDependency {
    pub source: String,
    pub version: String,
    pub digest: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Security {
    pub manifest_digest: String,
    pub manifest_signature: String,
    pub manifest_signature_kid: String,
    pub trust_root_version: String,
}
