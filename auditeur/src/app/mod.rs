use std::collections::HashMap;

use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use self::constraints::Constraint;

pub mod bundle;
pub mod constraints;
mod loaded;
mod loader;

pub use loaded::LoadedResource;
pub use loaded::Runner;
pub use loader::Loader;

// TODO: separate NamespacedId into ResourceId and BundleId

struct BundledLoader<M: Message> {
    loader: Box<dyn Loader<M>>,
    bundle: NamespacedId,
}

struct BuiltBundle {
    constraints: Constraints,
    parent: Option<NamespacedId>,
}

#[must_use = "An app does nothing unless it is run"]
pub struct App<M: Message> {
    sender: Sender<M>,
    exit_handle: Exiter,
    loaders: HashMap<NamespacedId, Option<BundledLoader<M>>>,
    bundles: HashMap<NamespacedId, Option<BuiltBundle>>,
}

impl<M: Message> App<M> {
    pub fn new(capacity: usize) -> Self {
        let sender = Sender::new(capacity);
        let exit_handle = Exiter::new();
        Self {
            sender,
            exit_handle,
            loaders: HashMap::new(),
            bundles: HashMap::new(),
        }
    }

    pub fn with_bundle(mut self, bundle: impl bundle::Supplier<M>) -> Self {
        self.add_bundle(bundle);
        self
    }

    pub fn add_bundle(&mut self, bundle: impl bundle::Supplier<M>) -> &mut Self {
        let bundle = bundle.build();
        bundle.apply_to(self, None);
        self
    }

    #[must_use]
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> Receiver<M> {
        self.sender.subscribe()
    }

    #[must_use]
    pub fn exiter(&self) -> Exiter {
        self.exit_handle.clone()
    }

    #[must_use]
    pub fn exiter_subscribe(&self) -> Receiver<Exit> {
        self.exit_handle.subscribe()
    }

    fn add_loader(&mut self, loader: Box<dyn Loader<M>>, bundle: NamespacedId) {
        self.loaders.insert(
            loader.ident().into_owned(),
            Some(BundledLoader { loader, bundle }),
        );
    }

    fn add_bundle_inner(
        &mut self,
        id: NamespacedId,
        constraints: Constraints,
        parent: Option<NamespacedId>,
    ) {
        self.bundles.insert(
            id,
            Some(BuiltBundle {
                constraints,
                parent,
            }),
        );
    }

    /// Listen to all messages until [an exit message](Exiter) is sent
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency was not found
    pub async fn run(mut self) -> Result<(), Error> {
        let mut registry = Registry::new(self.sender.clone(), self.exit_handle.clone());
        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for id in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve_loader(&mut registry, &id)?;
        }
        registry.run().await;
        Ok(())
    }

    fn resolve_loader(
        &mut self,
        registry: &mut Registry<M>,
        id: &NamespacedId,
    ) -> Result<(), Error> {
        let loader = self
            .loaders
            .get_mut(id)
            .ok_or_else(|| Error::NotFound { id: id.clone() })?;

        // if it's already finished, then early return
        let Some(BundledLoader { loader, bundle }) = loader.take() else {
            return Ok(());
        };

        // bundles always have to be resolved first
        self.resolve_bundle(registry, &bundle)?;

        // if there are constraints, we must load them first
        self.resolve_constraints(registry, loader.constraints())?;

        // then we can finally load the loader
        let runnable = loader.load(registry)?;
        registry.add_boxed(runnable);
        Ok(())
    }

    fn resolve_bundle(
        &mut self,
        registry: &mut Registry<M>,
        id: &NamespacedId,
    ) -> Result<(), Error> {
        let bundle = self
            .bundles
            .get_mut(id)
            .ok_or_else(|| Error::NotFound { id: id.clone() })?;

        // if it's already finished, then early return
        let Some(BuiltBundle {
            constraints,
            parent,
        }) = bundle.take()
        else {
            return Ok(());
        };

        // the bundle always has to be resolved first
        if let Some(parent) = parent {
            self.resolve_bundle(registry, &parent)?;
        }

        // then the constraints also have to be resolved
        self.resolve_constraints(registry, constraints)?;

        Ok(())
    }
    fn resolve_constraints(
        &mut self,
        registry: &mut Registry<M>,
        constraints: Constraints,
    ) -> Result<(), Error> {
        for constraint in constraints {
            match constraint {
                Constraint::AfterResource { id } => self.resolve_loader(registry, &id)?,
                Constraint::AfterBundle { id } => self.resolve_bundle(registry, &id)?,
            }
        }
        Ok(())
    }
}

/// Messages from the [`Registry`] about different loaded resources
#[non_exhaustive]
#[derive(Clone)]
pub enum RegistryMessage {
    /// `id` has been loaded
    ///
    /// This doesn't necessarily mean that the resource is currently running, only that it will
    /// eventually be run
    ///
    /// Note: this only is emitted for [listeners](crate::resource::Listener) and
    /// [watchers](crate::resource::Watcher), as resources are never loaded
    Loaded { id: NamespacedId },
    /// `id` has exited
    ///
    /// Note: this only is emitted for [listeners](crate::resource::Listener) and
    /// [watchers](crate::resource::Watcher), as resources are never loaded, so they can't exit
    Exited { id: NamespacedId },
    /// `from` accessed `to` through a [`Get<_>`](crate::input::Get) input
    Access {
        from: NamespacedId,
        to: NamespacedId,
    },
    /// Stop `id`, making it exit
    ///
    /// Note: this is automatically handled for both [listeners](crate::resource::Listener) and
    /// [watchers](crate::resource::Watcher)
    Stop { id: NamespacedId },
}

