use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use auditeur_plugin::auditeur::{
    app::{App, EventReceiver, Registry},
    prelude::*,
};
use tokio::select;
use tracing::{debug, trace};

use crate::{Error, PluginName, source::Binary};
use auditeur_plugin::{LibraryHandle, loading::Plugin};

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

struct Shared {
    libraries: HashMap<NamespacedId, LibraryHandle>,
    bundles: HashMap<PluginName, NamespacedId>,
}

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`] of all loaded plugins, keeping their dynamic libraries
/// loaded. The registry only drops the handle when [`RegistryEvent::BundleUnloaded`] is received
/// for the plugin's bundle, so that the library isn't held for forever.
#[must_use = "if the registry isn't added to an app, the libraries that are loaded will be leaked"]
#[resource("plugin_host:registry")]
#[allow(clippy::module_name_repetitions)] // conflicts with the normal auditeur Registry
pub struct PluginRegistry {
    shared: Arc<Mutex<Shared>>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<Vec<Binary>>,
}

#[create]
impl Create for PluginRegistry {
    async fn create(#[external] registry: Self) -> Result<Self> {
        Ok(registry)
    }
}

impl PluginRegistry {
    pub fn new() -> (Self, Handle) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let shared = Shared {
            libraries: HashMap::new(),
            bundles: HashMap::new(),
        };
        let shared = Arc::new(Mutex::new(shared));
        (
            Self {
                shared: shared.clone(),
                receiver,
            },
            Handle { shared, sender },
        )
    }

    fn shared_mut(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared
            .try_lock()
            .expect("registry library lock shouldn't be poisoned")
    }

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
        {
            let mut shared = self.shared_mut();
            shared
                .libraries
                .insert(plugin.bundle.clone(), plugin.library);
            shared
                .bundles
                .insert(library.name.clone(), plugin.bundle.clone());
        }
        Ok(plugin.bundle)
    }
}

impl Drop for PluginRegistry {
    fn drop(&mut self) {
        #[allow(clippy::collection_is_never_read)]
        let libraries: Vec<_> = {
            let libraries = &mut self.shared_mut().libraries;
            // the registry should only get dropped when it's empty
            if libraries.is_empty() {
                return;
            }
            libraries.drain().map(|(_, library)| library).collect()
        };
        // if the registry isn't empty, then leak the library handles just in case
        libraries.leak();
    }
}

impl Watcher for PluginRegistry {
    type Input = (EventReceiver, Registry, Stopper);

    async fn run(&mut self, (_, _, stopper): &mut Self::Input) -> Result<()> {
        // a strange workaround to avoid getting stopped: do it ourselves first
        stopper.stop::<Self>();
        Ok(())
    }

    // I hate how much indentation this method has, but I don't see any way to improve it much...
    async fn exit(&mut self, (registry_event, registry, _): &mut Self::Input) -> Result<()> {
        let mut should_exit = false;
        loop {
            select! {
                Some(event) = registry_event.recv() => {
                    match event {
                        RegistryEvent::BundleUnloaded(id) => {
                            let libraries = &mut self.shared_mut().libraries;
                            if let Some(library) = libraries.remove(&id) {
                                debug!("unloading bundle `{id}`'s library...");
                                drop(library);
                                debug!("unloaded bundle `{id}`'s library");
                            }
                            // we only should stop if all of the libraries are unloaded first
                            if should_exit && libraries.is_empty() {
                                break;
                            }
                        }
                        // NOTE: extra stop signals are ignored
                        RegistryEvent::Exit => {
                            // WARN: if this is removed and no bundles are unloaded after we
                            // receive the exit signal, the registry will never exit.
                            if self.shared_mut().libraries.is_empty() {
                                break; // we already get to exit :)
                            }
                            should_exit = true;
                        }
                        _ => (),
                    }
                },
                Some(plugins) = self.receiver.recv() => {
                    trace!("recieved plugins: {plugins:?}");
                    let mut app = registry.new_app();
                    // FIXME: check for necessary unloads
                    // (when a plugin is updated, for example)
                    for plugin in plugins {
                        // TODO: handle this error
                        // SAFETY: safety must be upheld by the caller of PluginSender::send
                        let _ = unsafe { self.load_plugin_from_file_unchecked(&plugin, &mut app) };
                    }
                    if let Err((registry, _)) = app.add_to_registry(registry).await {
                        for unloaded_bundle in registry.unloaded_bundles() {
                            // FIXME: If only part of a bundle is loaded, the registry gets put
                            // into a terrible situation. It can't just drop the library, since one
                            // of the library's resources could have still been loaded. But it also
                            // can't keep the library, since it will never be unloaded: the
                            // ResourceMap doesn't know that one of the resources was never loaded,
                            // so it'll forever wait for it to unload before unloading the bundle.
                            // Instead, we use the hacky solution: just leak the library and keep
                            // it always loaded.
                            let library = self.shared_mut().libraries.remove(unloaded_bundle);
                            if let Some(library) = library {
                                std::mem::forget(library);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Handle {
    shared: Arc<Mutex<Shared>>,
    sender: tokio::sync::mpsc::UnboundedSender<Vec<Binary>>,
}

impl Handle {
    /// # Safety
    ///
    /// Each of these plugins must adhere to the safety contract specified in
    /// [`PluginRegistry::load_plugin_from_file_unchecked`]
    pub unsafe fn send(&self, plugins: Vec<Binary>) {
        let _ = self.sender.send(plugins);
    }

    fn shared_mut(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared
            .try_lock()
            .expect("registry library lock shouldn't be poisoned")
    }

    #[must_use]
    pub fn is_loaded(&self, plugin: &PluginName) -> bool {
        let shared = self.shared_mut();
        (shared.bundles.get(plugin)).is_some_and(|id| shared.libraries.contains_key(id))
    }
}
