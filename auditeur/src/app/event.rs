use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

#[cfg(feature = "runtime-loading")]
use tokio::sync::oneshot;

#[cfg(feature = "runtime-loading")]
use crate::bundle;
use crate::input::dependency;
use crate::prelude::*;

#[cfg(doc)]
use crate::app::Runner;
#[cfg(doc)]
use crate::app::resource_map::ResourceMap;
#[cfg(doc)]
use crate::resource::{listener::Listener, watcher::Watcher};

/// Messages from the [`Registry`] about different events, such as loading or unloading resources
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RegistryEvent {
    /// Stop all [`Runner`]s, making the entire [`App`] exit
    ///
    /// This message can be sent using [`Stopper`].
    Exit,
    /// Stop `id`, making it exit
    ///
    /// Note: this is automatically handled for both
    /// [listeners](crate::resource::listener::Listener) and
    /// [watchers](crate::resource::watcher::Watcher),
    /// but custom [`Runner`]s will have to listen to this message themselves.
    ///
    /// This message can be sent using [`Stopper`].
    Stop(NamespacedId),
    /// `id` has begun resolving its dependencies in an [`App`]
    ///
    /// Note: All [resources](mod@crate::resource) will emit this message, even if they won't
    /// provide a [`Runner`] - see [`Self::Running`] instead, if that information is needed.
    Resolving {
        id: NamespacedId,
        bundle: NamespacedId,
    },
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
    /// Note: this only is emitted for resources that have [`Runner`]s, like [listeners](Listener)
    /// and [watchers](Watcher). As simple [Resources](crate::resource::Resource) are never run,
    /// they never emit this message.
    Running(NamespacedId),
    /// `id` has stopped running
    ///
    /// Note: this only is emitted for resources that are [`Runner`]s, like [listeners](Listener)
    /// and [watchers](Watcher). As simple [Resources](crate::resource::Resource) are never run,
    /// they never emit this message - see [`Self::Unloaded`] instead, if that information is
    /// needed.
    Stopped(NamespacedId),
    /// `id` has been unloaded from the app.
    ///
    /// If it is a:
    /// - [`ComponentId::Resource`]: The resource has stopped if it ever started running, and all
    ///   [references](crate::input::get::Get) of it from other [resources](mod@crate::resource)
    ///   have been dropped.
    /// - [`ComponentId::Bundle`]: All of the child components in this bundle have unloaded.
    /// - [`ComponentId::Message`]: All resources that use this message have either unloaded or
    ///   otherwise dropped their handle.
    ///
    /// Note: this will only ever be sent if the `resource-map` feature is enabled
    #[cfg(feature = "resource-map")]
    Unloaded(ComponentId),
    /// A bundle has started resolving its dependencies
    BundleResolving {
        id: NamespacedId,
        parent: Option<NamespacedId>,
        resources: Arc<[NamespacedId]>,
        messages: Arc<[NamespacedId]>,
        bundles: Arc<[NamespacedId]>,
    },
    /// A bundle has started loading into the registry
    BundleLoading {
        id: NamespacedId,
        parent: Option<NamespacedId>,
        resources: Arc<[NamespacedId]>,
        messages: Arc<[NamespacedId]>,
        bundles: Arc<[NamespacedId]>,
    },
    Adopt {
        bundle: NamespacedId,
        component: ComponentId,
    },
    ListenMessageLoaded {
        id: NamespacedId,
        bundle: NamespacedId,
    },
    TakeMessageLoaded {
        id: NamespacedId,
        bundle: NamespacedId,
    },
    /// Resource `dependent` now depends on `dependency` in some way. This should be called
    /// whenever one resources uses another resource's code in any way. For example, this is
    /// emitted when [get](crate::input::get::Get) requests are fulfilled and when
    /// [respond] connections are created. Note that this also means
    /// that this should be emitted when resources accept trait objects or function pointers
    /// from other resources.
    ///
    /// [`ResourceMap`] uses this to determine which resources to unload when another resource
    /// unloads. Specifically, it makes sure that all dependencies of a resource are also unloaded
    /// when it is unloaded.
    ///
    /// This may be emitted multiple times for a single dependency, but it also must be paired with
    /// the same amount of [`DecrementDependency`](Self::DecrementDependency) events. To
    /// automatically handle this, you may use [`dependency::Handle`], which emits this on creation
    /// and [`DecrementDependency`](Self::DecrementDependency) on drop.
    IncrementDependency(dependency::Full),
    /// Resource `dependent` now depends on `dependency` in some way.
    ///
    /// This is sent by [`ResourceMap`] exactly once, right when `dependent` first depends on
    /// `dependency`. The map does this by keeping track of how many
    /// [`IncrementDependency`](Self::IncrementDependency) and
    /// [`DecrementDependency`](Self::DecrementDependency) events happen, sending this when the
    /// dependency level goes from zero to one.
    #[cfg(feature = "resource-map")]
    NewDependency(dependency::Link),
    /// Asks `dependent` to remove its dependency on `dependency`.
    ///
    /// If resources accept external dependencies through their api (for example, by accepting
    /// trait objects), they should allow for removing these dependencies when asked. If they
    /// don't, dependency loops can arise.
    ///
    /// For example, if one resource A uses a library resource B's connection to send a trait
    /// object, A would depend on B through its connection, while B would depend on A through the
    /// object. In this case, B should mark the dependency coming from the object as
    /// [removable](dependency::Full::removable) and listen to the registry message stream for
    /// [`RemoveDependency`](Self::RemoveDependency) messages.
    ///
    /// This message can be sent using [`Stopper`].
    RemoveDependency(dependency::Link),
    /// Resource `dependent` has successfully removed its dependency on `dependency`.
    ///
    /// This may be sent multiple times, but each decrement event must correspond to another
    /// [`IncrementDependency`](Self::IncrementDependency) event. For example, there must never be
    /// more decrement events than increment events for a single dependency link.
    ///
    /// This can be automatically sent by [`dependency::Handle`].
    DecrementDependency(dependency::Full),
    /// Resource `dependent` has fully removed its dependency on `dependency`.
    ///
    /// This is sent by [`ResourceMap`] when `dependent` no longer depends on `dependency` in any
    /// way. That is, for every [`IncrementDependency`](Self::IncrementDependency) event that has
    /// been emitted, there was also a corresponding
    /// [`DecrementDependency`](Self::DecrementDependency) event.
    #[cfg(feature = "resource-map")]
    RemovedDependency(dependency::Link),
    /// There is a [`Get<_>`](crate::input::get::Get) request that must be fulfilled
    ///
    /// Note: this will only ever be sent if the `runtime-loading` feature is enabled
    #[cfg(feature = "runtime-loading")]
    Get(Arc<GetRequest>),
    /// Added a resource to the [`Registry`] that doesn't provide a [`Runner`]
    #[cfg(feature = "runtime-loading")]
    NonRunnable(ResourceSupplier),
    /// The resource `from` emitted `error`
    Error(Error),
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

#[derive(Clone, Debug, Error)]
#[error("resource '{source}' had an error: {error}")]
pub struct Error {
    source: NamespacedId,
    #[source]
    error: Arc<anyhow::Error>,
}

impl Error {
    pub(crate) fn new(source: NamespacedId, error: anyhow::Error) -> Self {
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

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && Arc::ptr_eq(&self.error, &other.error)
    }
}

impl Eq for Error {}

impl std::hash::Hash for Error {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        Arc::as_ptr(&self.error).hash(state);
    }
}

