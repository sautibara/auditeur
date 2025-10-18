//! The main entry-point for the application: loading and running [resources](mod@crate::resource)
//!
//! The main [`App`] handles loading [resources](crate::resource::Resource),
//! [listeners](crate::resource::listener::Listener), and [watchers](crate::resource::Watcher) from
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
//!       through [`Get<_>`](crate::input::get::Get) are already loaded
//! - Once loaded, resources are each built into a [`LoadedResource`]
//!     - [`LoadedResource`]s build the [`Input`]s for their resources from the registry
//!     - [`LoadedResource`]s allow for dynamically accessing the contained resource by the
//!       registry, enabling [`input::Get<_>`](crate::input::get::Get) to work
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
use std::collections::hash_map;
use std::fmt::Debug;
use std::future::Future;
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_recursion::async_recursion;
use futures::FutureExt;
use tokio::sync::mpsc;
use tracing::error;
use tracing::info_span;
use tracing::instrument;
use tracing::trace;

use crate::app::channels::Channels;
use crate::app::channels::ChannelsRefMut;
use crate::app::channels::Giver;
use crate::app::channels::TypedAnyChannel;
use crate::app::event::CloneAs;
use crate::app::event::ComponentId;
use crate::app::event::ComponentIdRef;
#[cfg(feature = "runtime-loading")]
use crate::app::resource_map::ResourceMapConnection;
use crate::bundle;
use crate::input::get::GetRef;
use crate::prelude::*;

pub mod channels;
mod constraints;
pub mod event;

#[cfg(feature = "resource-map")]
pub mod resource_map;
#[cfg(feature = "resource-map")]
pub use resource_map::ResourceMap;

pub use constraints::Constraint;
pub use constraints::Constraints;

use channels::ChannelsRef;

/// An object that can be loaded with a [`Registry`] to return a finished [`LoadedResource`]
///
/// This trait and its delayed execution is the thing that allows for the "trait magic" of
/// [inputs](Input). When loading, inputs are taken from the [Registry]. Resources are ensured to
/// already be loaded through the [`Constraints`] of each input, which is why delayed execution is
/// necessary.
#[async_trait]
pub trait Loader: Send {
    /// Consume the loader and create a [`LoadedResource`], resolving inputs using `registry`
    ///
    /// # Errors
    ///
    /// - If the inputs return any errors
    ///   - If any resources are not found
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error>;
    /// Return a list of [`Constraints`] that must be satisfied before loading
    fn constraints(&self) -> Constraints;
    /// Return a reference to the contained [resource](mod@crate::resource)'s [`NamespacedId`]
    fn ident(&self) -> &NamespacedIdRef;
    /// Returns a set of channels that this loader "owns".
    ///
    /// These channels will automatically be added to the [registry](Registry), but special care
    /// should be taken to make sure that a specific channel isn't added multiple times.
    #[must_use]
    fn channels(&self) -> Channels {
        Channels::new()
    }
}
/// A [resource](mod@crate::resource) that has already been loaded into a [`Registry`]
#[allow(clippy::module_name_repetitions)] // without Loaded, it would conflict with Resource
pub trait LoadedResource: Identifiable + Send + Sync {
    /// Optionally provide a [`Runner`] to run in the main loop
    ///
    /// This runner is joined alongside every other runner, if it is given
    ///
    /// # Errors
    ///
    /// - Returns [`Err(Self)`] if this resource doesn't provide a runner
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>>;
    /// Obtain a reference to the original resource within a trait object
    fn resource(&self) -> &dyn Resource;
    /// Obtain a reference to the original resource within a trait object
    fn resource_mut(&mut self) -> &mut dyn Resource;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: Identifiable + Send {
    async fn run(self: Box<Self>);
}

// TODO: separate NamespacedId into ResourceId and BundleId

/// The main entry-point for the application: the thing that runs resources
///
/// The app handles loading [resources](crate::resource::Resource),
/// [listeners](crate::resource::listener::Listener), and [watchers](crate::resource::Watcher),
/// giving them the correct [inputs](crate::input::Input) (including
/// [each other](crate::input::get::Get)), and [running them](Self::run) asynchronously. It also
/// makes sure they are loaded in the correct order, through [constraints](Constraints), allowing
/// them to have other dependencies.
///
/// The main container for resources is the [`Bundle`], and all resources that are added to an
/// `App` must be contained within one. As such, all resources added to an app must be added
/// through a bundle. If there are conflicts between resources with the same id, the one that is
/// added first is kept.
#[must_use = "An app does nothing unless it is run"]
pub struct App {
    loaders: HashMap<NamespacedId, Option<BundledLoader>>,
    bundles: HashMap<NamespacedId, Option<BuiltBundle>>,
    channels: HashMap<NamespacedId, Option<BuiltChannel>>,

