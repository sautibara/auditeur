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

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use arena::Arena;
use auditeur::{app::App, prelude::*, Message};
use config::{PluginName, Source};
use futures::Future;
use libloading::Library;
use log::error;
use thiserror::Error;

pub mod arena;
pub mod config;
mod registry;
pub use registry::PluginRegistry;

mod prelude {
    pub use crate::{
        config::{self, GitShorthand, PluginName, Source},
        Error, ErrorSink, PluginRegistry,
    };
}

#[doc(hidden)]
pub use stabby::export as stabby_export;

type PluginFn<M> = fn(PluginContext) -> Bundle<M>;

pub trait AppExt {
    /// # Safety
    ///
    /// The plugin system calls foreign code through the use of [dynamic linking](libloading), so
    /// it is inherently unsafe.
    #[allow(opaque_hidden_inferred_bound)] // I think this is a false positive?
    unsafe fn with_plugin_system(self, config: &config::Host) -> impl Future<Output = Self>;
}

impl<M: Message> AppExt for App<M> {
    async unsafe fn with_plugin_system(mut self, config: &config::Host) -> Self {
        let mut registry = PluginRegistry::new();
        let arena = Arena::open_or_init(config).await;
        if let Ok(mut arena) = arena.build().await {
            arena.add_binaries_to_registry(&mut registry, &mut self);
        }
        self.with_bundle(
            new_bundle!("auditeur:plugin_host")
                .with_resource_map()
                .with_listener(registry),
        )
    }
}

/// A handle to a [shared library](::libloading) that contains a plugin
///
/// # Safety
///
/// Once all handles are dropped, the library containing the plugin will be automatically unloaded.
/// This'll cause future attempts to call code from this plugin to segfault (to try to access memory
/// that isn't owned by the process), as the memory which should hold the code would already be
/// freed by unloading.
///
/// All [resources](mod@auditeur::resource)'s lifetimes are automatically handled by the
/// [`ResourceMap`](auditeur::resource::builtin::ResourceMap), so they will never unload while
/// being used. However, trait objects given to other plugins may cause undefined behavior if all
/// handles are dropped and the code that the trait object links to is called. To prevent this,
/// handles can be put within the trait objects to ensure that the library lasts at least as long
/// as the object.
// ^ this is some prime yappage
///
/// # Common Pitfalls
///
/// ## Exiting too Early
///
/// If a listener or watcher sends resources to other threads and then exits, those references will
/// be forgotten by the main [`ResourceMap`](crate::resource::builtin::ResourceMap), leading to
/// libraries being freed too early.
// TODO: example here
// TODO: move this to auditeur-plugin and reverse the dependency
#[derive(Clone)]
pub struct LibraryHandle(Arc<Library>);

impl LibraryHandle {
    pub(crate) fn library(&self) -> &Library {
        &self.0
    }
}

/// Additional context given to a plugin during load
///
/// This most notably contains a [`LibraryHandle`], which is sometimes necessary to keep the plugin
/// loaded.
#[derive(Clone)]
pub struct PluginContext {
    handle: LibraryHandle,
}

impl PluginContext {
    /// Get an owned handle to the plugin's dynamic library
    ///
    /// See [`LibraryHandle`] for more information
    pub fn handle(&self) -> LibraryHandle {
        self.handle.clone()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An error associated with loading a plugin dynamic library
    #[error("failed to load library: {0}")]
    LibLoading(#[from] libloading::Error),
    /// The plugin loader couldn't find or call the `main` function in a plugin
    #[error("couldn't call the init function in plugin `{path}`: {additional_info}")]
    CantLoadInitFunction {
        path: String,
        additional_info: String,
    },
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

    fn add_generic(&mut self, error: Error) {
        error!("encountered error when loading plugin system: {error}");
        self.generic.push(error);
    }

    pub fn generic(&self) -> &[Error] {
        &self.generic[..]
    }

    fn add_for_plugin(&mut self, name: PluginName, error: Error) {
        error!("encountered error when loading plugin '{name}': {error}");
        self.plugin_map.entry(name).or_default().push(error)
    }

    pub fn plugin_map(&self) -> &HashMap<PluginName, Vec<Error>> {
        &self.plugin_map
    }

    pub fn for_plugin(&self, name: PluginName) -> &[Error] {
        self.plugin_map
            .get(&name)
            .map(|vec| &vec[..])
            .unwrap_or_default()
    }

    fn add_for_source(&mut self, source: Source, error: Error) {
        error!("encountered error when loading source '{source}': {error}");
        self.source_map.entry(source).or_default().push(error)
    }

    pub fn source_map(&self) -> &HashMap<Source, Vec<Error>> {
        &self.source_map
    }

    pub fn for_source(&self, source: Source) -> &[Error] {
        self.source_map
            .get(&source)
            .map(|vec| &vec[..])
            .unwrap_or_default()
    }
}
