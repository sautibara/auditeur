use std::{
    collections::HashMap,
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
    pub plugins: Vec<Plugin>,
    #[serde(skip)]
    pub static_dependencies: Vec<Source>,
}

impl Config {
    #[must_use]
    pub fn defaults(project_dirs: &ProjectDirs) -> Self {
        Self {
            directories: Directories::defaults(project_dirs),
            arena: Arena::default(),
            plugins: vec![],
            static_dependencies: vec![],
        }
    }

    #[must_use]
    pub const fn directories(&self) -> &Directories {
        &self.directories
    }

    #[must_use]
    pub fn with_static_dependency(mut self, dependency: Source) -> Self {
        self.static_dependencies.push(dependency);
        self
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Arena {
    pub retain_build_artifacts: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(from = "PluginSerialized", into = "PluginSerialized")]
pub struct Plugin {
    pub source: Source,
    pub enabled: Enabled,
    pub dependencies: Vec<Plugin>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
#[serde(
    expecting = r#"expected a git shorthand in the form of "{user}/{repo}" or a struct containing a valid source"#
)]
enum PluginSerialized {
    // only a string - default to an enabled github source
    // ex: "sautibara/sautibara-rpc"
    UrlOnly(git::Shorthand),
    // no defaults, everything is included
    Full {
        // the source is flattened so it can be provided with only the source type
        // ex: ( github: "sautibara/sautibara-rpc" )
        #[serde(flatten)]
        source: Source,
        // skip serializing or deserializing if it's already the default value
        #[serde(default, skip_serializing_if = "is_default")]
        enabled: Enabled,
        #[serde(default, skip_serializing_if = "is_default")]
        dependencies: Vec<Plugin>,
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

impl From<Plugin> for PluginSerialized {
    fn from(plugin: Plugin) -> Self {
        let variant = plugin.source.into();
        if is_default(&plugin.enabled) {
            if let source::Variant::Github(url) = variant {
                return Self::UrlOnly(url);
            }
        }
        Self::Full {
            source: variant.into(),
            enabled: plugin.enabled,
            dependencies: plugin.dependencies,
        }
    }
}

impl From<PluginSerialized> for Plugin {
    fn from(plugin: PluginSerialized) -> Self {
        match plugin {
            PluginSerialized::UrlOnly(url) => Self {
                source: source::Variant::Github(url).into(),
                enabled: Enabled::default(),
                dependencies: vec![],
            },
            PluginSerialized::Full {
                source,
                enabled,
                dependencies,
            } => Self {
                source,
                enabled,
                dependencies,
            },
        }
    }
}