    loading_loaders: HashMap<NamespacedId, NamespacedId>,
    // TODO: check if adding App events is a good idea
    events: Vec<RegistryEvent>,
}

struct BundledLoader {
    loader: Box<dyn Loader>,
    bundle: NamespacedId,
}

pub(crate) struct BuiltBundle {
    pub constraints: Constraints,
    pub parent: Option<NamespacedId>,
    pub resources: Arc<[NamespacedId]>,
    pub bundles: Arc<[NamespacedId]>,
    #[cfg(feature = "runtime-loading")]
    pub init: Option<bundle::SharedInitFn>,
}

#[derive(Debug)]
struct BuiltChannel {
    channel: TypedAnyChannel,
    bundle: NamespacedId,
}

impl App {
    /// Create a new app
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            bundles: HashMap::new(),
            channels: HashMap::new(),

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

    pub(crate) fn add_bundle_inner(&mut self, id: NamespacedId, bundle: BuiltBundle) {
        self.bundles.insert(id, Some(bundle));
    }

    pub(crate) fn add_loader(
        &mut self,
        loader: Box<dyn Loader>,
        bundle: &NamespacedIdRef,
    ) -> NamespacedId {
        self.add_channels(loader.channels(), bundle);

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

    pub(crate) fn add_channels(&mut self, channels: Channels, bundle: &NamespacedIdRef) {
        let Channels { listen, take } = channels;

        let listen = listen
            .into_iter()
            .map(|(id, channel)| (id, TypedAnyChannel::Listen(channel)));
        let take = take
            .into_iter()
            .map(|(id, channel)| (id, TypedAnyChannel::Take(channel)));

        for (id, channel) in listen.chain(take) {
            let entry = self.channels.entry(id);
            match entry {
                hash_map::Entry::Occupied(entry) => {
                    // TODO: remove duplicated messages for ResourceMap
                    if entry.key() != resource_map::ResourceMapQuery::const_ident() {
                        error!(
                            "message '{}' added multiple times, using old message",
                            entry.key()
                        );
                    }
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(BuiltChannel {
                        channel,
                        bundle: bundle.to_owned(),
                    }));
                }
            }
        }
    }

    /// [Build](Self::build) the `App` and run all [runners](Runner) to completion
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run(self) -> Result<(), (Self, Error)> {
        let registry = self.build().await?;
        registry.run().await;
        Ok(())
    }

    /// Runs an asynchronous function alongside the app, stopping the app once the function exits.
    /// The function may also take in [`Input`]s.
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run_with<F, Fut, I>(self, func: F) -> Result<(), Error>
    where
        F: FnOnce(I) -> Fut,
        Fut: Future<Output = ()>,
        I: Input,
    {
        let ident = ComponentIdRef::Resource(ident!("auditeur:run_with"));
        let registry = self.build().await.map_err(|(_, err)| err)?;
        let input = I::create(&registry, ident).await?;
        let stopper = Stopper::create(&registry, ident).await?;
        let running = tokio::spawn(registry.run());
        func(input).await;
        stopper.exit();
        let _ = running.await;
        Ok(())
    }

