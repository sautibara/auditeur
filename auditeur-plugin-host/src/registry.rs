use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use auditeur_plugin::auditeur::{
    app::{App, EventReceiver, Registry},
    prelude::*,
    Message,
};
use log::{debug, trace};
use tokio::select;

use crate::{source::Binary, Error, PluginName};
use auditeur_plugin::{loading::Plugin, LibraryHandle};

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
pub struct PluginRegistry<M: Message> {
    shared: Arc<Mutex<Shared>>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<Vec<Binary>>,
    message: PhantomData<M>,
}

#[create]
impl<M: Message> Create<M> for PluginRegistry<M> {
    async fn create(#[external] registry: Self) -> Result<Self> {
        Ok(registry)
    }
}

impl<M: Message> PluginRegistry<M> {
    pub fn new() -> (Self, Handle<M>) {
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
                message: PhantomData,
            },
            Handle {
                sender,
                shared,
                message: PhantomData,
            },
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
        app: &mut App<M>,
    ) -> Result<NamespacedId, Error> {
        trace!("loading binary at '{}' to app", library.path.display());
        let plugin = Plugin::load_from_file_unchecked(&library.path, app)?;
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

impl<M: Message> Drop for PluginRegistry<M> {
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

#[async_trait]
impl<M: Message> Watcher<M> for PluginRegistry<M> {
    type Input = (EventReceiver, Registry<M>, Stopper);

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
                            if libraries.remove(&id).is_some() {
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
                    if let Err(registry) = app.add_to_registry(registry).await {
                        for unloaded_bundle in registry.unloaded_bundles() {
                            // FIXME: If only part of a bundle is loaded, the registry gets put
                            // into a terrible situation. It can't just drop the library, since one
                            // of the library's resources could have still been loaded. But it also
                            // can't keep the library, since it will never be unloaded: the
                            // ResourceMap doesn't know that one of the resources was never loaded,
                            // so it'll forever wait for it to unload before unloading the bundle.
                            // Instead, we use the hacky solution: just leak the library and keep
                            // it always loaded.
                            if let Some(library) = self.shared_mut().libraries.remove(unloaded_bundle) {
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
pub struct Handle<M: Message> {
    shared: Arc<Mutex<Shared>>,
    sender: tokio::sync::mpsc::UnboundedSender<Vec<Binary>>,
    message: PhantomData<M>,
}

impl<M: Message> Handle<M> {
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
