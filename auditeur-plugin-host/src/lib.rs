// - have a handle to the shared library
//   - each listener, watcher, and resource automatically keeps the handle
//   - plugins are also given a cloneable handle so they can put it in any trait objects they give
//   other plugins
//     - this should be put in some Context-like struct to allow adding new things in the future

// - when plugins are reloaded their dependencies *have* to be recompiled and reloaded too
//   - this is because the dependencies will expect the dependent to be structured in a certain
//   way, which may change when the plugin is reloaded

// notice that this library *has* to stay loaded as long as the plugin is loaded in the app
// (so pretty much the lifetime of the program)
// the app uses trait objects (such as Box<dyn Runnable>) to allow for external plugins
// but since the trait objects reference code from their library, if the library gets unloaded,
// they break (they can't find the thing that they're referencing, so they segfault)

use std::{collections::HashMap, path::PathBuf};

use arena::Arena;
use auditeur_plugin::auditeur::{app::App, prelude::*, Message};
use config::{PluginName, Source};
use futures::Future;
use log::error;
use thiserror::Error;

pub mod arena;
pub mod config;
pub mod registry;

mod prelude {
    pub use crate::{
        config::{self, GitShorthand, PluginName, Source},
        registry::PluginRegistry,
        Error, ErrorSink,
    };
}

use prelude::*;

pub use auditeur_plugin;

pub trait AppExt {
    /// # Safety
    ///
    /// The plugin system calls foreign code through the use of [dynamic
    /// linking](auditeur_plugin::LibraryHandle), so it is inherently unsafe - see
    /// [`PluginRegistry::load_plugin_from_file_unchecked`] for more information. However, the
    /// boundary is checked in many ways. The main way that unsafety could come from other plugins
    /// is from them having unsafe code or intentionally going past the safety checks, which isn't
    /// something that can be prevented. Thus, thus function isn't marked unsafe.
    #[allow(opaque_hidden_inferred_bound)] // I think this is a false positive?
    fn with_plugin_system(self, config: &config::Host) -> impl Future<Output = Self>;
}

impl<M: Message> AppExt for App<M> {
    async fn with_plugin_system(mut self, config: &config::Host) -> Self {
        let mut registry = PluginRegistry::new(&mut self);
        let mut arena = Arena::open_or_init(config).await;
        if let Some(arena) = arena.build().await {
            arena.add_to_registry(&mut registry);
        }
        // SAFETY: we add the registry immediately below, but in a bundle
        let registry = unsafe { registry.finalize() };
        self.with_bundle(new_bundle!("auditeur:plugin_host").with_listener(registry))
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("error when loading plugin: {0}")]
    Loading(#[from] auditeur_plugin::loading::Error),
    #[error("error with git repository '{url}': {error}")]
    Git {
        #[source]
        error: git2::Error,
        url: String,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path source '{0}' is not a directory on disk")]
    PathNotFound(PathBuf),
    #[error("path source '{0}' is empty")]
    NoComponentsInPath(PathBuf),
    #[error("'plugin.ron' was not found at '{0}'")]
    PluginFileNotFound(PathBuf),
    #[error("failed to serialize to file '{file}': {error}")]
    Serialization {
        #[source]
        error: ron::Error,
        file: PathBuf,
    },
    #[error("failed to deserialize from file '{file}': {error}")]
    Deserialization {
        #[source]
        error: ron::de::SpannedError,
        file: PathBuf,
    },
    #[error("build failed: {stderr}")]
    BuildFailed { stderr: String },
    #[error("failed to parse '{path}': {error}")]
    CargoParse {
        #[source]
        error: cargo_toml::Error,
        path: PathBuf,
    },
    #[error("plugin '{0}' does not specify a package in its Cargo.toml")]
    NoPackage(PluginName),
    #[error("plugin '{for_plugin}' requires dependency '{dependency}', which doesn't exist")]
    UnsatisfiedDependency {
        for_plugin: PluginName,
        dependency: PluginName,
    },
    #[error("failed to serialize to file '{path}': {error}")]
    TomlSerialization {
        #[source]
        error: toml::ser::Error,
        path: PathBuf,
    },
    #[error("invalid rustc version, found: '{found}', expected: '{expected}'")]
    InvalidRustc { found: String, expected: String },
    #[error("recieved message from cargo: {}", .0.message.rendered.as_deref().unwrap_or_default())]
    CargoMessage(cargo_metadata::CompilerMessage),
    #[error("plugin '{name}' has no library crate (checked {})", manifest_path.display())]
    PluginNoLibrary {
        name: PluginName,
        manifest_path: PathBuf,
    },
    #[error("plugin '{name}' has no package (checked {})", manifest_path.display())]
    PluginNoPackage {
        name: PluginName,
        manifest_path: PathBuf,
    },
}

impl From<cargo_metadata::CompilerMessage> for Error {
    fn from(value: cargo_metadata::CompilerMessage) -> Self {
        Self::CargoMessage(value)
    }
}

pub struct ErrorSink {
    plugin_map: HashMap<PluginName, Vec<Error>>,
    source_map: HashMap<Source, Vec<Error>>,
    generic: Vec<Error>,
}

impl ErrorSink {
    fn new() -> Self {
        Self {
            plugin_map: HashMap::new(),
            source_map: HashMap::new(),
            generic: Vec::new(),
        }
    }

    fn add_generic(&mut self, error: impl Into<Error>) {
        let error = error.into();
        error!("encountered error when loading plugin system: {error}");
        self.generic.push(error);
    }

    #[must_use]
    pub fn generic(&self) -> &[Error] {
        &self.generic[..]
    }

    fn add_for_plugin(&mut self, name: PluginName, error: impl Into<Error>) {
        let error = error.into();
        error!("encountered error when loading plugin '{name}': {error}");
        self.plugin_map.entry(name).or_default().push(error);
    }

    #[must_use]
    pub const fn plugin_map(&self) -> &HashMap<PluginName, Vec<Error>> {
        &self.plugin_map
    }

    #[must_use]
    pub fn for_plugin(&self, name: &PluginName) -> &[Error] {
        self.plugin_map
            .get(name)
            .map(|vec| &vec[..])
            .unwrap_or_default()
    }

    fn add_for_source(&mut self, source: Source, error: impl Into<Error>) {
        let error = error.into();
        error!("encountered error when loading source '{source}': {error}");
        self.source_map.entry(source).or_default().push(error);
    }

    #[must_use]
    pub const fn source_map(&self) -> &HashMap<Source, Vec<Error>> {
        &self.source_map
    }

    #[must_use]
    pub fn for_source(&self, source: &Source) -> &[Error] {
        self.source_map
            .get(source)
            .map(|vec| &vec[..])
            .unwrap_or_default()
    }
}
