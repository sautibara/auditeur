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
//!       a resource into a [`LoadedResource`]
//! - When the app begins being built, a [`Registry`] is created to hold each [`LoadedResource`]
//! - Each [`Loader`] is loaded into the registry based on its [`Constraints`]
//!     - [`Constraints`] are essentially a list of resources that must be loaded before loading
//!       another resource
//!     - These are necessary so that resources can ensure that the other resources they access
//!       through [`Get<_>`](crate::input::Get) are already loaded
//! - Once loaded, resources are each built into a [`LoadedResource`]
//!     - [`LoadedResource`]s build the [`Input`]s for their resources from the registry
//!     - [`LoadedResource`]s allow for dynamically accessing the contained resource by the
//!       registry, enabling [`input::Get<_>`](crate::input::Get) to work
//!     - Then, [`LoadedResource`]s may also provide a [`Runner`] to run in the main loop, which is
//!       how listeners and watchers are run
//! - When the registry is [run](Registry::run), each [`Runner`] is asynchronously run at the same
//!   time until they all finish
//!
//! The basic pipeline: [`impl Resource`](Resource) / [`impl Listener`](Listener) /
//! [`impl Watcher`](Watcher) -> [`dyn Loader`](Loader) -> [`dyn LoadedResource`](LoadedResource)
//! -> (optionally) [`dyn Runner`](Runner)

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use async_recursion::async_recursion;
use dashmap::mapref::one::MappedRef;
use dashmap::DashMap;
use tokio::sync::broadcast;
use tokio::sync::watch;
#[cfg(feature = "runtime-loading")]
use tokio::sync::{mpsc, oneshot};
#[cfg(feature = "runtime-loading")]
use tracing::error;
use tracing::info_span;
use tracing::instrument;
use tracing::trace;
use tracing::warn;
use tracing::Instrument;

use crate::bundle;
use crate::input::get::GetRef;
use crate::prelude::*;

mod constraints;
pub(crate) mod loaded;
pub(crate) mod loader;

#[cfg(feature = "resource-map")]
pub mod resource_map;
#[cfg(feature = "resource-map")]
pub use resource_map::ResourceMap;

pub use loaded::LoadedResource;
pub use loaded::Runner;
pub use loader::Loader;

pub use constraints::Constraint;
pub use constraints::Constraints;

// TODO: separate NamespacedId into ResourceId and BundleId

struct BundledLoader {
    loader: Box<dyn Loader>,
    bundle: NamespacedId,
}

