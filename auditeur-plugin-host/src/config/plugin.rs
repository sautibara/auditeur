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
#[serde(try_from = "Fields", into = "Fields")]
pub struct Config {
    pub offset: PathBuf,
    pub children: Vec<Child>,
    pub plugin: Option<Plugin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub info: PluginInfo,
    pub export: Option<Export>,
    pub dependencies: HashMap<PluginName, Dependency>,
}

impl Plugin {
    #[must_use]
    pub const fn name(&self) -> &PluginName {
        &self.info.name
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    #[serde(rename = "path", default, skip_serializing_if = "is_default")]
    pub offset: PathBuf,
    #[serde(rename = "workspace", default, skip_serializing_if = "is_default")]
    pub children: Vec<Child>,
    #[serde(rename = "plugin")]
    pub plugin_info: Option<PluginInfo>,
    pub export: Option<Export>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub dependencies: HashMap<PluginName, Dependency>,
}

impl From<Config> for Fields {
    fn from(value: Config) -> Self {
        let Config {
            offset,
            children,
            plugin,
        } = value;

        let (plugin_info, export, dependencies) = plugin.map_or_else(Default::default, |plugin| {
            (Some(plugin.info), plugin.export, plugin.dependencies)
        });

        Self {
            offset,
            children,
            plugin_info,
            export,
            dependencies,
        }
    }
}

#[derive(Error, Debug)]
pub enum ConfigTryFromFieldsError {
    #[error("expected key `plugin` to be present, since `export` is specified")]
    ExportWithoutPluginInfo,
    #[error("expected key `plugin` to be present, since `dependencies` is specified")]
    DependenciesWithoutPluginInfo,
}

impl TryFrom<Fields> for Config {
    type Error = ConfigTryFromFieldsError;
    fn try_from(value: Fields) -> Result<Self, Self::Error> {
        let Fields {
            offset,
            children,
            plugin_info,
            export,
            dependencies,
        } = value;

        let plugin = if let Some(info) = plugin_info {
            Some(Plugin {
                info,
                export,
                dependencies,
            })
        } else if export.is_some() {
            return Err(ConfigTryFromFieldsError::ExportWithoutPluginInfo);
        } else if !dependencies.is_empty() {
            return Err(ConfigTryFromFieldsError::DependenciesWithoutPluginInfo);
        } else {
            None
        };

        Ok(Self {
            offset,
            children,
            plugin,
        })
    }
}

// TODO: make it not untagged and just use fields

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(
    untagged,
    expecting = "expected a path or inline plugin in the form of {...}"
)]
pub enum Child {
    Path(PathBuf),
    Inline(Config),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)]
pub struct PluginInfo {
    pub name: PluginName,
    pub version: semver::Version,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Export {
    #[serde(default, skip_serializing_if = "is_default")]
    pub builtin: bool,
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
    expecting = r#"expected dependency options in the form of "<version>" or ( version = "<version", ... ) "#
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
