use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

pub struct PluginManager;

/// # Examples
///
///  ```
/// # fn main() -> Result<(), ron::de::SpannedError> { let file = r#"
/// (
///   directories: (
///     data_root: "/home/thate/.local/share/sautibara-app",
///     workspace: "workspace",
///     compiled_plugins: "plugins",
///     sources: "sources",
///   ),
///   arena: (
///     retain_build_artifacts: false,
///     auto_add_dependencies: true,
///     developer_mode: false,
///   ),
///   plugins: [
///     "sautibara/sautibara-rpc",
///     (
///       github: "sautibara/sautibara-media-keys",
///       enabled: false,
///     ),
///     ( builtin: ( path: "libsautibara-rpc.dll", name: "sautibara_rpc" ) ),
///   ]
/// )
/// # "#; ron::de::from_str::<auditeur_plugin_host::config::Host>(file).map(drop) }
///  ```
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub directories: Directories,
    #[serde(default)]
    pub arena: Arena,
    #[serde(default)]
    pub plugins: PluginGroupMap,
}

impl Config {
    #[must_use]
    pub fn defaults(project_dirs: &ProjectDirs) -> Self {
        Self {
            directories: Directories::defaults(project_dirs),
            arena: Arena::default(),
            plugins: PluginGroupMap::default(),
        }
    }

    #[must_use]
    pub const fn directories(&self) -> &Directories {
        &self.directories
    }

    #[must_use]
    pub fn source(&self, id: &SourceId) -> Option<&Source> {
        self.plugins.get(id).map(|plugin| &plugin.source)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Directories {
    data_root: PathBuf,
    #[serde(default = "default_workspace")]
    workspace: PathBuf,
    #[serde(default = "default_compiled_plugins")]
    compiled_plugins: PathBuf,
    #[serde(default = "default_sources")]
    sources: PathBuf,
}

fn default_workspace() -> PathBuf {
    PathBuf::from("workspace")
}

fn default_compiled_plugins() -> PathBuf {
    PathBuf::from("plugins")
}

fn default_sources() -> PathBuf {
    PathBuf::from("sources")
}

impl Directories {
    #[must_use]
    pub fn defaults(project_dirs: &ProjectDirs) -> Self {
        let local_data_dir = project_dirs.data_local_dir();
        Self {
            data_root: local_data_dir.to_path_buf(),
            workspace: default_workspace(),
            compiled_plugins: default_compiled_plugins(),
            sources: default_sources(),
        }
    }

    fn extend_data_root_with(&self, path: &Path) -> PathBuf {
        // if the path is absolute, just use it whole
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.data_root.join(path)
        }
    }

    /// Returns the path that holds the shared cargo workspace to compile plugins
    #[must_use]
    pub fn workspace_dir(&self) -> PathBuf {
        self.extend_data_root_with(&self.workspace)
    }

    /// Returns the path that holds compiled plugins for the app
    #[must_use]
    pub fn binary_dir(&self) -> PathBuf {
        self.extend_data_root_with(&self.compiled_plugins)
    }

    /// Returns the path that holds all plugin sources
    #[must_use]
    pub fn source_dir(&self) -> PathBuf {
        self.extend_data_root_with(&self.sources)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Arena {
    #[serde(default = "default_true")]
    pub auto_add_dependencies: bool,
    #[serde(default)]
    pub retain_build_artifacts: bool,
    #[serde(default)]
    pub auto_update: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Arena {
    fn default() -> Self {
        Self {
            auto_add_dependencies: true,
            retain_build_artifacts: false,
            auto_update: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(from = "PluginGroupMapSerialized", into = "PluginGroupMapSerialized")]
pub struct PluginGroupMap {
    map: HashMap<SourceId, PluginGroup>,
}

impl Deref for PluginGroupMap {
    type Target = HashMap<SourceId, PluginGroup>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl DerefMut for PluginGroupMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(transparent)]
struct PluginGroupMapSerialized {
    list: Vec<PluginGroup>,
}

impl From<PluginGroupMap> for PluginGroupMapSerialized {
    fn from(value: PluginGroupMap) -> Self {
        Self {
            list: value.map.into_values().collect(),
        }
    }
}

impl From<PluginGroupMapSerialized> for PluginGroupMap {
    fn from(value: PluginGroupMapSerialized) -> Self {
        Self {
            map: value
                .list
                .into_iter()
                .map(|group| (group.source.id().clone(), group))
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(from = "PluginGroupSerialized", into = "PluginGroupSerialized")]
pub struct PluginGroup {
    pub source: Source,
    pub enabled: Enabled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
#[serde(
    expecting = r#"expected a git shorthand in the form of "{user}/{repo}" or a struct containing a valid source"#
)]
enum PluginGroupSerialized {
    // only a string - default to an enabled github source
    // ex: "sautibara/sautibara-rpc"
    UrlOnly(git::Shorthand),
    // no defaults, everything is included
    Full {
        // the source is flattened so it can be provided with only the source type
        // ex: ( github: "sautibara/sautibara-rpc" )
        #[serde(flatten)]
        source: Source,
        #[serde(default, skip_serializing_if = "is_default")]
        enabled: Enabled,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum Enabled {
    All(bool),
    PerPlugin(HashMap<PluginName, bool>),
}

impl Enabled {
    #[must_use]
    pub fn get(&self, name: &PluginName) -> bool {
        match self {
            Self::All(val) => *val,
            Self::PerPlugin(map) => map.get(name).copied().unwrap_or(true),
        }
    }

    /// Returns `true` if all plugins are enabled
    #[must_use]
    pub fn all_enabled(&self) -> bool {
        match self {
            Self::All(val) => *val,
            Self::PerPlugin(map) => map.values().all(|enabled| *enabled),
        }
    }
}

impl Default for Enabled {
    fn default() -> Self {
        Self::All(true)
    }
}

fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}

impl From<PluginGroup> for PluginGroupSerialized {
    fn from(plugin: PluginGroup) -> Self {
        let variant = plugin.source.into();
        if is_default(&plugin.enabled)
            && let source::Variant::Github(url) = variant
        {
            Self::UrlOnly(url)
        } else {
            Self::Full {
                source: variant.into(),
                enabled: plugin.enabled,
            }
        }
    }
}

impl From<PluginGroupSerialized> for PluginGroup {
    fn from(plugin: PluginGroupSerialized) -> Self {
        match plugin {
            PluginGroupSerialized::UrlOnly(url) => Self {
                source: source::Variant::Github(url).into(),
                enabled: Enabled::default(),
            },
            PluginGroupSerialized::Full { source, enabled } => Self { source, enabled },
        }
    }
}
