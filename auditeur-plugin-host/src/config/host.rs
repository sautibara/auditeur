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
///     plugin_arena: "plugin_arena",
///     compiled_plugins: "plugins",
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
}

impl Config {
    pub fn new(project_dirs: ProjectDirs) -> Self {
        Self {
            directories: Directories::new(project_dirs),
            arena: Arena::default(),
            plugins: vec![],
        }
    }

    pub fn directories(&self) -> &Directories {
        &self.directories
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Directories {
    data_root: PathBuf,
    plugin_arena: PathBuf,
    compiled_plugins: PathBuf,
}

impl Directories {
    pub fn new(project_dirs: ProjectDirs) -> Self {
        let local_data_dir = project_dirs.data_local_dir();
        Self {
            data_root: local_data_dir.to_path_buf(),
            plugin_arena: PathBuf::from("plugin_arena"),
            compiled_plugins: PathBuf::from("plugins"),
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
    pub fn arena_dir(&self) -> PathBuf {
        self.extend_data_root_with(&self.plugin_arena)
    }

    /// Returns the path that holds compiled plugins for the app
    pub fn plugin_dir(&self) -> PathBuf {
        self.extend_data_root_with(&self.compiled_plugins)
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(untagged)]
#[serde(
    expecting = r#"expected a git shorthand in the form of "{user}/{repo}" or a struct containing a valid source"#
)]
enum PluginSerialized {
    // only a string - default to an enabled github source
    // ex: "sautibara/sautibara-rpc"
    UrlOnly(GitShorthand),
    // no defaults, everything is included
    Full {
        // the source is flattened so it can be provided with only the source type
        // ex: ( github: "sautibara/sautibara-rpc" )
        #[serde(flatten)]
        source: Source,
        // skip serializing or deserializing if it's already the default value
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
    pub fn get(&self, name: &PluginName) -> bool {
        match self {
            Self::All(val) => *val,
            Self::PerPlugin(map) => map.get(name).copied().unwrap_or_default(),
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
        if is_default(&plugin.enabled) {
            if let Source::Github(url) = plugin.source {
                return Self::UrlOnly(url);
            }
        }
        Self::Full {
            source: plugin.source,
            enabled: plugin.enabled,
        }
    }
}

impl From<PluginSerialized> for Plugin {
    fn from(plugin: PluginSerialized) -> Self {
        match plugin {
            PluginSerialized::UrlOnly(url) => Self {
                source: Source::Github(url),
                enabled: Enabled::default(),
            },
            PluginSerialized::Full { source, enabled } => Self { source, enabled },
        }
    }
}