pub(crate) struct BuiltBundle {
    pub constraints: Constraints,
    pub parent: Option<NamespacedId>,
    pub resources: Arc<[NamespacedId]>,
    pub bundles: Arc<[NamespacedId]>,
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
/// `App` must be contained within one. As such, all resources added to an app must be added
/// through a bundle. If there are conflicts between resources with the same id, the one that is
/// added first is kept.
#[must_use = "An app does nothing unless it is run"]
pub struct App {
    loaders: HashMap<NamespacedId, Option<BundledLoader>>,
    bundles: HashMap<NamespacedId, Option<BuiltBundle>>,
    loading_loaders: HashMap<NamespacedId, NamespacedId>,
    // TODO: check if adding App events is a good idea
    events: Vec<RegistryEvent>,
}

impl App {
    /// Create a new app
    #[cfg_attr(
        not(feature = "resource-map"),
        allow(clippy::let_and_return, unused_mut)
    )]
    pub fn new() -> Self {
        let mut this = Self::new_empty();
        #[cfg(feature = "resource-map")]
        this.add_bundle(new_bundle!("auditeur:resource_map").with_watcher::<ResourceMap>(()));
        this
    }

    fn new_empty() -> Self {
        Self {
            loaders: HashMap::new(),
            bundles: HashMap::new(),
            loading_loaders: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    ///
    /// This takes ownership of the [`App`], allowing for usage closer to a builder pattern
    pub fn with_bundle(mut self, bundle: impl bundle::Supplier) -> Self {
        self.add_bundle(bundle);
        self
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    pub fn add_bundle(&mut self, bundle: impl bundle::Supplier) -> &mut Self {
        let bundle = bundle.build();
        bundle.apply_to(self, None);
        self
    }

    pub(crate) fn add_loader(
        &mut self,
        loader: Box<dyn Loader>,
        bundle: &NamespacedIdRef,
    ) -> NamespacedId {
        let id = loader.ident().to_owned();
        // prefer the eldest loader
        self.loaders
            .entry(id.clone())
            .or_insert(Some(BundledLoader {
                loader,
                bundle: bundle.to_owned(),
            }));
        id
    }

    pub(crate) fn add_bundle_inner(&mut self, id: NamespacedId, bundle: BuiltBundle) {
        self.bundles.insert(id, Some(bundle));
    }

    /// [Build](Self::build) the `App` and run all [runners](Runner) to completion
    ///
    /// - `capacity`: the capacity of the message channel - how many messages the channel
    ///   can hold before listeners start to lag
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run(self, capacity: usize) -> Result<(), Self> {
        let registry = self.build(capacity).await?;
        registry.run().await;
        Ok(())
    }

    /// Build the `App` into a [`Registry`], without running it
    ///
    /// - `capacity`: the capacity of the message channel - how many messages the channel
    ///   can hold before listeners start to lag
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn build(self, capacity: usize) -> Result<Registry, Self> {
        let mut registry = Registry::new(capacity);
        self.add_to_registry(&mut registry).await?;
        Ok(registry)
    }

    /// Add the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn add_to_registry(mut self, registry: &mut Registry) -> Result<(), Self> {
        self.add_to_registry_fallible(registry)
            .await
            .map_err(|err| {
                registry.event_sender.send_error(
                    ident!("auditeur:registry").to_owned(),
                    anyhow::Error::from(err).context("registry failed to build"),
                );
                self
            })
    }

    async fn add_to_registry_fallible(&mut self, registry: &mut Registry) -> Result<(), Error> {
        // add the loaders and bundles that have already been loaded in the registry to our list
        // this is necessary so that constraints can know that their dependencies have already been
        // loaded
        self.loaders
            .extend(registry.loaded_resources().into_iter().map(|id| (id, None)));
        self.bundles
            .extend(registry.loaded_bundles().into_iter().map(|id| (id, None)));
        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for loader in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve_loader(registry, &loader).await?;
        }
        // free events :)
        registry.event_sender_ref().extend(self.events.drain(..));
        Ok(())
    }

    #[async_recursion]
    async fn resolve_loader(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        let loader = self
            .loaders
            .get_mut(id)
            .ok_or_else(|| Error::LoaderNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(BundledLoader { loader, bundle }) = loader.take() else {
            return Ok(());
        };
        // show that this loader is currently being loaded
        // TODO: use this to prevent dependency loops
        self.loading_loaders.insert(id.to_owned(), bundle.clone());
        // bundles always have to be resolved first
        // this doesn't cause a loop because resolve_bundle already [`Option::take`]s the bundle
        // out of the list, so it can't be loaded again
        self.resolve_bundle(registry, &bundle).await?;
        // if there are constraints, we must load them first
        self.resolve_constraints(registry, &loader.constraints())
            .await?;
        // then we can finally load the loader
        registry.add_loader(loader, bundle).await?;
        self.loading_loaders.remove(id);
        Ok(())
    }

    #[async_recursion]
    async fn resolve_bundle(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        let bundle = self
            .bundles
            .get_mut(id)
            .ok_or_else(|| Error::BundleNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(bundle) = bundle.take() else {
            return Ok(());
        };
        // parent bundles always have to be resolved first
        if let Some(parent) = &bundle.parent {
            self.resolve_bundle(registry, parent).await?;
        }
        // then the constraints also have to be resolved
        self.resolve_constraints(registry, &bundle.constraints)
            .await?;
        // then the bundle can finally be added to the registry
        registry.add_bundle(id.to_owned(), bundle);
        Ok(())
    }

    async fn resolve_constraints(
        &mut self,
        registry: &mut Registry,
        constraints: &Constraints,
    ) -> Result<(), Error> {
        for constraint in constraints {
            match constraint {
                Constraint::AfterResource { id } => self.resolve_loader(registry, id).await?,
                Constraint::AfterBundle { id } => self.resolve_bundle(registry, id).await?,
            }
        }
        Ok(())
    }

    pub fn add_error(&mut self, source: NamespacedId, error: anyhow::Error) {
        self.events.push(RegistryEvent::Error(ErrorEvent {
            source,
            error: Arc::new(error),
        }));
    }

    pub fn add_errors<I: IntoIterator<Item = anyhow::Error>>(
        &mut self,
        source: &NamespacedIdRef,
        errors: I,
    ) {
        self.events.extend(errors.into_iter().map(|error| {
            RegistryEvent::Error(ErrorEvent {
                source: source.to_owned(),
                error: Arc::new(error),
            })
        }));
    }

    // TODO: documentation to clarify name
    pub fn unloaded_loaders(&self) -> impl Iterator<Item = &NamespacedIdRef> {
        self.loading_loaders.keys().map(Deref::deref)
    }

    #[must_use]
    pub fn unloaded_bundles(&self) -> HashSet<&NamespacedIdRef> {
        self.loading_loaders.values().map(Deref::deref).collect()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("bundles", &self.bundles.keys())
            .field("loaders", &self.loaders.keys())
            .field("events", &self.events)
            .field("loading_loaders", &self.loading_loaders)
            .finish()
    }
}

/// The underlying state of a [`Registry`]. This separates the main underlying functionality of the
/// registry into two states: building and running. If the `runtime-loading` feature isn't enabled,
/// only the building state is compiled, as we can never access a registry after it starts running.
/// If the feature is enabled, both states are compiled, and channels are used to talk between the
/// running registry and its handles.
///
/// The "main underlying functionality" is in practice [adding resources](Self::add_boxed) and
/// [getting handles to already-added resources](Self::get_as)
enum RegistryState {
    Building(BuildingRegistry),
    #[cfg(feature = "runtime-loading")]
    Running(RunningRegistry),
}

impl RegistryState {
    /// Returns `true` if the registry state is [`Running`].
    ///
    /// [`Running`]: RegistryState::Running
    #[must_use]
    #[cfg(feature = "runtime-loading")]
    const fn is_running(&self) -> bool {
        matches!(self, Self::Running(..))
    }

    #[cfg_attr(not(feature = "runtime-loading"), allow(clippy::unused_async))]
    #[inline]
    async fn add_boxed_resource(&mut self, resource: Box<dyn LoadedResource>) {
        match self {
            Self::Building(registry) => registry.add_boxed_resource(resource),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.add_boxed_resource(resource).await,
        }
    }

    #[inline]
    fn add_bundle(&mut self, bundle: NamespacedId) {
        match self {
            Self::Building(registry) => registry.add_bundle(bundle),
            #[cfg(feature = "runtime-loading")]
            Self::Running(_) => (),
        }
    }

    #[must_use]
    #[cfg_attr(not(feature = "runtime-loading"), allow(clippy::unused_async))]
    #[inline]
    async fn get_as<R: Resource + GetRef>(
        &self,
        reference_holder: &NamespacedIdRef,
        event_sender: &EventSender,
    ) -> Option<R::Ref> {
        match self {
            Self::Building(registry) => registry.get_as::<R>(reference_holder, event_sender),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.get_as::<R>(reference_holder, event_sender).await,
        }
    }

    #[inline]
    fn loaded_resources(&self) -> HashSet<NamespacedId> {
        match self {
            Self::Building(registry) => registry.loaded_resources(),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.loaded_resources(),
        }
    }

    #[inline]
    fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        match self {
            Self::Building(registry) => registry.loaded_bundles(),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.loaded_bundles(),
        }
    }
}

struct BuildingRegistry {
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
    bundles: HashSet<NamespacedId>,
    #[cfg(feature = "runtime-loading")]
    resource_sender: mpsc::Sender<Box<dyn LoadedResource>>,
    #[cfg(feature = "runtime-loading")]
    resource_receiver: mpsc::Receiver<Box<dyn LoadedResource>>,
}

impl BuildingRegistry {
    fn new() -> Self {
        #[cfg(feature = "runtime-loading")]
        let (resource_sender, resource_receiver) = mpsc::channel(64);

        Self {
            resources: HashMap::new(),
            bundles: HashSet::new(),
            #[cfg(feature = "runtime-loading")]
            resource_sender,
            #[cfg(feature = "runtime-loading")]
            resource_receiver,
        }
    }

    fn add_boxed_resource(&mut self, resource: Box<dyn LoadedResource>) {
        self.resources.insert(resource.ident().into(), resource);
    }

    fn add_bundle(&mut self, bundle: NamespacedId) {
        self.bundles.insert(bundle);
    }

    #[must_use]
    #[instrument(skip_all, fields(from = %reference_holder, to = %R::const_ident()))]
    fn get_as<R: Resource + GetRef>(
        &self,
        reference_holder: &NamespacedIdRef,
        event_sender: &EventSender,
    ) -> Option<R::Ref> {
        let id = R::const_ident().into();
        let loaded = self.resources.get(&id)?;
        let reference = loaded
            .resource_any()
            .downcast_ref::<R>()
            .map(GetRef::get_ref);
        // only send the event if the reference actually succeeded
        if reference.is_some() {
            event_sender.send(RegistryEvent::Reference {
                by: reference_holder.into(),
                to: id,
            });
        } else {
            warn!("attempt to downcast {} failed", id);
        }
        reference
    }

    fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.resources.keys().cloned().collect()
    }

    fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.bundles.clone()
    }
}