/// A request to all [resources](mod@crate::resource) to fulfil a
/// [`Get<_>`](crate::input::get::Get) input
///
/// Under the hood, this is sent to all running resources as a [`RegistryEvent`]. Each resource
/// checks if the request's id matches their own, and if so, uses
/// [`MaybeGetRef`](crate::input::get::MaybeGetRef) to try to create the reference to send back. If
/// the trait succeeds, they send the result back to the asker.
///
/// `MaybeGetRef` is used because there must be a trait that works for all resources. The
/// underlying runner won't be able to know if the type it represents implements [`GetRef`], so
/// instead it must be able to use a trait that works either way.
#[cfg(feature = "runtime-loading")]
pub struct GetRequest {
    id: NamespacedId,
    dependent: ComponentId,
    // NOTE: This must be a tokio Mutex, not a std one. Why? I'm not really sure!
    // In fact, this was the culprit of what has been the most dreadful bug I have ever encountered
    // due to insane amounts of inconsistency... Deadlocks are not my friend - they very well may be
    // my greatest enemy D:
    sender: tokio::sync::Mutex<Option<oneshot::Sender<Box<dyn crate::input::get::ResourceRef>>>>,
}

#[cfg(feature = "runtime-loading")]
impl GetRequest {
    pub(crate) fn new(
        id: NamespacedId,
        dependent: ComponentId,
    ) -> (
        Self,
        oneshot::Receiver<Box<dyn crate::input::get::ResourceRef>>,
    ) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let get_request = Self {
            id,
            dependent,
            sender: tokio::sync::Mutex::new(Some(sender)),
        };
        (get_request, receiver)
    }

    /// Check if `resource` is the resource that is asked for by the request, and if so, call
    /// [`GetRef`] and send the result back to the original asker.
    #[allow(clippy::missing_panics_doc)] // should resume panic when lock is poisoned
    pub async fn check<R: Resource>(&self, resource: &R, registry: &Registry) {
        self.check_ref(resource, registry).await;
    }

    /// Returns `true` if `R` if the resource that is asked for by this request
    #[inline]
    pub fn matches<R: Resource>(&self) -> bool {
        R::id() == self.id
    }

    /// Check if `resource` is the resource that is asked for by the request, and if so, call
    /// [`GetRef`] and send the result back to the original asker.
    #[allow(clippy::missing_panics_doc)] // should resume panic when lock is poisoned
    pub async fn check_ref(&self, resource: &dyn Resource, registry: &Registry) {
        if !self.matches_ref(resource) {
            return;
        }
        trace!("fulfilling get request");
        #[allow(clippy::option_if_let_else)] // not using the return type...
        if let Ok(reference) = resource
            .maybe_get_ref(registry, self.dependent.as_ref())
            .await
        {
            self.send(reference).await;
        } else {
            warn!("resource matches GetRequest, but MaybeGetRef returned Err");
            self.cancel().await;
        }
    }

    /// Returns `true` if `resource` if the resource that is asked for by this request
    #[inline]
    pub fn matches_ref(&self, resource: &dyn Resource) -> bool {
        resource.dyn_id() == self.id
    }

    /// Returns the [id](NamespacedId) of the resource that the request is asking for
    pub fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    /// Returns the [id](ComponentIdRef) of the component that is asking for this request
    pub fn dependent(&self) -> ComponentIdRef<'_> {
        self.dependent.as_ref()
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
    pub(crate) async fn send(&self, reference: Box<dyn crate::input::get::ResourceRef>) {
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
    resource: std::sync::Mutex<Option<Box<dyn super::LoadedResource>>>,
}