    /// Build the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn build(self) -> Result<Registry, (Self, Error)> {
        let mut registry = Registry::new();
        self.add_to_registry(&mut registry).await?;
        Ok(registry)
    }

    /// Add the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn add_to_registry(mut self, registry: &mut Registry) -> Result<(), (Self, Error)> {
        trace!("adding App to registry: {self:?}");
        self.add_to_registry_fallible(registry)
            .await
            .map_err(|err| (self, err))
    }

    async fn add_to_registry_fallible(&mut self, registry: &mut Registry) -> Result<(), Error> {
        // free events :)
        registry.event_sender_ref().extend(self.events.drain(..));
        // add the loaders and bundles that have already been loaded in the registry to our list
        // this is necessary so that constraints can know that their dependencies have already been
        // loaded
        self.loaders.extend(
            registry
                .loaded_resources()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );
        self.bundles.extend(
            registry
                .loaded_bundles()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );
        self.channels.extend(
            registry
                .loaded_messages()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );

        if !self.loaders.contains_key(ResourceMap::const_ident()) {
            self.add_bundle(resource_map::init);
        }

        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for loader in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve_loader(registry, &loader).await?;
        }
        Ok(())
    }

    #[async_recursion]
    async fn resolve_loader(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving loader {id}");
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
        // then now that we have the parent bundle we could say that this resource is loading
        registry.event_sender.send(RegistryEvent::Resolving {
            id: loader.ident().into(),
            bundle: bundle.clone(),
        });
        // if there are constraints, we must load them first
        self.resolve_constraints(registry, &loader.constraints())
            .await?;
        // then we can finally load the loader
        registry.add_loader(loader, bundle).await?;
        self.loading_loaders.remove(id);
        trace!("resolved loader {id}");
        Ok(())
    }

    #[async_recursion]
    async fn resolve_bundle(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving bundle {id}");
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
        // then now that we have the parent bundle we could say that this bundle is loading
        registry.event_sender.send(RegistryEvent::BundleResolving {
            id: id.to_owned(),
            parent: bundle.parent.clone(),
            resources: bundle.resources.clone(),
            bundles: bundle.bundles.clone(),
        });
        // then the constraints also have to be resolved
        self.resolve_constraints(registry, &bundle.constraints)
            .await?;
        // then the bundle can finally be added to the registry
        registry.add_bundle(id.to_owned(), bundle);
        trace!("resolved bundle {id}");
        Ok(())
    }

    #[async_recursion]
    async fn resolve_message(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving message {id}");
        let channel = self
            .channels
            .get_mut(id)
            .ok_or_else(|| Error::MessageNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(channel) = channel.take() else {
            return Ok(());
        };
        // the parent bundle always have to be resolved first
        self.resolve_bundle(registry, &channel.bundle).await?;

        match channel.channel {
            TypedAnyChannel::Listen(listen_channel) => {
                // then now that we have the parent bundle we could say that this bundle is loading
                registry
                    .event_sender
                    .send(RegistryEvent::ListenMessageLoaded {
                        id: id.to_owned(),
                        bundle: channel.bundle.clone(),
                    });

                // then the channel can finally be added to the registry
                let mut channels = Channels::new();
                channels.listen.insert(id.to_owned(), listen_channel);
                registry.channels_mut().add(channels).await;
            }
            TypedAnyChannel::Take(take_channel) => {
                registry
                    .event_sender
                    .send(RegistryEvent::TakeMessageLoaded {
                        id: id.to_owned(),
                        bundle: channel.bundle.clone(),
                    });

                let mut channels = Channels::new();
                channels.take.insert(id.to_owned(), take_channel);
                registry.channels_mut().add(channels).await;
            }
        }
        trace!("resolved message {id}");
        Ok(())
    }

    async fn resolve_constraints(
        &mut self,
        registry: &mut Registry,
        constraints: &Constraints,
    ) -> Result<(), Error> {
        for constraint in constraints {
            match constraint {
                Constraint::After(component) => match component {
                    ComponentId::Resource(id) => self.resolve_loader(registry, id).await?,
                    ComponentId::Bundle(id) => self.resolve_bundle(registry, id).await?,
                    ComponentId::Message(id) => self.resolve_message(registry, id).await?,
                },
            }
        }
        Ok(())
    }

    pub fn add_error(&mut self, source: NamespacedId, error: anyhow::Error) {
        self.events
            .push(RegistryEvent::Error(event::Error::new(source, error)));
    }

    pub fn add_errors<I: IntoIterator<Item = anyhow::Error>>(
        &mut self,
        source: &NamespacedIdRef,
        errors: I,
    ) {
        self.events.extend(
            errors
                .into_iter()
                .map(|error| RegistryEvent::Error(event::Error::new(source.to_owned(), error))),
        );
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
            .field("channels", &self.channels)
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
#[derive(Debug)]
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

    #[cfg_attr(not(feature = "runtime-loading"), allow(clippy::unused_async))]
    #[inline]
    async fn get_as<R: Resource + GetRef>(
        &self,
        dependent: ComponentIdRef<'_>,
        event_sender: &EventSender,
        parent: &Registry,
    ) -> Result<R::Ref, crate::Error> {
        match self {
            Self::Building(registry) => registry.get_as::<R>(dependent, parent).await,
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.get_as::<R>(dependent, event_sender).await,
        }
    }

    #[inline]
    async fn loaded_resources(&self) -> HashSet<NamespacedId> {
        match self {
            Self::Building(registry) => registry.loaded_resources(),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.loaded_resources().await,
        }
    }

    #[inline]
    async fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        match self {
            Self::Building(registry) => registry.loaded_bundles(),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.loaded_bundles().await,
        }
    }

    #[inline]
    async fn loaded_messages(&self) -> HashSet<NamespacedId> {
        match self {
            Self::Building(registry) => registry.loaded_messages(),
            #[cfg(feature = "runtime-loading")]
            Self::Running(registry) => registry.loaded_messages().await,
        }
    }

    const fn channels_ref<'a>(&'a self, event_sender: &'a EventSender) -> ChannelsRef<'a> {
        match self {
            Self::Building(building) => building.channels_ref(event_sender),
            #[cfg(feature = "runtime-loading")]
            Self::Running(running) => running.channels_ref(event_sender),
        }
    }

    const fn channels_ref_mut<'a>(
        &'a mut self,
        event_sender: &'a EventSender,
    ) -> ChannelsRefMut<'a> {
        match self {
            Self::Building(building) => building.channels_ref_mut(event_sender),
            #[cfg(feature = "runtime-loading")]
            Self::Running(running) => running.channels_ref_mut(event_sender),
        }
    }
}