#[cfg(feature = "runtime-loading")]
struct RunningRegistry {
    resource_sender: mpsc::Sender<Box<dyn LoadedResource>>,
    map: resource_map::Handle,
}

#[cfg(feature = "runtime-loading")]
impl RunningRegistry {
    async fn add_boxed_resource(&self, resource: Box<dyn LoadedResource>) {
        let _ = self.resource_sender.send(resource).await;
    }

    #[must_use]
    #[instrument(skip_all, fields(from = %reference_holder, to = %R::const_ident()))]
    async fn get_as<R: Resource + GetRef>(
        &self,
        reference_holder: &NamespacedIdRef,
        event_sender: &EventSender,
    ) -> Option<R::Ref> {
        let id = R::const_ident().to_owned();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let get_request = GetRequest {
            id: id.clone(),
            sender: tokio::sync::Mutex::new(Some(sender)),
        };
        // FIXME: by using an EventSender, the requests actually never get dropped
        // - it all stays in the main Vec, awaiting new receivers
        // this should be separated into another channel for only Get requests
        event_sender.send(RegistryEvent::Get(Arc::new(get_request)));
        // if the request if fulfilled, this will return Some
        // if it isn't and all requests get dropped, the channel drops and returns a RecvError
        let reference = tokio::time::timeout(Duration::from_millis(100), receiver)
            .await
            .ok()?
            .ok()?;
        // let reference = receiver.await.ok()?;
        trace!("succeeded getting {id}");
        // and then, the actual reference gets downcasted
        let Ok(reference) = crate::downcast(reference.as_any()) else {
            // downcasting shouldn't ever fail, so this is an important tracing
            error!("attempt to downcast {} failed", id);
            return None;
        };
        // the reference worked! we can send the message now
        event_sender.send(RegistryEvent::Reference {
            by: reference_holder.into(),
            to: id,
        });
        Some(*reference)
    }

    fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.map.loaded_resources()
    }

    fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.map.loaded_bundles()
    }
}

struct Channels {
    capacity: usize,
    map: DashMap<NamespacedId, Box<dyn crate::Any + Send + Sync>>,
}

impl Channels {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            map: DashMap::new(),
        }
    }

    fn sender_ref<M: Message>(
        &self,
    ) -> MappedRef<'_, NamespacedId, Box<dyn crate::Any + Send + Sync>, broadcast::Sender<M>> {
        self.map
            .entry(M::const_ident().to_owned())
            .or_insert_with(|| Box::new(broadcast::Sender::<M>::new(self.capacity)))
            .downgrade()
            .map(|val| {
                val.downcast_ref()
                    .expect("this should always be a valid Sender")
            })
    }

    fn sender<M: Message>(&self) -> broadcast::Sender<M> {
        self.sender_ref().clone()
    }

    fn receiver<M: Message>(&self) -> broadcast::Receiver<M> {
        self.sender_ref().subscribe()
    }
}

/// A list of registered and loaded [resources](LoadedResource), built from an [`App`], that can be run as a main loop
///
/// A registry is built from an [`App`] declaration, which handles loading things in the correct
/// order. This allows resources to access other resources through [`Self::get_as`], or, much more
/// commonly, [`Get<_>`](crate::input::Get). It is the main thing that [inputs](crate::input)
/// interact with, allowing resources to automatically get a large amount of information about the
/// app. As such, most channels shown below are also able to be used as inputs (they implement
/// [`Input`]).
///
/// There are many bundled channels for notification passing:
/// - To tell only one resource or the entire program to stop, use a [`Stopper`] through [`Self::stopper`]
/// - To receive notifications about what happens to the registry, use [`Self::event_receiver`]
/// - To listen to messages, use [`Self::message_receiver`]
pub struct Registry {
    channels: Arc<Channels>,
    event_sender: EventSender,
    state: RegistryState,
}

impl Registry {
    fn new(capacity: usize) -> Self {
        Self {
            channels: Arc::new(Channels::new(capacity)),
            event_sender: EventSender::new(),
            state: RegistryState::Building(BuildingRegistry::new()),
        }
    }

    /// Asynchronously run each [`Runner`] that has been registered, until they all exit
    #[cfg(not(feature = "runtime-loading"))]
    pub async fn run(self) {
        use tokio::task::JoinSet;

        let RegistryState::Building(building_registry) = self.state;

        let mut futures = JoinSet::new();
        for resource in building_registry.resources.into_values() {
            if let Ok(runner) = resource.runner() {
                let _ = futures.spawn(Self::run_runner(runner, self.event_sender.clone()));
            }
        }

        futures.join_all().await;
    }

    /// Asynchronously run each [`Runner`] that has been registered, until they all exit
    ///
    /// # Panics
    ///
    /// - If using the `runtime-loading` feature and this is called on a registry handle
    #[cfg(feature = "runtime-loading")]
    pub async fn run(self) {
        use futures::FutureExt;
        use tokio::task::JoinSet;

        let RegistryState::Building(building_registry) = self.state else {
            panic!("tried to run an already running registry");
        };

        let mut receiver = building_registry.resource_receiver;
        let mut futures = JoinSet::new();
        for resource in building_registry.resources.into_values() {
            match resource.runner() {
                Ok(runner) => {
                    let _ = futures.spawn(Self::run_runner(runner, self.event_sender.clone()));
                }
                Err(resource) => self
                    .event_sender
                    .send(RegistryEvent::NonRunnable(ResourceSupplier::new(resource))),
            }
        }
        // drop the sender now so we don't keep the channel alive
        // (which would mess with the complete condition)
        drop(building_registry.resource_sender);

        // the registry listens for any new futures to add to the queue
        loop {
            tokio::select! {
                // this is simply to check if the JoinSet is finished
                val = futures.join_next() => {
                    if val.is_none() {
                        break;
                    }
                },
                resource = receiver.recv().fuse() => {
                    if let Some(runner) = resource.and_then(|resource| resource.runner().ok()) {
                        let sender = self.event_sender.clone();
                        futures.spawn(Self::run_runner(runner, sender));
                        // futures.push(Self::run_runner(runner, sender));
                    }
                },
                // this automatically gets called if there aren't any more futures or event senders
                else => break,
            };
        }
    }

    async fn run_runner(runner: Box<dyn Runner>, sender: EventSender) {
        let id = runner.ident().to_owned();
        sender.send(RegistryEvent::Running(id.clone()));
        runner.run().instrument(info_span!("runner", %id)).await;
        sender.send(RegistryEvent::Stopped(id));
    }

