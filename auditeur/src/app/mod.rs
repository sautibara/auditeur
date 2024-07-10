use std::collections::HashMap;

use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

mod bundle;
mod constraints;
mod loaded;
mod loader;

pub use bundle::Bundle;
pub use bundle::Supplier as BundleSupplier;

pub use loaded::LoadedResource;
pub use loaded::Runner;
pub use loader::Loader;

pub use constraints::Constraint;
pub use constraints::Constraints;

// TODO: separate NamespacedId into ResourceId and BundleId

struct BundledLoader<M: Message> {
    loader: Box<dyn Loader<M>>,
    bundle: NamespacedId,
}

struct BuiltBundle {
    constraints: Constraints,
    parent: Option<NamespacedId>,
}

/// The main entry-point for the application: the thing that runs resources
///
/// The app handles loading [resources](crate::resource::Resource),
/// [listeners](crate::resource::Listener), and [watchers](crate::resource::Watcher), giving them
/// the correct [inputs](crate::input::Input) (including [each other](crate::input::Get)), and
/// [running them](Self::run) asynchronously. It also makes sure they are loaded in the correct
/// order, through [constraints](Constraints), allowing them to have other dependencies.
///
/// The main container for resources is the [`Bundle`], and all resources that are added to an
/// `App` must be contained within one. As such, only bundles are allowed to be added to the main
/// app builder, although resources may be added to each bundles.
#[must_use = "An app does nothing unless it is run"]
pub struct App<M: Message> {
    capacity: usize,
    loaders: HashMap<NamespacedId, Option<BundledLoader<M>>>,
    bundles: HashMap<NamespacedId, Option<BuiltBundle>>,
}

impl<M: Message> App<M> {
    /// Create a new app with a specified `capacity` of messages for each channel
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            loaders: HashMap::new(),
            bundles: HashMap::new(),
        }
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    ///
    /// This takes ownership of the [`App`], allowing for usage closer to a builder pattern
    pub fn with_bundle(mut self, bundle: impl bundle::Supplier<M>) -> Self {
        self.add_bundle(bundle);
        self
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    pub fn add_bundle(&mut self, bundle: impl bundle::Supplier<M>) -> &mut Self {
        let bundle = bundle.build();
        bundle.apply_to(self, None);
        self
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

    /// [Build](Self::build) the `App` and run all [runners](Runner) to completion
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run(self) -> Result<(), Error> {
        let registry = self.build()?;
        registry.run().await;
        Ok(())
    }

    /// Build the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub fn build(mut self) -> Result<Registry<M>, Error> {
        let mut registry = Registry::new(self.capacity);
        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for id in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve_loader(&mut registry, &id)?;
        }
        Ok(registry)
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

/// A list of registered and loaded [resources](LoadedResource), built from an [`App`], that can be run as a main loop
///
/// A registry is built from an [`App`] declaration, which handles loading things in the correct
/// order. This allows resources to access other resources through [`Self::get`], or, much more
/// commonly, [`Get<_>`](crate::input::Get). It is the main thing that [inputs](crate::input)
/// interact with, allowing resources to automatically get a large amount of information about the
/// app. As such, most channels shown below are also able to be used as inputs (they implement
/// [`Input`]).
///
/// There are many bundled channels for notification passing:
/// - To tell all runners to exit at the same time, use an [`Exiter`] through [`Self::exiter`]
/// - To tell only one resource to exit, use a [`Stopper`] through [`Self::stopper`]
/// - To recieve notifications about what happens to the registry, use [`Self::meta_receiver`]
/// - To listen to messages, use [`Self::message_reciever`]
pub struct Registry<M: Message> {
    message_sender: Sender<M>,
    meta_sender: Sender<RegistryEvent>,
    exit_handle: Exiter,
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
}

impl<M: Message> Registry<M> {
    fn new(capacity: usize) -> Self {
        Self {
            message_sender: Sender::new(capacity),
            meta_sender: Sender::new(capacity),
            exit_handle: Exiter::new(),
            resources: HashMap::new(),
        }
    }

    /// Asynchronously run each [`Runner`] that has been registered, until they all exit
    pub async fn run(self) {
        let futures = self
            .resources
            .into_values()
            .filter_map(LoadedResource::runner)
            .map(Runner::run);
        futures::future::join_all(futures).await;
    }

    /// Add an already-loaded resource to the registry. The resource can be accessed through
    /// [`Self::get`], and if it provides a [`Runner`], that will be run in the main loop.
    pub fn add_boxed(&mut self, runner: Box<dyn LoadedResource>) -> &mut Self {
        self.resources.insert(runner.ident().into(), runner);
        self
    }

    /// Get a reference to a loaded resource. This uses [`Any`](std::any::Any) in order to get the
    /// contained object if it exists. If the object is cloneable, that can also be used.
    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<&R> {
        let id = R::const_ident().into();
        let loaded = self.resources.get(&id)?;
        loaded.resource().downcast_ref()
    }
}

/// These are methods to access the various channels that the registry contains
impl<M: Message> Registry<M> {
    #[must_use]
    pub fn message_sender(&self) -> Sender<M> {
        self.message_sender.clone()
    }

    #[must_use]
    pub fn message_reciever(&self) -> Receiver<M> {
        self.message_sender.subscribe()
    }

    #[must_use]
    pub fn exiter(&self) -> Exiter {
        self.exit_handle.clone()
    }

    #[must_use]
    pub fn exit_reciever(&self) -> Receiver<Exit> {
        self.exit_handle.subscribe()
    }

    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper {
            sender: self.meta_sender(),
        }
    }

    #[must_use]
    pub(crate) fn meta_sender(&self) -> Sender<RegistryEvent> {
        self.meta_sender.clone()
    }

    #[must_use]
    pub fn meta_receiver(&self) -> Receiver<RegistryEvent> {
        self.meta_sender.subscribe()
    }
}

/// Messages from the [`Registry`] about different events, such as loading or unloading resources
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RegistryEvent {
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

/// A handle on a [`Registry`] that can be used to stop one single resource
#[derive(Clone, Debug)]
pub struct Stopper {
    sender: Sender<RegistryEvent>,
}

impl Stopper {
    /// Stop the resource `R`, if it exists
    #[inline]
    pub fn stop<R: Resource>(&self) {
        self.stop_id(R::const_ident());
    }

    /// Stop the resource with id `id`, if it exists
    pub fn stop_id(&self, id: NamespacedIdRef<'_>) {
        let id = id.into_owned();
        let _ = self.sender.send(RegistryEvent::Stop { id });
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exit;

/// A handle on a [`Registry`] that can be used to exit the entire program
#[derive(Clone, Debug)]
pub struct Exiter {
    exiter: Sender<Exit>,
}

impl Exiter {
    #[must_use]
    fn new() -> Self {
        Self {
            exiter: Sender::new(1),
        }
    }

    /// Tell all resources in the [`Registry`] to exit
    pub fn exit(&self) {
        let _ = self.exiter.send(Exit);
    }

    #[must_use]
    pub fn subscribe(&self) -> Receiver<Exit> {
        self.exiter.subscribe()
    }
}

impl Default for Exiter {
    fn default() -> Self {
        Self::new()
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