struct BuildingRegistry {
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
    bundles: HashSet<NamespacedId>,
    channels: Channels,
    #[cfg(feature = "resource-map")]
    resource_map_connection: Giver<resource_map::ResourceMapQuery>,
    #[cfg(feature = "runtime-loading")]
    resource_sender: mpsc::Sender<Box<dyn LoadedResource>>,
    #[cfg(feature = "runtime-loading")]
    resource_receiver: mpsc::Receiver<Box<dyn LoadedResource>>,
}

impl Debug for BuildingRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("BuildingRegistry");
        debug_struct.field("resources", &self.resources.keys());
        debug_struct.field("bundles", &self.bundles);
        debug_struct.field("channels", &self.channels);
        #[cfg(feature = "resource-map")]
        debug_struct.field("resource_map_connection", &self.resource_map_connection);
        #[cfg(feature = "runtime-loading")]
        debug_struct.field("resource_sender", &self.resource_sender);
        #[cfg(feature = "runtime-loading")]
        debug_struct.field("resource_receiver", &self.resource_receiver);
        debug_struct.finish()
    }
}

impl BuildingRegistry {
    fn with_channels(mut channels: Channels) -> Self {
        #[cfg(feature = "runtime-loading")]
        let (resource_sender, resource_receiver) = mpsc::channel(64);

        #[cfg(feature = "resource-map")]
        let resource_map_connection = channels.add_take_channel::<resource_map::ResourceMapQuery>();

        Self {
            resources: HashMap::new(),
            bundles: HashSet::new(),
            channels,
            #[cfg(feature = "resource-map")]
            resource_map_connection,
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
    #[instrument(skip_all, fields(from = %dependent, to = %R::const_ident()))]
    async fn get_as<R: Resource + GetRef>(
        &self,
        dependent: ComponentIdRef<'_>,
        parent: &Registry,
    ) -> Result<R::Ref, crate::Error> {
        let id = R::const_ident();
        let loaded = self
            .resources
            .get(id)
            .ok_or_else(|| Error::ResourceNotFound { id: id.to_owned() })?;
        let downcast = (loaded.resource() as &dyn crate::Any)
            .downcast_ref::<R>()
            .ok_or_else(|| Error::DowncastFailed { id: id.to_owned() })?;
        downcast.get_ref(parent, dependent).await
    }

    fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.resources.keys().cloned().collect()
    }

    fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.bundles.clone()
    }

