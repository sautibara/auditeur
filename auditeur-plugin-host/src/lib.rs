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

use std::{collections::HashMap, ffi::OsStr, marker::PhantomData, sync::Arc};

use auditeur::{app::App, prelude::*, Message};
use libloading::Library;
use log::debug;
use stabby::libloading::StabbyLibrary;
use thiserror::Error;

#[doc(hidden)]
pub use stabby::export as stabby_export;

type PluginFn<M> = fn(PluginContext) -> Bundle<M>;

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

/// A plugin and its associated [`Bundle`] that has already been loaded into an [`App`]
struct Plugin {
    library: LibraryHandle,
    bundle: NamespacedId,
}

impl Plugin {
    /// Load a plugin from a dynamic library at `path` and run its `main` function
    ///
    /// # Safety
    ///
    /// Calling a function from a dynamic library is inherently unsafe, as we're trusting the
    /// it to have the correct signature. If the signature is wrong, it could lead to undefined
    /// behavior.
    ///
    /// # Errors
    ///
    /// - If the library fails to load
    /// - If the library's main function fails to load
    unsafe fn load_from_file_unchecked<M: Message>(
        path: impl AsRef<OsStr>,
        app: &mut App<M>,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let library = libloading::Library::new(path)?;
        let handle = LibraryHandle(Arc::new(library));
        let main: libloading::Symbol<PluginFn<M>> = handle
            .library()
            .get_canaried(b"init")
            .map_err(|err| Error::CantLoadInitFunction {
                path: path.to_string_lossy().to_string(),
                additional_info: err.to_string(),
            })?;
        let context = PluginContext {
            handle: handle.clone(),
        };
        let bundle = main(context);
        let bundle_id = bundle.id().into_owned();
        app.add_bundle(bundle);
        Ok(Self {
            library: handle,
            bundle: bundle_id,
        })
    }
}

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`] of all loaded plugins, keeping their dynamic libraries
/// loaded. The registry only drops the handle when [`RegistryEvent::BundleUnloaded`] is recieved
/// for the plugin's bundle, so that the library isn't held for forever.
#[must_use = "the registry must be added as a listener to the app, otherwise plugin code will be immediately unloaded"]
#[resource("plugin_host:library_holder")]
pub struct PluginRegistry<M: Message> {
    libraries: HashMap<NamespacedId, LibraryHandle>,
    message: PhantomData<M>,
}

impl<M: Message> Default for PluginRegistry<M> {
    fn default() -> Self {
        Self {
            libraries: Default::default(),
            message: Default::default(),
        }
    }
}

impl<M: Message> PluginRegistry<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a plugin from a dynamic library at `path`, run its `main` function, and save its
    /// handle in the registry
    ///
    /// # Safety
    ///
    /// Calling a function from a dynamic library is inherently unsafe, as we're trusting the
    /// it to have the correct signature. If the signature is wrong, it could lead to undefined
    /// behavior.
    ///
    /// # Errors
    ///
    /// - If the library fails to load
    /// - If the library's main function fails to load
    pub unsafe fn load_plugin_from_file_unchecked(
        &mut self,
        path: impl AsRef<OsStr>,
        app: &mut App<M>,
    ) -> Result<&mut Self, Error> {
        let plugin = Plugin::load_from_file_unchecked(path, app)?;
        self.libraries.insert(plugin.bundle, plugin.library);
        Ok(self)
    }
}

#[async_trait]
impl<M: Message> Listener<M> for PluginRegistry<M> {
    type Input = ();

    async fn handle(&mut self, _: M, (): &mut ()) -> Result<()> {
        Ok(())
    }

    async fn registry_event(&mut self, event: RegistryEvent, (): &mut ()) -> Result<()> {
        if let RegistryEvent::BundleUnloaded { id } = event {
            self.libraries.remove(&id);
            debug!("unloaded bundle `{id}`'s library");
        }
        Ok(())
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
}
