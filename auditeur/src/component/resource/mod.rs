//! [`Component`](super::Component)s that can provide [tasks](Runner) to be run and/or
//! [handles](Get) to their state.
//!
//! [`Resource`]s are [`Identified`] by a specific [`NamespacedId`] that uniquely represents them.
//! If they are a [`Listener`] or [`Watcher`], they may also run code in the main async loop, and
//! if they implement [`GetRef`], other resources may obtain synchronized references of them.
//!
//! To implement this trait, use [`#[resource]`](macro@crate::resource) and either:
//! - Call one of the predefined resource macros,
//!     - [`#[listener]`](macro@crate::listener),
//!     - [`#[watcher]`](macro@crate::watcher),
//!     - [`#[pure_resource]`](macro@crate::pure_resource),
//! - Implement [`GetRunnerWith`] to use a default runner implementation,
//! - Implement [`ResourceParts`] manually,
//!
//! To provide a handle type, implement [`GetRef`] (either manually or through [`respond`]).
//! Then, [`#[resource]`](macro@crate::resource) will automatically pick this implementation up and
//! make it accessible through the [`Get`] input.
//!
//! # Examples
//!
//! See the module documentation of [`listener`](mod@listener).

use crate::{
    any,
    app::{
        AppFn, AppMutFnOnce, AppMutFuture,
        event::Event,
        map::{Dependency, DependencyType, RequestSet, Waiter},
    },
    component::{
        ComponentIdPath, IdentifiedComponent, IdentifiedType, Register, RegistryOf,
        listen::Listener,
        registry::{
            EitherCallback, ErasedLoader, Loaded, Loader, LoaderCallback, LoaderCallbacks,
            Registries, Registry,
        },
        resource::get::{DynGetRefLoader, Get, GetRef, MaybeGetRef},
    },
    prelude::*,
};

use anyhow::{Context, Result, anyhow};
use tracing::{Instrument, debug};

pub mod get;
pub mod listener;
pub mod pure;
pub mod watcher;

/// Components that can provide [tasks](Runner) to be run and/or [handles](Get) to their state.
///
/// See the [module documentation](self) for more information.
pub trait Resource:
    IdentifiedType + ResourceParts + MaybeGetRef + Create + any::Any + Send + Sync + Sized + 'static
{
}

impl<
    T: IdentifiedType
        + ResourceParts
        + MaybeGetRef
        + Create
        + any::Any
        + Send
        + Sync
        + Sized
        + 'static,
> Resource for T
{
}

/// The implementation-defined parts of the [`Resource`] trait.
///
/// The generic parameter `R` is the type of the resource. If implementing this trait manually,
/// this should be [`Self`] (the default). If creating a [`Method`](GetRunnerWith::Method) for
/// [`GetRunnerWith`], this should be the resource type.
pub trait ResourceParts<R: ResourceParts = Self> {
    type Runner: Runner;

    /// Creates the [`Runner`] for this resource, wrapping it up into a task.
    fn runner(
        resource: R,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Runner, Error>> + Send;

    /// Provides any children that this resource should be added with.
    ///
    /// The default implementation adds no children.
    ///
    /// This method is run exactly once when the resource is [added](Children::with) to a
    /// [`Children`] collection.
    #[must_use]
    fn children(children: Children) -> Children {
        children
    }

    /// Any [`Options`] to adjust the functionality of this resource.
    const OPTIONS: Options = Options::new();
}

/// Resources that use default implementations for [`ResourceParts`].
pub trait GetRunnerWith: Send + Sized {
    /// The default implementation of [`ResourceParts`] to use.
    ///
    /// For example,
    /// - [`listener::ListenerMethod`],
    /// - [`watcher::WatcherMethod`],
    /// - [`pure::PureMethod`],
    type Method: ResourceParts<Self>;

    /// Provides any children that this resource should be added with.
    #[must_use]
    fn children(children: Children) -> Children {
        children
    }