    fn loaded_messages(&self) -> HashSet<NamespacedId> {
        self.channels.ids()
    }

    const fn channels_ref<'a>(&'a self, event_sender: &'a EventSender) -> ChannelsRef<'a> {
        ChannelsRef::new_building(&self.channels, event_sender)
    }

    const fn channels_ref_mut<'a>(
        &'a mut self,
        event_sender: &'a EventSender,
    ) -> ChannelsRefMut<'a> {
        ChannelsRefMut::new_building(&mut self.channels, event_sender)
    }
}

#[cfg(feature = "runtime-loading")]
struct RunningRegistry {
    resource_sender: mpsc::Sender<Box<dyn LoadedResource>>,
    map: ResourceMapConnection,
}

#[cfg(feature = "runtime-loading")]
impl RunningRegistry {
    async fn add_boxed_resource(&self, resource: Box<dyn LoadedResource>) {
        let _ = self.resource_sender.send(resource).await;
    }

    #[must_use]
    #[instrument(skip_all, fields(from = %dependent, to = %R::const_ident()))]
    async fn get_as<R: Resource + GetRef>(
        &self,
        dependent: ComponentIdRef<'_>,
        event_sender: &EventSender,
    ) -> Result<R::Ref, crate::Error> {
        let id = R::const_ident();
        let (get_request, receiver) = event::GetRequest::new(id.to_owned(), dependent.into_owned());
        // FIXME: by using an EventSender, the requests actually never get dropped
        // - it all stays in the main Vec, awaiting new receivers
        // this should be separated into another channel for only Get requests
        event_sender.send(RegistryEvent::Get(Arc::new(get_request)));
        // if the request if fulfilled, this will return Some
        // if it isn't and all requests get dropped, the channel drops and returns a RecvError
        let reference = tokio::time::timeout(std::time::Duration::from_millis(100), receiver)
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| Error::ResourceNotFound { id: id.to_owned() })?;

        trace!("succeeded getting {id}");

        crate::downcast(reference)
            .map(|boxed| *boxed)
            .map_err(|_| Error::DowncastFailed { id: id.to_owned() })
    }

    async fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.map.loaded_resources().await
    }

    async fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.map.loaded_bundles().await
    }

    async fn loaded_messages(&self) -> HashSet<NamespacedId> {
        self.map.loaded_messages().await
    }

    const fn channels_ref<'a>(&'a self, event_sender: &'a EventSender) -> ChannelsRef<'a> {
        ChannelsRef::new_running(&self.map, event_sender)
    }

    const fn channels_ref_mut<'a>(&'a self, event_sender: &'a EventSender) -> ChannelsRefMut<'a> {
        ChannelsRefMut::new_running(&self.map, event_sender)
    }
}

impl Debug for RunningRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RunningRegistry {{ .. }}")
    }
}