    /// Load a [`Loader`] into the registry, adding the [`LoadedResource`] it contains. The
    /// resource will be able to be accessed through [`Self::get_as`], and if it provides a
    /// [`Runner`], the runner will be run in the main loop
    ///
    /// Note: As this method sends the correct [`RegistryEvent`]s, it should be prefered to loading
    /// the loader externally
    ///
    /// # Errors
    ///
    /// - If the [`Loader`] returns an error; see [`Loader::load`]
    pub async fn add_loader(
        &mut self,
        loader: Box<dyn Loader>,
        bundle: NamespacedId,
    ) -> Result<&mut Self, Error> {
        self.event_sender.send(RegistryEvent::Loading {
            id: loader.ident().into(),
            bundle,
        });
        // TODO: handle this error
        let resource = loader.load(self).await?;
        Ok(self.add_boxed_resource(resource).await)
    }

    /// Add an already-loaded resource to the registry. The resource can be accessed through
    /// [`Self::get_as`], and if it provides a [`Runner`], the runner will be run in the main loop
    pub async fn add_boxed_resource(&mut self, resource: Box<dyn LoadedResource>) -> &mut Self {
        self.event_sender
            .send(RegistryEvent::Loaded(resource.ident().into()));
        self.state.add_boxed_resource(resource).await;
        self
    }

    /// Register a bundle to the registry
    ///
    /// Note: This only adds the "specification" of the bundle to the registry (i.e. its id,
    /// potential parent, ids of its child resources and bundles, etc) - not its actual resources
    pub(crate) fn add_bundle(&mut self, id: NamespacedId, bundle: BuiltBundle) -> &mut Self {
        self.state.add_bundle(id.clone());
        self.event_sender.send(RegistryEvent::BundleLoading {
            id,
            parent: bundle.parent,
            resources: bundle.resources,
            bundles: bundle.bundles,
        });
        self
    }

    /// Get a [synchronized reference](GetRef) to a loaded resource, using `reference_holder` as
    /// the designated holder of the reference for [`RegistryEvent::Reference`] (see
    /// [`ResourceMap`] for why this information is necessary).
    ///
    /// This uses [`Any`] in order to get the contained object, if it exists.
    // NOTE: in a previous version, this returned Option<&R>. But that messes with RegistryEvent,
    // as resources could get a reference and then immediately drop it, confusing the ResourceMap
    // (the map would think that the resource would still contain the reference).
    #[must_use]
    pub async fn get_as<R: Resource + GetRef>(
        &self,
        reference_holder: &NamespacedIdRef,
    ) -> Option<R::Ref> {
        self.state
            .get_as::<R>(reference_holder, &self.event_sender)
            .await
    }
}

/// Utilities for managing an already-running [`Registry`]
///
/// Notice that these are only accessable through the `runtime-loading` feature
#[cfg(feature = "runtime-loading")]
impl Registry {
    /// Obtain a handle to the [`Registry`], which can be used to manage it externally
    ///
    /// The handle acts the same as a normal, still-building registry, but it uses channels to
    /// communicate and send new loaders to the currently-running registry.
    ///
    /// Note: This is only accessable through the `runtime-loading` feature
    ///
    /// Note: Under the feature, [`Registry`] also implements [`Input`] by calling this function
    ///
    /// # Errors
    ///
    /// - If [`ResourceMap`] hasn't been loaded yet
    pub fn handle(&self, reference_holder: &NamespacedIdRef) -> Result<Self, Error> {
        let (resource_sender, map) = match &self.state {
            RegistryState::Running(running) => {
                (running.resource_sender.clone(), running.map.clone())
            }
            RegistryState::Building(building) => (
                building.resource_sender.clone(),
                building
                    .get_as::<ResourceMap>(reference_holder, &self.event_sender)
                    .ok_or_else(|| Error::LoaderNotFound {
                        id: ResourceMap::const_ident().to_owned(),
                    })?,
            ),
        };
        let implementation = RegistryState::Running(RunningRegistry {
            resource_sender,
            map,
        });
        let handle = Self {
            channels: self.channels.clone(),
            event_sender: self.event_sender.clone(),
            state: implementation,
        };
        Ok(handle)
    }

    /// Returns `true` if this [`Registry`] instance is a [handle](Self::handle) to an
    /// already-running registry
    #[must_use]
    pub const fn is_handle(&self) -> bool {
        self.state.is_running()
    }

    /// Creates a new [`App`] that can eventually be added to this registry
    pub fn new_app(&self) -> App {
        App::new_empty()
    }
}

/// Methods to access the various channels that the registry contains
impl Registry {
    #[must_use]
    pub fn message_sender<M: Message>(&self) -> broadcast::Sender<M> {
        self.channels.sender()
    }

    #[must_use]
    pub fn message_receiver<M: Message>(&self) -> broadcast::Receiver<M> {
        self.channels.receiver()
    }

    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper {
            sender: self.event_sender(),
        }
    }

    #[must_use]
    pub(crate) fn event_sender(&self) -> EventSender {
        self.event_sender.clone()
    }

    #[must_use]
    pub(crate) const fn event_sender_ref(&self) -> &EventSender {
        &self.event_sender
    }

    #[must_use]
    pub fn event_receiver(&self) -> EventReceiver {
        self.event_sender.subscribe()
    }
}

/// Methods to access the current state of the registry
impl Registry {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] that is currently loaded
    #[must_use]
    pub fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.state.loaded_resources()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] that is currently loaded
    #[must_use]
    pub fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.state.loaded_bundles()
    }
}

