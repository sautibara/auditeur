use std::{collections::HashMap, ffi::OsStr, marker::PhantomData, path::PathBuf};

use auditeur_plugin::auditeur::{
    app::{App, EventReceiver, Registry},
    prelude::*,
    Message,
};
use log::{debug, trace};
use tokio::select;

use crate::Error;
use auditeur_plugin::{loading::Plugin, LibraryHandle};

/// A registry of all of the currently loaded plugins in the [`App`]
///
/// This registry keeps a [`LibraryHandle`] of all loaded plugins, keeping their dynamic libraries
/// loaded. The registry only drops the handle when [`RegistryEvent::BundleUnloaded`] is received
/// for the plugin's bundle, so that the library isn't held for forever.
#[must_use = "if the registry isn't added to an app, the libraries that are loaded will be leaked"]
#[resource("plugin_host:registry")]
#[allow(clippy::module_name_repetitions)] // conflicts with the normal auditeur Registry
pub struct PluginRegistry<M: Message> {
    libraries: HashMap<NamespacedId, LibraryHandle>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<Vec<PathBuf>>,
    message: PhantomData<M>,
}

#[create]
impl<M: Message> Create<M> for PluginRegistry<M> {
    async fn create(#[external] registry: Self) -> Result<Self> {
        Ok(registry)
    }
}

impl<M: Message> PluginRegistry<M> {
    pub fn new() -> (Self, PluginSender) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                libraries: HashMap::new(),
                receiver,
                message: PhantomData,
            },
            PluginSender { sender },
        )
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
        let path = path.as_ref();
        trace!("loading binary at '{}' to app", path.to_string_lossy());
        let plugin = Plugin::load_from_file_unchecked(path, app)?;
        self.libraries.insert(plugin.bundle, plugin.library);
        Ok(self)
    }
}

impl<M: Message> Drop for PluginRegistry<M> {
    fn drop(&mut self) {
        // the registry should only get dropped when it's empty
        if self.libraries.is_empty() {
            return;
        }
        // if the registry isn't empty, then leak the library handles just in case
        #[allow(clippy::collection_is_never_read)]
        let libraries: Vec<_> = self.libraries.drain().map(|(_, library)| library).collect();
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
                            self.libraries.remove(&id);
                            debug!("unloaded bundle `{id}`'s library");
                            // we only should stop if all of the libraries are unloaded first
                            if should_exit && self.libraries.is_empty() {
                                break;
                            }
                        }
                        // NOTE: extra stop signals are ignored
                        RegistryEvent::Exit => {
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
                        let _ = unsafe { self.load_plugin_from_file_unchecked(plugin, &mut app) };
                    }
                    // TODO: handle this error
                    let _ = app.add_to_registry(registry).await;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PluginSender {
    sender: tokio::sync::mpsc::UnboundedSender<Vec<PathBuf>>,
}

impl PluginSender {
    /// # Safety
    ///
    /// Each of these plugins must adhere to the safety contract specified in
    /// [`PluginRegistry::load_plugin_from_file_unchecked`]
    pub unsafe fn send(&self, paths: Vec<PathBuf>) {
        let _ = self.sender.send(paths);
    }
}