/// A list of registered and loaded [resources](LoadedResource), built from an [`App`], that can be run as a main loop
///
/// A registry is built from an [`App`] declaration, which handles loading things in the correct
/// order. This allows resources to access other resources through [`Self::get_as`], or, much more
/// commonly, [`Get<_>`](crate::input::get::Get). It is the main thing that [inputs](crate::input)
/// interact with, allowing resources to automatically get a large amount of information about the
/// app. As such, most channels shown below are also able to be used as inputs (they implement
/// [`Input`]).
///
/// There are many bundled channels for notification passing:
/// - To tell only one resource or the entire program to stop, use a [`Stopper`] through [`Self::stopper`]
/// - To receive notifications about what happens to the registry, use [`Self::event_receiver`]
/// - To listen to messages, use [`Self::message_receiver`]
#[derive(Debug)]
#[cfg_attr(feature = "runtime-loading", derive(PathAny))]
pub struct Registry {
    event_sender: EventSender,
    state: RegistryState,

    #[cfg(feature = "runtime-loading")]
    bundle_init_sender: channels::Shouter<event::BundleInit>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// Creates a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        let mut channels = Channels::new();

        #[cfg(feature = "runtime-loading")]
        let bundle_init_sender = channels.add_listen_channel::<event::BundleInit>();

        let state = RegistryState::Building(BuildingRegistry::with_channels(channels));

