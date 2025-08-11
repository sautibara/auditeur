use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// # Examples
///
///  ```
/// # fn main() -> Result<(), ron::de::SpannedError> { let file = r#"
/// (
///   plugin: (
///     name: "sautibara_rpc",
///     version: "0.0.1",
///   ),
///   dependencies: {
///     "sautibara_core": "0.0.1"
///   }
/// )
/// # "#; ron::de::from_str::<auditeur_plugin_host::config::Plugin>(file).map(drop) }
///  ```
///
///  ```
/// # fn main() -> Result<(), ron::de::SpannedError> { let file = r#"
/// (
///   workspace: [
///     "sautibara-rpc",
///     "sautibara-media-keys",
///   ]
/// )
/// # "#; ron::de::from_str::<auditeur_plugin_host::config::Plugin>(file).map(drop) }
///  ```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum Config {
    Workspace(Workspace),
    Single(Single),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    #[serde(rename = "workspace")]
    pub paths: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Single {
    pub plugin: PluginInfo,
    #[serde(default)]
    pub dependencies: HashMap<PluginName, Dependency>,
}

impl Single {
    #[must_use]
    pub const fn name(&self) -> &PluginName {
        &self.plugin.name
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct PluginInfo {
    pub name: PluginName,
    pub version: semver::Version,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(from = "DependencySerialized", into = "DependencySerialized")]
pub struct Dependency {
    pub version: semver::VersionReq,
    pub source: Option<Source>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
#[serde(
    expecting = r#"expected dependency options in the form of "<version>" or { version = "<version", ... } "#
)]
enum DependencySerialized {
    VersionOnly(semver::VersionReq),
    Full {
        version: semver::VersionReq,
        #[serde(default, skip_serializing_if = "is_default")]
        source: Option<Source>,
    },
}

impl From<Dependency> for DependencySerialized {
    fn from(value: Dependency) -> Self {
        if value.source.is_some() {
            Self::Full {
                version: value.version,
                source: value.source,
            }
        } else {
            Self::VersionOnly(value.version)
        }
    }
}

impl From<DependencySerialized> for Dependency {
    fn from(value: DependencySerialized) -> Self {
        match value {
            DependencySerialized::VersionOnly(version) => Self {
                version,
                source: None,
            },
            DependencySerialized::Full { version, source } => Self { version, source },
        }
    }
}

fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}
