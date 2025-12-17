use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
#[cfg(feature = "runtime-loading")]
use tokio::sync::mpsc;

#[cfg(feature = "resource-map")]
use crate::app::ResourceMap;
#[cfg(feature = "runtime-loading")]
use crate::app::channels;
use crate::app::channels::ChannelsRefMut;
#[cfg(feature = "resource-map")]
use crate::app::channels::Giver;
#[cfg(feature = "resource-map")]
use crate::app::resource_map;
#[cfg(feature = "runtime-loading")]
use crate::app::resource_map::ResourceMapConnection;
use crate::app::{BuiltBundle, Loader, Runner, event};
#[cfg(feature = "runtime-loading")]
use crate::input::CloneAs;
use crate::input::get::GetRef;
use crate::{app::LoadedResource, prelude::*};

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
/// - To listen to messages, use [`Self::channels`]
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
    /// the designated holder of the reference for [`RegistryEvent::IncrementDependency`] (see
    /// [`ResourceMap`] for why this information is necessary).
    ///
    /// This uses [`Any`](crate::Any) in order to get the contained object, if it exists.
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
    fn clone_as(&self, _: ComponentIdRef<'_>) -> Self
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

    /// Returns a set of the [`NamespacedId`]s of each [`Message`](crate::Message) that is currently loaded
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