        Self {
            #[cfg(feature = "runtime-loading")]
            bundle_init_sender,

            event_sender: EventSender::new(),
            state,
        }
    }

    /// Asynchronously run each [`Runner`] that has been registered, until they all exit
    #[cfg(not(feature = "runtime-loading"))]
    pub async fn run(self) {
        use tokio::task::JoinSet;

        let mut resource_map_option: Option<Box<dyn LoadedResource>>;

        #[cfg(feature = "resource-map")]
        {
            let mut resource_map = ResourceMap::new(self.event_sender.clone());
            let resource_map =
                crate::resource::listener::LoadedListener::build(resource_map, &self)
                    .await
                    .expect("building ResourceMap shouldn't fail");
            let resource_map = Box::new(resource_map) as Box<dyn LoadedResource>;

            resource_map_option = Some(resource_map);
        }

        #[cfg(not(feature = "resource-map"))]
        {
            resource_map_option = None;
        }

        let RegistryState::Building(building_registry) = self.state;

        let mut futures = JoinSet::new();
        for resource in building_registry
            .resources
            .into_values()
            .chain(resource_map_option)
        {
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

        let RegistryState::Building(mut building_registry) = self.state else {
            unreachable!("already checked that it was building above");
        };

        // NOTE: We must add the channels to the ResourceMap before running the other resources.
        // If another resource asks the map to load something before all the channels are added, it
        // could fail when it should succeed.

        let resource_map = building_registry
            .resources
            .get_mut(ResourceMap::const_ident())
            .expect("ResourceMap should be loaded");
        let resource_map: &mut ResourceMap = (resource_map.resource_mut() as &mut dyn crate::Any)
            .downcast_mut()
            .expect("ResourceMap downcast shouldn't fail");
        resource_map.add_channels_pub(building_registry.channels);

        // Now that we initialized the ResourceMap, we could continue to the other resources.

        let mut receiver = building_registry.resource_receiver;
        let mut futures = JoinSet::new();
        for resource in building_registry.resources.into_values() {
            match resource.runner() {
                Ok(runner) => {
                    let _ = futures.spawn(Self::run_runner(runner, self.event_sender.clone()));
                }
                Err(resource) => self.event_sender.send(RegistryEvent::NonRunnable(
                    event::ResourceSupplier::new(resource),
                )),
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
        // We actually don't know if the runner will be unwind safe
        // (for example, the runner could use interior mutability
        // to break an invariant in another runner and then panic).
        // But we can't add the bound to async_trait,
        // so instead we just assume that it will be anyways.
        // It's more likely that the runner will be unwind safe,
        // and we don't want the entire app to exit because of a panic.
        let panic_result = AssertUnwindSafe(runner.run())
            .catch_unwind()
            .instrument(info_span!("runner", %id))
            .await;
        if panic_result.is_err() {
            error!(%id, "runner stopped due to a panic");
        }
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
        #[cfg(feature = "runtime-loading")]
        if let Some(init) = bundle.init {
            self.bundle_init_sender.send(event::BundleInit {
                id: id.clone(),
                init,
            });
        }

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
    ///
    /// # Errors
    ///
    /// - If [`R::get_ref`](GetRef::get_ref) returns an error.
    // NOTE: in a previous version, this returned Option<&R>. But that messes with RegistryEvent,
    // as resources could get a reference and then immediately drop it, confusing the ResourceMap
    // (the map would think that the resource would still contain the reference).
    pub async fn get_as<R: Resource + GetRef>(
        &self,
        dependent: ComponentIdRef<'_>,
    ) -> Result<R::Ref, crate::Error> {
        self.state
            .get_as::<R>(dependent, &self.event_sender, self)
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
    /// # Panics
    ///
    /// - If this method is called on an app built from [`Self::new_app`].
    #[must_use]
    pub fn handle(&self) -> Self {
        use crate::app::event::ComponentIdRef;

        let ident = ComponentIdRef::Resource(ident!("auditeur:registry"));

        let (resource_sender, connection) = match &self.state {
            RegistryState::Running(running) => {
                (running.resource_sender.clone(), running.map.clone_as(ident))
            }
            RegistryState::Building(building) => (
                building.resource_sender.clone(),
                ResourceMapConnection::new_for_registry(
                    self.event_sender.clone(),
                    building.resource_map_connection.clone_erased(),
                ),
            ),
        };

        Self {
            event_sender: self.event_sender.clone(),
            state: RegistryState::Running(RunningRegistry {
                resource_sender,
                map: connection,
            }),
            #[cfg(feature = "runtime-loading")]
            bundle_init_sender: self.bundle_init_sender.clone_as(ident),
        }
    }

    /// Returns `true` if this [`Registry`] instance is a [handle](Self::handle) to an
    /// already-running registry
    #[must_use]
    pub const fn is_handle(&self) -> bool {
        self.state.is_running()
    }

    /// Creates a new [`App`] that can eventually be added to this registry
    pub fn new_app(&self) -> App {
        App::new()
    }
}

#[cfg(feature = "runtime-loading")]
impl CloneAs for Registry {
    fn clone_as(&self, _: event::ComponentIdRef<'_>) -> Self
    where
        Self: Sized,
    {
        self.handle()
    }
}

/// Methods to access the various channels that the registry contains
impl Registry {
    #[must_use]
    pub const fn channels(&self) -> ChannelsRef<'_> {
        self.state.channels_ref(&self.event_sender)
    }

    #[must_use]
    pub(crate) const fn channels_mut(&mut self) -> ChannelsRefMut<'_> {
        self.state.channels_ref_mut(&self.event_sender)
    }

    #[must_use]
    pub fn stopper(&self) -> Stopper {
        Stopper::new(self.event_sender())
    }

    #[must_use]
    pub fn event_sender(&self) -> EventSender {
        self.event_sender.clone()
    }

    #[must_use]
    pub const fn event_sender_ref(&self) -> &EventSender {
        &self.event_sender
    }

    #[must_use]
    pub fn event_receiver(&self) -> channels::EventReceiver {
        self.event_sender.subscribe()
    }
}

/// Methods to access the current state of the registry
impl Registry {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] that is currently loaded
    #[must_use]
    pub async fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.state.loaded_resources().await
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] that is currently loaded
    #[must_use]
    pub async fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.state.loaded_bundles().await
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Message`] that is currently loaded
    #[must_use]
    pub async fn loaded_messages(&self) -> HashSet<NamespacedId> {
        self.state.loaded_messages().await
    }
}

#[cfg(feature = "runtime-loading")]
impl Input for Registry {
    /// Use a registry [handle](Self::handle) as an input to a running resource
    async fn create(registry: &Self, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(registry.handle())
    }
}
