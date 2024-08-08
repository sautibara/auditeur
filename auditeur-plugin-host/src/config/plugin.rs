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
    pub dependencies: HashMap<PluginName, semver::VersionReq>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct PluginInfo {
    pub name: PluginName,
    pub version: semver::Version,
}
