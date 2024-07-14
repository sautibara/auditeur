//! The main entry-point for the application: loading and running [resources](mod@crate::resource)
//!
//! The main [`App`] handles loading [resources](crate::resource::Resource),
//! [listeners](crate::resource::Listener), and [watchers](crate::resource::Watcher) from
//! [bundles](Bundle) given by the user.
//!
//! # The Loading Process
//!
//! Note: This is a very technical explanation, and none of it is needed to be known to effectively
//! use this crate. However, it may help with understanding certain things - especially loaders.
//!
//! - Before being added to an [`App`], resources are each wrapped in a [`Loader`]
//!     - Loaders are trait objects that handle resolving [`Input`]s and building
//!     a resource into a [`LoadedResource`]
//! - When the app begins being built, a [`Registry`] is created to hold each [`LoadedResource`]
//! - Each [`Loader`] is loaded into the registry based on its [`Constraints`]
//!     - [`Constraints`] are necessary so that resources can ensure that the other resources they
//!     access through [`Get<_>`](crate::input::Get) are already loaded
//! - Once loaded, resources are each built into a [`LoadedResource`]
//!     - [`LoadedResource`]s allow for dynamically accessing the contained resource, which enables
//!     [`Registry::get`] and thus [`input::Get<_>`](crate::input::Get) to work
//!     - If the resource calls for any [`Input`]s, the [`LoadedResource`] contains them
//!     - Then, [`LoadedResource`]s may also provide a [`Runner`] to run in the main loop, which is
//!     how listeners and watchers are run
//! - When the registry is [run](Registry::run), each [`Runner`] is
//! [joined](futures::future::join_all) until they all finish
//!
//! The basic pipeline: [`impl Resource`](Resource) / [`impl Listener`](Listener) /
//! [`impl Watcher`](Watcher) -> [`dyn Loader`](Loader) -> [`dyn LoadedResource`](LoadedResource)
//! -> (optionally) [`dyn Runner`](Runner)

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::broadcast::Receiver;

use crate::bundle;
use crate::prelude::*;

mod constraints;
pub(crate) mod loaded;
pub(crate) mod loader;

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

    pub(crate) fn add_loader(&mut self, loader: Box<dyn Loader<M>>, bundle: NamespacedId) {
        self.loaders.insert(
            loader.ident().into_owned(),
            Some(BundledLoader { loader, bundle }),
        );
    }

    pub(crate) fn add_bundle_inner(
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
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
}

impl<M: Message> Registry<M> {
    fn new(capacity: usize) -> Self {
        Self {
            message_sender: Sender::new(capacity),
            meta_sender: Sender::new(capacity),
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
        Exiter {
            sender: self.meta_sender(),
        }
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

impl<M: Message> Input<M> for Sender<M> {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.message_sender())
    }
}

/// Messages from the [`Registry`] about different events, such as loading or unloading resources
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RegistryEvent {
    /// Stop all [`Runner`]s, making the entire [`App`] exit
    Exit,
    /// Stop `id`, making it exit
    ///
    /// Note: this is automatically handled for both [listeners](crate::resource::Listener) and
    /// [watchers](crate::resource::Watcher), but custom [`Runner`]s will have to listen to this
    /// message themselves.
    Stop { id: NamespacedId },
    /// `id` has been loaded into the main [`Registry`]
    ///
    /// Note: All [resources](mod@crate::resource) will emit this message, even if they won't
    /// provide a [`Runner`] - see [`Self::Running`] instead, if that information is needed.
    Loaded { id: NamespacedId },
    /// `id` has started running
    ///
    /// Note: this only is emitted for resources that have [`Runner`]s, like
    /// [listeners](crate::resource::Listener) and [watchers](crate::resource::Watcher). As simple
    /// [Resources](crate::resource::Resource) are never run, they never emit this message.
    Running { id: NamespacedId },
    /// `id` has stopped running
    ///
    /// Note: this only is emitted for resources that are [`Runner`]s, like
    /// [listeners](crate::resource::Listener) and [watchers](crate::resource::Watcher). As simple
    /// [Resources](crate::resource::Resource) are never run, they never emit this message - see
    /// [`Self::Exited`] instead, if that information is needed.
    Stopped { id: NamespacedId },
    /// `id` has been unloaded from the app: it has stopped if it ever started running, and all
    /// [references](crate::input::Get) of it from other [resources](mod@crate::resource) have been
    /// dropped
    ///
    /// Note: this will only ever be sent if a [`ResourceMap`] is currently running
    Unloaded { id: NamespacedId },
    /// A bundle has been loaded into the registry
    BundleLoaded {
        id: NamespacedId,
        resources: Arc<[NamespacedId]>,
        bundles: Arc<[NamespacedId]>,
    },
    /// The last child of the bundle represented by `id` has unloaded, unloading the bundle
    ///
    /// Note: this will only ever be sent if a [`ResourceMap`] is currently running
    BundleExited { id: NamespacedId },
    /// The resource `from` referenced the resource `to` through a [`Get<_>`](crate::input::Get)
    /// [input](crate::input::Input)
    Reference {
        from: NamespacedId,
        to: NamespacedId,
    },
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

impl<M: Message> Input<M> for Stopper {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.stopper())
    }
}