#[cfg(feature = "runtime-loading")]
impl Input for Registry {
    /// Use a registry [handle](Self::handle) as an input to a running resource
    async fn create(registry: &Self, reference_holder: &NamespacedIdRef) -> Result<Self, Error> {
        registry.handle(reference_holder)
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_resource(ResourceMap::const_ident())
    }
}

impl<M: Message> Input for Sender<M> {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
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
    Stop(NamespacedId),
    /// `id` has begun being loaded into the main [`Registry`]
    ///
    /// Note: All [resources](mod@crate::resource) will emit this message, even if they won't
    /// provide a [`Runner`] - see [`Self::Running`] instead, if that information is needed.
    Loading {
        id: NamespacedId,
        bundle: NamespacedId,
    },
    /// `id` has been loaded into the main [`Registry`]
    ///
    /// Note: All [resources](mod@crate::resource) will emit this message, even if they won't
    /// provide a [`Runner`] - see [`Self::Running`] instead, if that information is needed.
    Loaded(NamespacedId),
    /// `id` has started running
    ///
    /// Note: this only is emitted for resources that have [`Runner`]s, like
    /// [listeners](crate::resource::Listener) and [watchers](crate::resource::Watcher). As simple
    /// [Resources](crate::resource::Resource) are never run, they never emit this message.
    Running(NamespacedId),
    /// `id` has stopped running
    ///
    /// Note: this only is emitted for resources that are [`Runner`]s, like
    /// [listeners](crate::resource::Listener) and [watchers](crate::resource::Watcher). As simple
    /// [Resources](crate::resource::Resource) are never run, they never emit this message - see
    /// [`Self::Unloaded`] instead, if that information is needed.
    Stopped(NamespacedId),
    /// `id` has been unloaded from the app: it has stopped if it ever started running, and all
    /// [references](crate::input::Get) of it from other [resources](mod@crate::resource) have been
    /// dropped
    ///
    /// Note: this will only ever be sent if the `resource-map` feature is enabled
    #[cfg(feature = "resource-map")]
    Unloaded(NamespacedId),
    /// A bundle has started loading into the registry
    BundleLoading {
        id: NamespacedId,
        parent: Option<NamespacedId>,
        resources: Arc<[NamespacedId]>,
        bundles: Arc<[NamespacedId]>,
    },
    /// The last child of the bundle represented by `id` has unloaded, unloading the bundle
    ///
    /// Note: this will only ever be sent if the `resource-map` feature is enabled
    #[cfg(feature = "resource-map")]
    BundleUnloaded(NamespacedId),
    /// The resource `from` referenced the resource `to` through a [`Get<_>`](crate::input::Get)
    /// [input](crate::input::Input)
    Reference { by: NamespacedId, to: NamespacedId },
    /// There is a [`Get<_>`](crate::input::Get) request that must be fulfilled
    ///
    /// Note: this will only ever be sent if the `runtime-loading` feature is enabled
    #[cfg(feature = "runtime-loading")]
    Get(Arc<GetRequest>),
    /// Added a resource to the [`Registry`] that doesn't provide a [`Runner`]
    #[cfg(feature = "runtime-loading")]
    NonRunnable(ResourceSupplier),
    /// The resource `from` emitted `error`
    Error(ErrorEvent),
}

impl RegistryEvent {
    /// Returns `true` if the registry event is [`Error`].
    ///
    /// [`Error`]: RegistryEvent::Error
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(..))
    }
}

#[derive(Clone, Debug)]
pub struct ErrorEvent {
    source: NamespacedId,
    error: Arc<anyhow::Error>,
}

impl ErrorEvent {
    fn new(source: NamespacedId, error: anyhow::Error) -> Self {
        Self {
            source,
            error: Arc::new(error),
        }
    }

    #[must_use]
    pub fn source(&self) -> &NamespacedIdRef {
        &self.source
    }

    #[must_use]
    pub fn error(&self) -> &anyhow::Error {
        &self.error
    }
}

impl PartialEq for ErrorEvent {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && Arc::ptr_eq(&self.error, &other.error)
    }
}

impl Eq for ErrorEvent {}

impl Hash for ErrorEvent {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        Arc::as_ptr(&self.error).hash(state);
    }
}

/// A request to all [resources](mod@crate::resource) to fulfil a [`Get<_>`](crate::input::Get) input
///
/// Under the hood, this is sent to all running resources as a [`RegistryEvent`]. Each resource
/// checks if the request's id matches their own, and if so, uses
/// [`MaybeGetRef`](crate::input::MaybeGetRef) to try to create the reference to send back. If the
/// trait succeeds, they send the result back to the asker.
///
/// `MaybeGetRef` is used because there must be a trait that works for all resources. The
/// underlying runner won't be able to know if the type it represents implements [`GetRef`], so
/// instead it must be able to use a trait that works either way.
#[cfg(feature = "runtime-loading")]
pub struct GetRequest {
    id: NamespacedId,
    // NOTE: This must be a tokio Mutex, not a std one. Why? I'm not really sure!
    // In fact, this was the culprit of what has been the most dreadful bug I have ever encountered
    // due to insane amounts of inconsistency... Deadlocks are not my friend - they very well may be
    // my greatest enemy D:
    sender: tokio::sync::Mutex<Option<oneshot::Sender<Box<dyn crate::input::get::CloneAny>>>>,
}