    /// Optional options that overlay on the options obtained from [`Self::Method`].
    const OPTIONS: Options = Options::new();
}

impl<R: GetRunnerWith> ResourceParts for R {
    type Runner = <R::Method as ResourceParts<Self>>::Runner;

    async fn runner(resource: Self, app: &mut App) -> Result<Self::Runner, Error> {
        <R::Method as ResourceParts<Self>>::runner(resource, app).await
    }

    fn children(children: Children) -> Children {
        let children = <R as GetRunnerWith>::children(children);
        <R::Method as ResourceParts<Self>>::children(children)
    }

    const OPTIONS: Options =
        <R as GetRunnerWith>::OPTIONS.overlay_on(<R::Method as ResourceParts<Self>>::OPTIONS);
}

/// A task obtained from a [`Resource`] to be run in the async executor.
///
/// The resource cannot finish unloading until this task finishes. The task is expected to
/// automatically stop by listening to a [`StopListener`]. This is done by default for
/// [`listener`](mod@listener)s, but not [`watcher`](mod@watcher)s.
///
/// By default, this also keeps the resource loaded unless it is requested to be stopped. To make
/// it not keep the resource alive by default, use [`Options::weak`].
pub trait Runner: Send {
    // NOTE: Why is the runner given an app handle? Well, we have to keep the app alive while the
    // runner is running, so we have to keep around an app handle anyways. Most resources could use
    // that app handle we're keeping around, so we may as well give a reference to it.
    /// Runs this task to completion, using an app handle if necessary.
    fn run(self, app: &mut App) -> impl Future<Output = ()> + Send;
}

/// Options that can adjust the functionality of a [`Resource`].
#[derive(Clone, Copy)]
pub struct Options {
    weak: Option<bool>,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// Creates a new options struct that uses default implementations for everything.
    #[must_use]
    pub const fn new() -> Self {
        Self { weak: None }
    }

    /// Sets the `weak` value for these options.
    ///
    /// If this is `false`, the resource will automatically be loaded when added to the app, and
    /// its [`Runner`] will keep it loaded in the app. If this is `true`, neither of these will
    /// happen.
    ///
    /// Default: `false`.
    #[must_use]
    pub const fn weak(self, weak: bool) -> Self {
        Self { weak: Some(weak) }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    const fn is_weak(&self) -> bool {
        if let Some(weak) = self.weak {
            weak
        } else {
            false
        }
    }

    /// Overlays the options of `self` onto the options of `other`.
    ///
    /// This is identical to `other.overlay(self)`.
    #[must_use]
    pub const fn overlay_on(self, other: Self) -> Self {
        Self {
            weak: if let Some(weak) = self.weak {
                Some(weak)
            } else {
                other.weak
            },
        }
    }

    /// Overlays the options of `other` onto the options of `self`.
    ///
    /// Set options are prioritized over unset options. That is, if `other` was obtained using
    /// only [`Self::new`] (and no options were set), this will do nothing.
    #[must_use]
    pub const fn overlay(self, other: Self) -> Self {
        other.overlay_on(self)
    }
}

/// The [`Registry`] for [`Resource`]s.
///
/// This is automatically added to every [`App`].
#[identified("auditeur:resources")]
pub struct Resources;
impl Registry for Resources {
    type Loader = ErasedLoader<ResourcesLoaded>;
    type Loaded = ResourcesLoaded;

    fn children(children: Children) -> Children {
        children.with::<Stop>(())
    }
}

impl RegistryOf for Resources {
    type Registry = Registries;
}

impl<R: Resource> Register<R> for Resources {
    type ComponentInput = R::ExternalInput;
    fn register(external_input: Self::ComponentInput) -> (<Self as Registry>::Loader, Children) {
        (
            ErasedLoader::new(ResourceLoader::<R> { external_input }),
            R::children(Children::new()),
        )
    }
}

struct ResourceLoader<R: Resource> {
    external_input: R::ExternalInput,
}

impl<R: Resource> Loader<ErasedLoader<ResourcesLoaded>> for ResourceLoader<R> {
    type Loaded = ResourcesLoaded;

