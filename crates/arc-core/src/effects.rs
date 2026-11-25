//! Effect manifests as data (docs/02 §3.4, docs/09 §1).
//!
//! `arc-core` validates effect instances against manifests but owns no shaders. The renderer
//! (`arc-render`) provides the built-in manifests through [`EffectCatalog`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Float,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub key: String,
    #[serde(rename = "type")]
    pub ty: ParamType,
    pub range: [f64; 2],
    pub default: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectManifest {
    pub id: String,
    pub version: u32,
    pub display: String,
    #[serde(default)]
    pub category: Vec<String>,
    pub params: Vec<ParamSpec>,
}

impl EffectManifest {
    pub fn param(&self, key: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.key == key)
    }
}

/// Source of effect manifests for validation and discovery.
pub trait EffectCatalog: Send + Sync + std::fmt::Debug {
    fn manifest(&self, id: &str) -> Option<&EffectManifest>;
    fn all(&self) -> Vec<&EffectManifest>;
}

/// A catalog backed by a map; used by the renderer's built-ins and by tests.
#[derive(Debug, Clone, Default)]
pub struct StaticCatalog {
    manifests: BTreeMap<String, EffectManifest>,
}

impl StaticCatalog {
    pub fn new(manifests: impl IntoIterator<Item = EffectManifest>) -> Self {
        StaticCatalog {
            manifests: manifests.into_iter().map(|m| (m.id.clone(), m)).collect(),
        }
    }
}

impl EffectCatalog for StaticCatalog {
    fn manifest(&self, id: &str) -> Option<&EffectManifest> {
        self.manifests.get(id)
    }
    fn all(&self) -> Vec<&EffectManifest> {
        self.manifests.values().collect()
    }
}
