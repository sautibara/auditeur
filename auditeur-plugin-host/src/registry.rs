use std::collections::{HashMap, HashSet};

use auditeur_plugin::auditeur::{
    app::{App, Registry, event::BundleInit, resource_map::ResourceMapConnection},
    bundle,
    prelude::*,
};
use tracing::{debug, error, trace, warn};

use crate::{Error, PluginName, source::Binary};
use auditeur_plugin::loading::Plugin;

// FIXME: Let a, b, and c be resources within separate libraries.
// Then, let a depend on b, and c own a reference and LibraryHandle to a.
// Then, let a be dropped in the Registry.
// But now since the PluginRegistry doesn't see a being used, it can also unload b.
// But a still depends on b - when c uses a, a could still try to use b - a use after free.
// This could be fixed by requiring references to be added when they're given to another resource.

// Fix this by creating some type of CallbackCollector (please change name) that ould collect dynamic
// objects that implement some type of Source trait, which would allow getting the source id of a
// dynamic object (eventually for a RegistryEvent::Reference event). This would be included as a
// component with shared (UserInput) input of some type of CallbackCollection. It would then listen
// on some type of message channel that could send new callbacks. The callbacks would be owned by
// the CallbackCollection and only accessible as references. This would allow removing callbacks
// too.

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`](auditeur_plugin::LibraryHandle) of all loaded plugins,
/// keeping their dynamic libraries loaded. The registry only drops the handle when
/// [`RegistryEvent::Unloaded`] is received for the plugin's bundle, so that the library isn't
/// held for forever.
#[must_use = "if the registry isn't added to an app, the libraries that are loaded will be leaked"]
#[derive(Runnable, Default)]
#[listener(
    "plugin_host:registry",
    listen_to_registry,
    registry_component = false,
    respond,
    listen(BundleInit)
)]
#[allow(clippy::module_name_repetitions)] // conflicts with the normal auditeur Registry
pub struct PluginRegistry {
    libraries: HashMap<NamespacedId, bundle::SharedInitFn>,
    bundles: HashMap<PluginName, NamespacedId>,
    plugins: HashSet<NamespacedId>,
    should_exit: bool,
}

#[create]
impl Create for PluginRegistry {
    async fn create(#[external] this: Self) -> Result<Self> {
        Ok(this)
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

#[has_input]
impl ListenToRegistry for PluginRegistry {
    async fn on(&mut self, message: RegistryEvent, (): ()) -> anyhow::Result<bool> {
        match message {
            RegistryEvent::Unloaded(ComponentId::Bundle(id)) => {
                if let Some(library) = self.libraries.remove(&id) {
                    debug!("unloading bundle `{id}`'s library...");
                    drop(library);
                    debug!("unloaded bundle `{id}`'s library");
                }
                // we only should stop if all of the libraries are unloaded first
                if self.should_exit && self.libraries.is_empty() {
                    return Ok(true);
                }
            }
            // NOTE: extra stop signals are ignored
            RegistryEvent::Exit => {
                // WARN: if this is removed and no bundles are unloaded after we
                // receive the exit signal, the registry will never exit.
                if self.libraries.is_empty() {
                    return Ok(true); // we already get to exit :)
                }
                self.should_exit = true;
            }
            _ => (),
        }
        Ok(false)
    }
}

#[has_input]
impl Listen<BundleInit> for PluginRegistry {
    async fn on(
        &mut self,
        message: BundleInit,
        resource_map: &mut ResourceMapConnection,
    ) -> anyhow::Result<()> {
        if !self.plugins.contains(message.id()) {
            return Ok(());
        }

        if let Some(new_init) = message.init() {
            if let Some(old_init) = self.libraries.remove(message.id())
                && old_init != new_init
                // double check to make sure it's actually still loaded
                && resource_map.loaded_bundles().await.contains(message.id())
            {
                // NOTE: this is the main reason the PluginRegistry exists (at least now).
                // Initially, the registry was created to keep libraries of bundles always alive.
                // But when reloading was added, the ResourceMap began keeping libraries alive
                // through their initialization functions (which have a handle to the library
                // stored). But there's a catch! See, the ResourceMap only keeps a single
                // initialization function stored per bundle (if it exists), so it has to kick out
                // old ones when new ones come in. If only part of a bundle is reloaded, then,
                // there may be two versions of the plugin... but then the old library is unloaded
                // (as the map kicks it out), which would lead the resources from the old version
                // to segfault (as their code is no longer loaded).
                // HACK: the fix for this, at least now, is to just leak the old library - just
                // like what happens when the registry is dropped with libraries.
                error!(
                    concat!(
                        "attempted to reload plugin with bundle id '{}' without unloading all of",
                        " its resources, forced to leak its library",
                    ),
                    message.id(),
                );
                std::mem::forget(old_init);
            }

            self.libraries.insert(message.id().to_owned(), new_init);
        }

        Ok(())
    }
}

impl PluginRegistry {
    /// Load a plugin from a dynamic library at `path`, run its `main` function, and save its
    /// handle in the registry.
    ///
    /// Returns the [`NamespacedId`] of the bundle that is contained in the plugin.
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
        library: &Binary,
        app: &mut App,
    ) -> Result<NamespacedId, Error> {
        trace!("loading binary at '{}' to app", library.path.display());
        // SAFETY: upheld by caller
        let plugin = unsafe { Plugin::load_from_file_unchecked(&library.path, app) }?;
        self.libraries.insert(plugin.bundle.clone(), plugin.init);
        self.bundles
            .insert(library.name.clone(), plugin.bundle.clone());
        self.plugins.insert(plugin.bundle.clone());
        Ok(plugin.bundle)
    }
}

#[respond("plugin_host:registry")]
impl PluginRegistry {
    /// # Safety
    ///
    /// Each of these plugins must adhere to the safety contract specified in
    /// [`PluginRegistry::load_plugin_from_file_unchecked`]
    pub async unsafe fn load_binaries(
        &mut self,
        plugins: Vec<Binary>,
        registry: &mut Registry,
    ) -> Result<(), Vec<Error>> {
        let mut errors = Vec::new();

        trace!("recieved plugins: {plugins:?}");
        let mut app = registry.new_app();

        // FIXME: check for necessary unloads
        // (when a plugin is updated, for example)
        for plugin in plugins {
            // TODO: handle this error
            // SAFETY: safety must be upheld by the caller of PluginSender::send
            let res = unsafe { self.load_plugin_from_file_unchecked(&plugin, &mut app) };
            if let Err(err) = res {
                errors.push(err);
            }
        }

        if let Err((app, _)) = app.add_to_registry(registry).await {
            for unloaded_bundle in app.unloaded_bundles() {
                // FIXME: If only part of a bundle is loaded, the registry gets put
                // into a terrible situation. It can't just drop the library, since one
                // of the library's resources could have still been loaded. But it also
                // can't keep the library, since it will never be unloaded: the
                // ResourceMap doesn't know that one of the resources was never loaded,
                // so it'll forever wait for it to unload before unloading the bundle.
                // Instead, we use the hacky solution: just leak the library and keep
                // it always loaded.
                let library = self.libraries.remove(unloaded_bundle);
                if let Some(library) = library {
                    std::mem::forget(library);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Drop for PluginRegistry {
    fn drop(&mut self) {
        #[allow(clippy::collection_is_never_read)]
        let libraries: Vec<_> = {
            let libraries = &mut self.libraries;
            // the registry should only get dropped when it's empty
            if libraries.is_empty() {
                return;
            }
            libraries.drain().map(|(_, library)| library).collect()
        };
        // if the registry isn't empty, then leak the library handles just in case
        warn!("PluginRegistry was dropped with libraries, leaking them");
        libraries.leak();
    }
}