pub struct Stopper {
    sender: Sender<RegistryMessage>,
}

impl Stopper {
    #[inline]
    pub fn stop<R: Resource>(&self) {
        self.stop_id(R::const_ident());
    }

    pub fn stop_id(&self, id: NamespacedIdRef<'_>) {
        let id = id.into_owned();
        let _ = self.sender.send(RegistryMessage::Stop { id });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exit;

#[derive(Clone, Debug)]
pub struct Exiter {
    exiter: tokio::sync::broadcast::Sender<Exit>,
}

impl Exiter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            exiter: tokio::sync::broadcast::Sender::new(1),
        }
    }

    pub fn exit(&self) {
        let _ = self.exiter.send(Exit);
    }

    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Exit> {
        self.exiter.subscribe()
    }
}

impl Default for Exiter {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NotFound;

pub struct Registry<M: Message> {
    sender: Sender<M>,
    meta_sender: Sender<RegistryMessage>,
    exit_handle: Exiter,
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
}

impl<M: Message> Registry<M> {
    fn new(sender: Sender<M>, exit_handle: Exiter) -> Self {
        Self {
            sender,
            meta_sender: Sender::new(256),
            exit_handle,
            resources: HashMap::new(),
        }
    }

    #[must_use]
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> Receiver<M> {
        self.sender.subscribe()
    }

    #[must_use]
    pub fn exiter(&self) -> Exiter {
        self.exit_handle.clone()
    }

    #[must_use]
    pub(crate) fn exit_signal(&self) -> Receiver<Exit> {
        self.exit_handle.subscribe()
    }

    #[must_use]
    pub fn meta_receiver(&self) -> Receiver<RegistryMessage> {
        self.meta_sender.subscribe()
    }

    #[must_use]
    pub(crate) fn meta_sender(&self) -> Sender<RegistryMessage> {
        self.meta_sender.clone()
    }

    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper {
            sender: self.meta_sender(),
        }
    }

    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<&R> {
        let id = R::const_ident().into();
        let loaded = self.resources.get(&id)?;
        loaded.resource().downcast_ref()
    }

    pub fn add_boxed(&mut self, runner: Box<dyn LoadedResource>) -> &mut Self {
        self.resources.insert(runner.ident().into(), runner);
        self
    }

    async fn run(self) {
        let futures = self
            .resources
            .into_values()
            .filter_map(LoadedResource::runner)
            .map(Runner::run);
        futures::future::join_all(futures).await;
    }
}

// PluginRegistry
// /// this only holds the listeners and watchers that are currently running
// /// they're the roots of what stays loaded
// - running: HashMap<NamespacedId, Arc<ResourceInfo>>
// /// this holds everything, including resources that are currently running and resources that are
// /// unloaded
// - loaded: HashMap<NamespacedId, Weak<ResourceInfo>>
//
// ResourceInfo
// - id: NamespacedId
// - type: ResourceType
// - library: Option<PluginLibrary>
// - dependencies: Vec<Arc<ResourceInfo>>
// - dependents: Vec<NamespacedId>
//
// ResourceType
// - Resource
// - Plugin
//
// This automatically allows for keeping alive both plugins and resources in a safe manner, and
// provides information that is necessary to unloading plugins. (why does this writing seem like
// I'm writing for the HR of a multinational corporation it's so dry)
//
// When creating the registry, all resources have to stay loaded in an Arc
// When running, resources and plugins that aren't currently running should be stored in a Weak, so
// that they aren't kept alive even though they shouldn't be
//
// Plugins will be stored in resources' dependencies (the plugin's code is a dependency for the
// resource), and as such you can see the resources in a plugin by viewing its dependents.

// building plugins
//
// To build a plugin, you either provide a closure or function that takes in a plugin builder and
// returns a plugin declaration.
//
// App
// - add_plugin(fn(PluginBuilder<NoId>) -> PluginBuilder<HasId>)
//
// PluginBuilder<NoId>
// - id
//
// PluginBuilder<HasId>
// - add_listener
// - add_watcher
// - add_resource
// - add_plugin
// - load_after(id)

// how to implement?
//
// Rename "plugins" to "bundles" - they're just groups of resources
//
// The main app essentially becomes its own bundle, so enforce that
// - Only have a single add_bundle method in the app
// - Only bundles can add loaders, not the main app
// - Only the app can be used to run the... app
//
// This has a few interesting effects
// - *All* listeners will have a parent bundle
// - As such, listeners can be always paired with a parent bundle, no options needed
//
// To model bundles in the actual app,
// - There will be two HashMaps: loaders and bundles
// - Everything will be put into the same HashMaps, in the main app
// - Loaders will always be paired with a bundle
// - Bundles may also be paired with other bundles
// - Bundles may have their own constraints, and child loaders will have to finish those first