    async fn load(&self, app: &mut App) -> Result<ResourcesLoaded, Error> {
        let stopper = Shouter::create(app).await?;
        let resource = R::create(&self.external_input, app).await?;
        let get_ref_loader = resource.maybe_get_ref(app).await?;

        let runner = R::runner(resource, app).await?;
        // NOTE: This is the handle that keeps the app alive as long as the runner is running.
        let mut handle = app.handle().await;
        let handle = tokio::task::spawn(
            async move {
                debug!("starting");
                runner.run(&mut handle).await;
                debug!("exited");
            }
            .instrument(info_span!("run")),
        );

        Ok(ResourcesLoaded {
            handle,
            get_ref_loader,
            id: R::id(),
            stopper,
        })
    }

    fn on_add(&self) -> impl LoaderCallback<ErasedLoader<ResourcesLoaded>, Output = ()> {
        if R::OPTIONS.is_weak() {
            EitherCallback::Left(LoaderCallbacks::none())
        } else {
            EitherCallback::Right(LoaderCallbacks::load_lazy(DependencyType::Transient).discard())
        }
    }

    fn on_load(&self) -> impl LoaderCallback<ErasedLoader<ResourcesLoaded>, Output = ()> {
        if R::OPTIONS.is_weak() {
            return EitherCallback::Left(
                LoaderCallbacks::load(DependencyType::Transient).discard(),
            );
        }

        EitherCallback::Right(
            LoaderCallbacks::get(
                DependencyType::Removable,
                async |_: &mut ResourcesLoaded, app: &mut App| app.get_event_listener().await,
            )
            .and_then(async |mut listener| {
                tokio::task::spawn(async move {
                    let info = Dependency::info_of(&listener);
                    let this_id = info.id.clone().into_id();

                    loop {
                        let event = listener.listen().await;
                        if let Event::RequestDependencyRemoval { id } = event
                            && id == this_id
                        {
                            break;
                        }
                    }
                });
                Ok(())
            }),
        )
    }
}

impl<R: Resource> IdentifiedComponent for ResourceLoader<R> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId::within::<Resources, R>()
    }
}

#[doc(hidden)]
#[derive(Any)]
pub struct ResourcesLoaded {
    get_ref_loader: Option<Box<dyn DynGetRefLoader>>,

    handle: tokio::task::JoinHandle<()>,
    id: &'static NamespacedIdRef,
    stopper: Shouter<Stop>,
}

impl ResourcesLoaded {
    pub(crate) async fn get(&self, app: &mut App) -> Result<Option<Box<dyn any::Any>>, Error> {
        let Some(loader) = self.get_ref_loader.as_ref() else {
            return Ok(None);
        };

        let reference = loader.load_ref(app).await?;
        Ok(Some(reference))
    }
}

impl Loaded for ResourcesLoaded {
    async fn unload(self, app: &mut App) -> Result<(), Error> {
        self.stopper.shout(Stop(self.id));
        self.handle
            .await
            .context(format!("resource '{}' failed to run", self.id))
            .map_err(|err| app.errors().wrap(err))?;
        Ok(())
    }
}

