use std::{collections::HashMap, ffi::OsStr, marker::PhantomData, sync::Arc};

use auditeur::{app::App, prelude::*, Message};
use log::debug;
use stabby::libloading::StabbyLibrary;

use crate::{Error, LibraryHandle, PluginContext, PluginFn};

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`] of all loaded plugins, keeping their dynamic libraries
/// loaded. The registry only drops the handle when [`RegistryEvent::BundleUnloaded`] is recieved
/// for the plugin's bundle, so that the library isn't held for forever.
#[must_use = "the registry must be added as a listener to the app, otherwise plugin code will be immediately unloaded"]
#[resource("plugin_host:registry")]
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