#[cfg(feature = "runtime-loading")]
impl GetRequest {
    /// Check if `resource` is the resource that is asked for by the request, and if so, call
    /// [`GetRef`] and send the result back to the original asker.
    #[allow(clippy::missing_panics_doc)] // should resume panic when lock is poisoned
    pub async fn check<R: Resource>(&self, resource: &R) {
        self.check_ref(resource).await;
    }

    /// Returns `true` if `R` if the resource that is asked for by this request
    #[inline]
    pub fn matches<R: Resource>(&self) -> bool {
        R::const_ident() == &self.id
    }

    /// Check if `resource` is the resource that is asked for by the request, and if so, call
    /// [`GetRef`] and send the result back to the original asker.
    #[allow(clippy::missing_panics_doc)] // should resume panic when lock is poisoned
    pub async fn check_ref(&self, resource: &dyn Resource) {
        if !self.matches_ref(resource) {
            return;
        }
        trace!("fulfilling get request");
        #[allow(clippy::option_if_let_else)] // not using the return type...
        if let Some(reference) = resource.maybe_get_ref() {
            self.send(reference).await;
        } else {
            warn!("resource matches GetRequest, but MaybeGetRef returned None");
            self.cancel().await;
        }
    }

    /// Returns `true` if `resource` if the resource that is asked for by this request
    #[inline]
    pub fn matches_ref(&self, resource: &dyn Resource) -> bool {
        resource.ident() == &self.id
    }

    /// Returns the [id](NamespacedId) of the resource that the request is asking for
    pub fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    /// Cancels the get request, instantly returning nothing
    ///
    /// This should only be used by the resource that this request is for if they are sure that
    /// they can't fulfil the request.
    pub async fn cancel(&self) {
        drop(self.sender.lock().await.take());
    }

    /// Fulfils the request by sending `reference`
    ///
    /// Note: this doesn't actually check if the reference matches the request - use
    /// [`Self::matches`] to check first
    pub(crate) async fn send(&self, reference: Box<dyn crate::input::get::CloneAny>) {
        let Some(sender) = self.sender.lock().await.take() else {
            // this could only happen with a name collision
            warn!("resource matches GetRequest, but the channel was already used");
            return;
        };
        let _ = sender.send(reference);
    }
}

#[cfg(feature = "runtime-loading")]
impl PartialEq for GetRequest {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[cfg(feature = "runtime-loading")]
impl Eq for GetRequest {}

#[cfg(feature = "runtime-loading")]
impl Debug for GetRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.id)
    }
}

#[cfg(feature = "runtime-loading")]
impl std::hash::Hash for GetRequest {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Clone)]
#[cfg(feature = "runtime-loading")]
pub struct ResourceSupplier {
    arc: Arc<ResourceSupplierInner>,
}

#[cfg(feature = "runtime-loading")]
struct ResourceSupplierInner {
    id: NamespacedId,
    resource: std::sync::Mutex<Option<Box<dyn LoadedResource>>>,
}

#[cfg(feature = "runtime-loading")]
impl ResourceSupplier {
    /// Get the [`NamespacedId`] of the inner resource
    #[must_use]
    pub fn id(&self) -> &NamespacedIdRef {
        &self.arc.id
    }

    fn new(resource: Box<dyn LoadedResource>) -> Self {
        let arc = ResourceSupplierInner {
            id: resource.ident().to_owned(),
            resource: std::sync::Mutex::new(Some(resource)),
        };
        Self { arc: Arc::new(arc) }
    }

    pub(crate) fn take(&self) -> Option<Box<dyn LoadedResource>> {
        let mut resource = self
            .arc
            .resource
            .lock()
            .expect("ResourceSupplier lock shouldn't be poisoned");
        resource.take()
    }
}

#[cfg(feature = "runtime-loading")]
impl Debug for ResourceSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ResourceSupplier")
    }
}

#[cfg(feature = "runtime-loading")]
impl PartialEq for ResourceSupplier {
    fn eq(&self, other: &Self) -> bool {
        self.arc.id == other.arc.id
    }
}

#[cfg(feature = "runtime-loading")]
impl Eq for ResourceSupplier {}

#[cfg(feature = "runtime-loading")]
impl Hash for ResourceSupplier {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.arc.id.hash(state);
    }
}

/// A handle on a [`Registry`] that can be used to stop one single resource or exit the entire
/// program
#[derive(Clone, Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct Stopper {
    sender: EventSender,
}

impl Stopper {
    /// Tell all resources in the [`Registry`] to exit
    pub fn exit(&self) {
        self.sender.send(RegistryEvent::Exit);
    }

    /// Stop the resource `R`, if it exists
    #[inline]
    pub fn stop<R: Identifiable>(&self) {
        self.stop_id(R::const_ident());
    }

    /// Stop the resource with id `id`, if it exists
    pub fn stop_id(&self, id: &NamespacedIdRef) {
        let id = id.to_owned();
        self.sender.send(RegistryEvent::Stop(id));
    }
}

impl Input for Stopper {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(registry.stopper())
    }
}