#[derive(Clone)]
#[listen_message("auditeur:resource_stop")]
struct Stop(&'static NamespacedIdRef);

/// An [`Input`] that provides a method that can tell when a resource's runner should stop.
pub struct StopListener {
    stop_receiver: Listener<Stop>,
    id: ComponentIdPath,
    stopped: bool,
}

impl StopListener {
    /// Waits for the app to tell the runner to stop.
    pub async fn wait(&mut self) {
        if self.stopped {
            return;
        }

        loop {
            let Stop(id) = self.stop_receiver.listen().await;
            if *self.id == id {
                break;
            }
        }

        self.stopped = true;
    }
}

impl Input for StopListener {
    async fn create(app: &mut App) -> Result<Self, Error> {
        let dependent = app.dependent().dependent_id().into_id();
        if dependent.registry != Resources::id() {
            return Err(app.errors().wrap(anyhow!(
                "the dependent for a StopListener should be in the '{}' registry, but it was actually in the '{}' registry",
                Resources::id(),
                dependent.registry
            )));
        }

        Ok(Self {
            stop_receiver: Listener::create(app).await?,
            id: dependent.component,
            stopped: false,
        })
    }
}

/// The return type of [`AppFn::resources_mut`]. See [`AppResourcesMut`].
#[must_use]
pub struct AppFnResourcesMut;
impl AppFnResourcesMut {
    /// See [`AppResourcesMut::stop`].
    pub fn stop<R: Resource>(self) -> impl AppMutFnOnce<Output = Waiter> {
        self.stop_id(ComponentIdPath::Static(R::id()))
    }

    /// See [`AppResourcesMut::stop_id`].
    pub fn stop_id(self, id: ComponentIdPath) -> impl AppMutFnOnce<Output = Waiter> {
        AppFn
            .components_mut()
            .request(RequestSet::unload(ComponentId {
                registry: Resources::id(),
                component: id,
            }))
    }
}

/// Methods for [`App`] to manage [`Resource`]s.
pub struct AppResourcesMut<'a> {
    app: &'a mut App,
}

impl<'a> AppResourcesMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Gets the handle of a resource added to the registry that implements [`GetRef`].
    ///
    /// # Errors
    ///
    /// - Propagated from [`AppComponentsMut::get`](crate::app::AppComponentsMut::get)
    #[allow(clippy::missing_panics_doc)]
    pub async fn get<R: Resource + GetRef>(self) -> Result<Get<R>, Error> {
        let dependency = (self.app.components_mut())
            .get::<Resources, _>(
                ComponentIdPath::Static(R::id()),
                async move |loaded, app| {
                    let reference = (loaded.get(app).await?)
                        .expect("this should never return None because `R` implements GetRef");
                    let reference: Box<R::Ref> =
                        crate::any::downcast(reference).map_err(|_| Error::DowncastFailed {
                            id: ComponentId::within::<Resources, R>(),
                        })?;
                    Ok(*reference)
                },
            )
            .await?;
        Ok(Get::new(dependency))
    }

    /// Requests for the resource `R` to be unloaded, including stopping its runner.
    pub fn stop<R: Resource>(self) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.resources_mut().stop::<R>().call(self.app)
    }

    /// Requests for the resource with id `id` to be unloaded, including stopping its runner.
    pub fn stop_id(self, id: ComponentIdPath) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.resources_mut().stop_id(id).call(self.app)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{app::map::RequestSet, prelude::*};
    use auditeur_macros::pure_resource;

    #[tokio::test]
    async fn weak_resource_exits() -> Result<(), Error> {
        #[resource("test:resource")]
        #[pure_resource]
        #[create(default)]
        struct TestResource;

        let app = App::builder().with::<TestResource>(()).spawn();

        crate::test::finish(app, false).await
    }

    #[tokio::test]
    async fn weak_resource_stops_itself() -> Result<(), Error> {
        #[resource("test:resource")]
        #[listener(weak)]
        #[create(default)]
        struct TestResource;

        let mut app = App::builder().with::<TestResource>(()).spawn();

        let dependency = app
            .components_mut()
            .load(TestResource::component_id())
            .await?;
        drop(dependency);

        crate::test::finish(app, false).await
    }

    #[tokio::test]
    async fn strong_resource_keeps_alive() {
        #[resource("test:resource")]
        #[listener]
        #[create(default)]
        struct TestResource;

        let mut app = App::builder().with::<TestResource>(()).spawn();

        let waiter = app
            .components_mut()
            .wait_for(RequestSet::remove(App::component_id()))
            .await;
        // Release our dependency on the app so that we don't keep it alive.
        app.release_dependency_unsafe();

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiter.is_ready());

        app.resources_mut().stop::<TestResource>().await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(waiter.is_ready());
    }
}