/// A handle on a [`Registry`] that can be used to exit the entire program
#[derive(Clone, Debug)]
pub struct Exiter {
    sender: Sender<RegistryEvent>,
}

impl Exiter {
    /// Tell all resources in the [`Registry`] to exit
    pub fn exit(&self) {
        let _ = self.sender.send(RegistryEvent::Exit);
    }
}

impl<M: Message> Input<M> for Exiter {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.exiter())
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

// What about when the Registry starts running?
//
// In the Registry, hold an enum between a LoadingRegistry and a Handle
// - These will figure out how to implement methods such as get<R> and add_boxed
// - Methods concerning channels can be shared - the channels will be stored alongside the enums
//
// Why can't it just be one trait and the trait objects take in an impl Trait?
// - Simple: trait objects can't take in impl Trait types
//
// LoadingRegistry
// - HashMap<NamespacedId, Box<dyn LoadedResource>>,
// - broadcast::Sender<Box<dyn LoadedResource>>,
//
// Handle
// - broadcast::Sender<Box<dyn LoadedResource>>
// - broadcast::Sender<RegistryEvent>?
//
// Handle can actually become an Input returning a full Registry handle

// How should the plugin system be applied to an App?
//
// First off, the plugin system has to be applied to the App, not the Registry
// - The plugin system has to utilize the App to make things load in the right order
// - Although, it'll also have to emulate it when it's adding bundles from plugins loaded after the
//   registry has already started running
//     - Maybe put an add_bundle method on a registry handle?
//
// So we'll have to make the plugin system have a function that takes in a &mut App, adds the
// plugins that are meant to be loaded, and adds the main plugin system bundle
//
// This could be wrapped in an extension trait, making the app have something like an
// add_plugin_system(config) function
//
// ```
// App::new(1024)
//     .with_bundle(...)
//     .with_plugin_system(...)
//     .run()
// ```

// ResourceMap redo
//
// There's one big question about Handles: how about Resources?
// - So if we're going to allow users to get resources through Handles, normal Resources need to
//   stay alive for a while
// - This means that we now need to find out when specific resources are needed and unneeded based
//   on which runners are using them... exactly what we need to do for plugins!
// - Because of this, we could use the same system for both of them
//
// How do we link the auditeur ResourceMap to auditeur_plugin_host?
// - We could simply extend the resource idea to bundles, telling listeners when bundles are loaded
//   and unloaded
//
// How to only add it when it's needed?
// - If the Handle system is only added when it's needed, it could lower complexity.
// - It can be added only when a Handle is actually cloned, which could be detected through an
//   Arc-like thing (through its strong_count)
// - If no Handles are created, the entire system can be ignored
// - This is fine because then there's no chance of a Handle ever being needed again: since no new
//   resources can be added (since there is no Handle), none will ask for a new Handle in the
//   future
