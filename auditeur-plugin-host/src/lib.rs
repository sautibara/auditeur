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

use std::{
    borrow::Borrow,
    fmt::Display,
    path::{Path, PathBuf},
};

use arena::Arena;
use auditeur_plugin::auditeur::app::App;
use futures::Future;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;

pub mod arena;
pub mod config;
pub mod registry;
pub mod source;

mod prelude {
    pub use crate::{
        config,
        registry::PluginRegistry,
        source::{self, git, Binary, OwnedId as SourceId, Source, Updated},
        Error, PluginName,
    };
    pub use auditeur_plugin::prelude::*;
}

use prelude::*;

pub use auditeur_plugin;

/// Spawn a blocking closure in [`tokio`], resuming any panics from the closure if they occur
async fn spawn_blocking<F, R>(func: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match tokio::task::spawn_blocking(func).await {
        Err(err) => std::panic::resume_unwind(err.into_panic()),
        Ok(val) => val,
    }
}

/// Serialize `value` to the file at `path`, using [`ron`]
async fn serialize<T: Serialize + Send + Sync + 'static>(
    path: PathBuf,
    value: T,
) -> Result<(), Error> {
    spawn_blocking(move || {
        let file = std::fs::File::create(&path)?;
        ron::ser::to_writer(file, &value)
            .map_err(|error| Error::Serialization { error, file: path })?;
        Ok(())
    })
    .await
}

/// Deserialize `value` from to the file at `path`, using [`ron`]
async fn deserialize<T: for<'de> Deserialize<'de> + Send + 'static>(
    path: PathBuf,
) -> Result<T, Error> {
    spawn_blocking(move || {
        let file = std::fs::File::open(&path)?;
        let value = ron::de::from_reader(file)
            .map_err(|error| Error::Deserialization { error, file: path })?;
        Ok(value)
    })
    .await
}

/// Returns `true` if `path` exists and isn't empty
fn path_exists_with_children(path: &Path) -> Result<bool, Error> {
    Ok(path.exists() && path.is_dir() && std::fs::read_dir(path)?.next().is_some())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PluginName(pub String);

impl Borrow<String> for PluginName {
    fn borrow(&self) -> &String {
        &self.0
    }
}

impl Borrow<str> for PluginName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
    fn with_plugin_system(self, config: config::Host) -> impl Future<Output = Self>;
}

impl AppExt for App {
    async fn with_plugin_system(mut self, config: config::Host) -> Self {
        let (mut registry, plugin_sender) = PluginRegistry::new();
        let (mut arena, mut errors) = Arena::open_or_init(config, plugin_sender).await;
        if let Ok(arena) = &mut arena {
            let res = arena.add_to_plugin_registry(&mut registry, &mut self);
            if let Err(err) = res {
                errors.push(err);
            }
        }
        self.add_bundle(
            new_bundle!("auditeur:plugin_host")
                .with_watcher::<Arena>(arena.map_err(Into::into))
                .with_watcher::<PluginRegistry>(registry),
        );
        self.add_errors(
            Arena::const_ident(),
            errors.into_iter().map(Into::into),
        );
        self
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
    #[error("failed to serialize to file '{path}': {error}")]
    TomlSerialization {
        #[source]
        error: toml::ser::Error,
        path: PathBuf,
    },
    #[error("invalid rustc version, found: '{found}', expected: '{expected}'")]
    InvalidRustc { found: String, expected: String },
    #[error("received message from cargo: {}", .0.message.rendered.as_deref().unwrap_or_default())]
    CargoMessage(Box<cargo_metadata::CompilerMessage>),
    #[error(
        "plugin '{plugin}' has no library crate (checked {})",
        manifest_path.display(),
    )]
    LibraryExpected {
        plugin: PluginName,
        manifest_path: PathBuf,
    },
    #[error(
        "plugin '{plugin}' has no package (checked {})",
        manifest_path.display()
    )]
    PackageExpected {
        plugin: PluginName,
        manifest_path: PathBuf,
    },
    #[error("dependency '{dependency}' for plugin '{plugin}' was not found")]
    PluginDependencyNotFound {
        plugin: PluginName,
        dependency: PluginName,
    },
    #[error("compiler artifact for '{plugin}' ('{artifact}') has no filename")]
    ArtifactHasNoFilename {
        plugin: PluginName,
        artifact: PathBuf,
    },
}

impl From<cargo_metadata::CompilerMessage> for Error {
    fn from(value: cargo_metadata::CompilerMessage) -> Self {
        Self::CargoMessage(Box::new(value))
    }
}

impl From<Error> for Vec<Error> {
    fn from(value: Error) -> Self {
        vec![value]
    }
}