#[cfg(feature = "runtime-loading")]
impl ResourceSupplier {
    /// Get the [`NamespacedId`] of the inner resource
    #[must_use]
    pub fn id(&self) -> &NamespacedIdRef {
        &self.arc.id
    }

    pub(crate) fn new(resource: Box<dyn super::LoadedResource>) -> Self {
        let arc = ResourceSupplierInner {
            id: resource.dyn_id().to_owned(),
            resource: std::sync::Mutex::new(Some(resource)),
        };
        Self { arc: Arc::new(arc) }
    }

    pub(crate) fn take(&self) -> Option<Box<dyn super::LoadedResource>> {
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

/// A message to obtain [`SharedInitFn`s](bundle::SharedInitFn)s of loaded bundles
///
/// Notice that this message is not feature-locked behind `runtime-loading`. This is because
/// resources must be able to accept this message with and without the feature, as the
/// [listener macro](crate::macros::listener) doesn't support conditional components.
#[message("auditeur:bundle_init")]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BundleInit {
    pub(crate) id: NamespacedId,
    #[cfg(feature = "runtime-loading")]
    pub(crate) init: bundle::SharedInitFn,
}

impl BundleInit {
    #[must_use]
    pub fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    #[must_use]
    #[cfg(feature = "runtime-loading")]
    pub fn init(&self) -> Option<bundle::SharedInitFn> {
        Some(self.init.clone())
    }
}
