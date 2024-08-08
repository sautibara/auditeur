use std::{collections::HashMap, ffi::OsStr, marker::PhantomData};

use auditeur_plugin::auditeur::{app::App, prelude::*, Message};
use log::{debug, trace};

use crate::Error;
use auditeur_plugin::{loading::Plugin, LibraryHandle};

pub struct Building<'app, M: Message> {
    app: &'app mut App<M>,
}
#[derive(PathAny)]
pub struct Finalized;

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`] of all loaded plugins, keeping their dynamic libraries
/// loaded. The registry only drops the handle when [`RegistryEvent::BundleUnloaded`] is recieved
/// for the plugin's bundle, so that the library isn't held for forever.
#[must_use = "the registry *must* be added as a listener to the app, otherwise plugin code may be unloaded while running"]
#[resource("plugin_host:registry")]
#[resource_custom_generics = "(M: Message) - (M, Finalized)"]
pub struct PluginRegistry<M: Message, A> {
    libraries: HashMap<NamespacedId, LibraryHandle>,
    maybe_app: A,
    message: PhantomData<M>,
}

impl<'app, M: Message> PluginRegistry<M, Building<'app, M>> {
    pub fn new(app: &'app mut App<M>) -> Self {
        Self {
            libraries: HashMap::new(),
            maybe_app: Building { app },
            message: PhantomData,
        }
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
    ) -> Result<&mut Self, Error> {
        let path = path.as_ref();
        trace!("loading binary at '{}' to app", path.to_string_lossy());
        let plugin = Plugin::load_from_file_unchecked(path, self.maybe_app.app)?;
        self.libraries.insert(plugin.bundle, plugin.library);
        Ok(self)
    }

    /// # Safety
    ///
    /// If the registry is dropped before dynamically loaded resources added to the [`App`] are,
    /// their shared libraries will be dropped, which will lead to undefined behavior (See
    /// [`LibraryHandle`] for a more in-depth explanation). As such, the registry *must* be added
    /// as a [`Listener`] to the app, which will keep it alive.
    pub unsafe fn finalize(self) -> PluginRegistry<M, Finalized> {
        PluginRegistry {
            libraries: self.libraries,
            maybe_app: Finalized,
            message: PhantomData,
        }
    }

    pub fn finalize_and_add(self) {
        let app = self.maybe_app.app;
        let finalized = PluginRegistry {
            libraries: self.libraries,
            maybe_app: Finalized,
            message: PhantomData,
        };
        app.add_bundle(new_bundle!("auditeur:plugin_host_registry").with_listener(finalized));
    }
}

#[async_trait]
impl<M: Message> Listener<M> for PluginRegistry<M, Finalized> {
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