/// A sender for [`RegistryEvent`]s
///
/// Each of these events will always be read by every other receiver; see [`EventReceiver`] for
/// more information about the channel.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct EventSender {
    watch: watch::Sender<Vec<RegistryEvent>>,
}

impl EventSender {
    pub(crate) fn new() -> Self {
        Self {
            watch: watch::Sender::new(Vec::new()),
        }
    }

    pub(crate) fn send(&self, event: RegistryEvent) {
        self.watch.send_modify(|vec| vec.push(event));
    }

    pub(crate) fn extend<I: IntoIterator<Item = RegistryEvent>>(&self, events: I) {
        self.watch.send_modify(|vec| vec.extend(events));
    }

    pub fn send_error(&self, ident: impl Into<NamespacedId>, error: anyhow::Error) {
        self.send(RegistryEvent::Error(ErrorEvent::new(ident.into(), error)));
    }

    pub fn handle_error<T>(&self, res: anyhow::Result<T>, id: &NamespacedIdRef) -> Option<T> {
        res.map_err(|err| self.send_error(id, err)).ok()
    }

    pub fn send_errors<E: Into<anyhow::Error>>(
        &self,
        ident: impl Into<NamespacedId>,
        errors: impl IntoIterator<Item = E>,
    ) {
        let ident = ident.into();
        let events = errors
            .into_iter()
            .map(|err| RegistryEvent::Error(ErrorEvent::new(ident.clone(), err.into())));
        self.extend(events);
    }

    pub(crate) fn subscribe(&self) -> EventReceiver {
        EventReceiver::new(self.watch.subscribe())
    }
}

impl Input for EventSender {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(registry.event_sender())
    }
}

/// A receiver for [`RegistryEvent`]s
///
/// This will always hold the entire history of events, even ones that happened before the receiver
/// was created. This allows receivers to make models of the [`Registry`] using its entire state,
/// like [`ResourceMap`].
#[derive(Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct EventReceiver {
    watch: watch::Receiver<Vec<RegistryEvent>>,
    index: usize,
}

impl EventReceiver {
    const fn new(receiver: watch::Receiver<Vec<RegistryEvent>>) -> Self {
        Self {
            watch: receiver,
            index: 0,
        }
    }

    /// Receive the oldest unhandled message or return [`None`] if the channel has closed.
    #[instrument(name = "recv_registry_event", skip(self))]
    pub async fn recv(&mut self) -> Option<RegistryEvent> {
        loop {
            let value = self.watch.borrow_and_update().get(self.index).cloned();
            if let Some(val) = value {
                self.index += 1;
                return Some(val);
            }
            self.watch
                .changed()
                .instrument(info_span!("waiting_for_new_event"))
                .await
                .ok()?;
        }
    }

    /// Receive an unhandled message if it is ready
    ///
    /// # Errors
    ///
    /// - [`EventTryRecvError::NotReady`] if the stream is not ready
    /// - [`EventTryRecvError::Closed`] if the stream is closed
    pub fn try_recv(&mut self) -> Result<RegistryEvent, EventTryRecvError> {
        let value = self.watch.borrow_and_update().get(self.index).cloned();
        if let Some(val) = value {
            self.index += 1;
            // can't use else 'cause the compiler gets angry at the mutable ref above
            return Ok(val);
        };
        // the easiest way to check if the channel is closed is through this method
        if self.watch.has_changed().is_err() {
            Err(EventTryRecvError::Closed)
        } else {
            Err(EventTryRecvError::NotReady)
        }
    }

    /// Resubscribe to the channel, starting from the beginning.
    #[must_use]
    pub fn resubscribe(&mut self) -> Self {
        Self {
            watch: self.watch.clone(),
            index: 0,
        }
    }

    /// Mark the most recent message as seen, ignoring recv
    pub fn mark_seen(&mut self) {
        let vec = self.watch.borrow_and_update();
        self.index = vec.len();
    }
}

impl Input for EventReceiver {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(registry.event_receiver())
    }
}

/// The result for [`EventReceiver::try_recv`]
pub enum EventTryRecvError {
    Closed,
    NotReady,
}

impl EventTryRecvError {
    /// Returns `true` if the event try recv error is [`Closed`].
    ///
    /// [`Closed`]: EventTryRecvError::Closed
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Returns `true` if the event try recv error is [`NotReady`].
    ///
    /// [`NotReady`]: EventTryRecvError::NotReady
    #[must_use]
    pub const fn is_not_ready(&self) -> bool {
        matches!(self, Self::NotReady)
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

// add_resource_with and such redo
//
// Instead of add_resource_with, do something like a Create trait
//
// ```
// trait Create {
//     // similar to input for resources
//     type RegistryInput: Input;
//     // input given when adding the type to an App
//     // necessary for things like a LibraryHandle
//     type ExternalInput;
//
//     fn create(
//         registry_input: Self::RegistryInput::Output,
//         external_input: ExternalInput,
//     ) -> Self;
// }
// ```
//
// ```
// let app = App::new(32)
//         new_bundle!("bundle:bundle")
//             .with_listener::<Library>(())
//             .with_resource::<QueueLimit>(32)
//     );
// ```
