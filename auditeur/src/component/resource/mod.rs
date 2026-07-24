//! Items that can be added to an [`App`] and potentially run

use std::pin::Pin;

use crate::{
    app::{
        AppFn, AppMutFnOnce, AppMutFuture,
        event::Event,
        map::{Dependency, DependencyId, DependencyType, RequestSet, Waiter},
        registry::{
            EitherCallback, ErasedLoader, Loaded, Loader, LoaderCallback, LoaderCallbacks,
            Registries, Registry,
        },
    },
    component::{
        Component, ComponentIdPath, IdentifiedComponent, IdentifiedType, RegistryOf,
        listen::Listener,
    },
    input::get::{DynGetRefLoader, Get, GetRef, MaybeGetRef},
    prelude::*,
};

use anyhow::{Result, anyhow};
use tokio::select;

pub mod listener;
pub mod pure;
pub mod watcher;

/// An item that can be added to an [`App`]
///
/// Resources are [`Identifiable`] by a specific [`NamespacedId`] that uniquely represents them.
/// If they are a [`Listener`] or [`Watcher`], they may also run code in the main async loop, and
/// if they implement [`GetRef`], other resources may obtain synchronized references of them.
///
/// To implement this trait, use [`#[resource]`](macro@crate::resource)
pub trait Resource:
    IdentifiedType + ResourceParts + MaybeGetRef + Create + crate::Any + Send + Sync + Sized + 'static
{
}

impl<
    T: IdentifiedType
        + ResourceParts
        + MaybeGetRef
        + Create
        + crate::Any
        + Send
        + Sync
        + Sized
        + 'static,
> Resource for T
{
}

pub trait ResourceParts<R: ResourceParts = Self> {
    type Runner: Runner;

    fn runner(
        resource: R,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Runner, Error>> + Send;

    #[must_use]
    fn children() -> Children {
        Children::new()
    }

    const OPTIONS: Options = Options::new();
}

pub trait GetRunnerWith: Send + Sized {
    type Method: ResourceParts<Self>;

    const OPTIONS: Options = Options::new();
}

impl<R: GetRunnerWith> ResourceParts for R {
    type Runner = <R::Method as ResourceParts<Self>>::Runner;

    async fn runner(resource: Self, app: &mut App) -> Result<Self::Runner, Error> {
        <R::Method as ResourceParts<Self>>::runner(resource, app).await
    }

    fn children() -> Children {
        <R::Method as ResourceParts<Self>>::children()
    }

    const OPTIONS: Options =
        <R as GetRunnerWith>::OPTIONS.overlay_on(<R::Method as ResourceParts<Self>>::OPTIONS);
}

pub trait Runner: Send + Sync {
    // NOTE: Why is the runner given an app handle? Well, we have to keep the app alive while the
    // runner is running, so we have to keep around an app handle anyways. Most resources could use
    // that app handle we're keeping around, so we may as well give a reference to it.
    fn run(self, app: &mut App) -> impl Future<Output = ()> + Send + Sync;
}

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
    #[must_use]
    pub const fn new() -> Self {
        Self { weak: None }
    }

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
}

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

impl<R: Resource> Component<Resources> for R {
    type ComponentInput = R::ExternalInput;
    fn register(
        external_input: Self::ComponentInput,
    ) -> (<Resources as Registry>::Loader, Children) {
        (
            ErasedLoader::new(ResourceLoader::<R> { external_input }),
            R::children(),
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
        let runner = async move { runner.run(&mut handle).await };
        let runner = Box::pin(runner);

        Ok(ResourcesLoaded {
            runner: Some(runner),
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
        let ty = if R::OPTIONS.is_weak() {
            DependencyType::Weak
        } else {
            DependencyType::Removable
        };

        LoaderCallbacks::get(ty, async |loaded: &mut ResourcesLoaded, _| {
            Ok(loaded.take_runner())
        })
        .and_then(async |runner| {
            tokio::task::spawn(Dependency::consume(runner, async |runner| {
                if let Some(runner) = runner {
                    runner.await;
                }
            }));
            Ok(())
        })
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
    runner: Option<Pin<Box<dyn Future<Output = ()> + Send + Sync>>>,
    get_ref_loader: Option<Box<dyn DynGetRefLoader>>,

    id: &'static NamespacedIdRef,
    stopper: Shouter<Stop>,
}

impl ResourcesLoaded {
    pub(crate) async fn get(&self, app: &mut App) -> Result<Option<Box<dyn crate::Any>>, Error> {
        let Some(loader) = self.get_ref_loader.as_ref() else {
            return Ok(None);
        };

        let reference = loader.load_ref(app).await?;
        Ok(Some(reference))
    }

    pub(crate) fn take_runner(
        &mut self,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + Sync>>> {
        self.runner.take()
    }
}

impl Loaded for ResourcesLoaded {
    async fn unload(self, _: &mut App) -> Result<(), Error> {
        self.stopper.shout(Stop(self.id));
        Ok(())
    }
}

#[derive(Clone)]
#[listen_message("auditeur:resource_stop")]
struct Stop(&'static NamespacedIdRef);

pub struct StopListener {
    stop_receiver: Listener<Stop>,
    event_receiver: Listener<Event>,
    id: ComponentIdPath,
    stopped: bool,
}

impl StopListener {
    pub async fn wait(&mut self) {
        if self.stopped {
            return;
        }

        loop {
            select! {
                Some(Stop(id)) = self.stop_receiver.listen() => {
                    if *self.id == id {
                        break;
                    }
                }
                Some(event) = self.event_receiver.listen() => {
                    if let Event::RequestDependencyRemoval { id: DependencyId {
                        dependent,
                        dependency,
                    }} = event
                        && dependent == App::component_id()
                        && dependency.registry == Resources::id()
                        && dependency.component == self.id
                    {
                        break;
                    }
                }
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
            event_receiver: Listener::create(app).await?,
            id: dependent.component,
            stopped: false,
        })
    }
}

/// A resource that has custom initialization logic for when it is started.
///
/// This is made to be used by [runner implementations](GetRunnerWith) for its specialization
/// capabilities (see [`init_specialization`](macro@init_specialization)). As of now, only the
/// [`listener`](macro@listener) macro uses this.
pub trait Init {
    fn init(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send + Sync;
}

#[doc(hidden)]
pub mod init_specialization {
    use crate::component::resource::Init;

    pub struct DefaultTag;
    pub trait DefaultKind {
        fn kind(self) -> DefaultTag;
    }

    impl DefaultTag {
        #[expect(clippy::unused_async)]
        pub async fn init<T: Send + Sync>(self, _: &mut T) -> anyhow::Result<()> {
            Ok(())
        }
    }

    impl<T: Send + Sync> DefaultKind for &mut T {
        fn kind(self) -> DefaultTag {
            DefaultTag
        }
    }

    pub struct PresentTag;
    pub trait PresentKind {
        fn kind(self) -> PresentTag;
    }

    impl PresentTag {
        pub async fn init<T: Init + Send + Sync>(self, value: &mut T) -> anyhow::Result<()> {
            value.init().await
        }
    }

    impl<T: Init + Send + Sync> PresentKind for &&mut T {
        fn kind(self) -> PresentTag {
            PresentTag
        }
    }
}

/// Calls the [`Init::init`] method of a resource if it exists, or runs a default implementation.
///
/// The default implementation is to do nothing and return [`Ok(())`].
///
/// # Examples
///
/// ```
/// # use auditeur::*; #[tokio::main] async fn main() -> anyhow::Result<()> {
/// use auditeur::prelude::*;
///
/// #[derive(Default)]
/// struct Foo {
///     initialized: bool,
/// }
///
/// impl Init for Foo {
///     async fn init(&mut self) -> anyhow::Result<()> {
///         self.initialized = true;
///         Ok(())
///     }
/// }
///
/// let mut foo = Foo::default();
/// init_specialization!(&mut foo).await?;
/// assert!(foo.initialized);
/// # Ok(()) }
/// ```
#[macro_export]
macro_rules! init_specialization {
    ($this:expr) => {{
        #[allow(unused_imports)]
        use $crate::component::resource::init_specialization::{DefaultKind, PresentKind};
        let this = $this;
        (&this).kind().init(this)
    }};
}

/// A resource that has custom exiting logic for when it is stopped.
///
/// This is made to be used by [runner implementations](GetRunnerWith) for its specialization
/// capabilities (see [`exit_specialization`](macro@exit_specialization)). As of now, only the
/// [`listener`](macro@listener) macro uses this.
pub trait Exit {
    fn exit(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send + Sync;
}

#[doc(hidden)]
pub mod exit_specialization {
    use crate::component::resource::Exit;

    pub struct DefaultTag;
    pub trait DefaultKind {
        fn kind(self) -> DefaultTag;
    }

    impl DefaultTag {
        #[expect(clippy::unused_async)]
        pub async fn exit<T: Send + Sync>(self, _: &mut T) -> anyhow::Result<()> {
            Ok(())
        }
    }

    impl<T: Send + Sync> DefaultKind for &mut T {
        fn kind(self) -> DefaultTag {
            DefaultTag
        }
    }

    pub struct PresentTag;
    pub trait PresentKind {
        fn kind(self) -> PresentTag;
    }

    impl PresentTag {
        pub async fn exit<T: Exit + Send + Sync>(self, value: &mut T) -> anyhow::Result<()> {
            value.exit().await
        }
    }

    impl<T: Exit + Send + Sync> PresentKind for &&mut T {
        fn kind(self) -> PresentTag {
            PresentTag
        }
    }
}

/// Calls the [`Exit::exit`] method of a resource if it exists, or runs a default implementation.
///
/// The default implementation is to do nothing and return [`Ok(())`].
#[macro_export]
macro_rules! exit_specialization {
    ($this:expr) => {{
        #[allow(unused_imports)]
        use $crate::component::resource::exit_specialization::{DefaultKind, PresentKind};
        let this = $this;
        (&this).kind().exit(this)
    }};
}

#[must_use]
pub struct AppFnResourcesMut;
impl AppFnResourcesMut {
    pub fn stop_id(self, id: ComponentIdPath) -> impl AppMutFnOnce<Output = Waiter> {
        AppFn.map_mut().request(RequestSet::unload(ComponentId {
            registry: Resources::id(),
            component: id,
        }))
    }

    pub fn stop<R: Resource>(self) -> impl AppMutFnOnce<Output = Waiter> {
        self.stop_id(ComponentIdPath::Static(R::id()))
    }
}

pub struct AppResourcesMut<'a> {
    app: &'a mut App,
}

impl<'a> AppResourcesMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// # Errors
    ///
    /// - Propagated from [`AppRegistriesMut::get`](crate::app::registry::AppRegistriesMut::get)
    pub async fn get<R: Resource + GetRef>(self) -> Result<Get<R>, Error> {
        let dependency = self
            .app
            .registries_mut()
            .get::<Resources, _>(
                ComponentIdPath::Static(R::id()),
                async move |loaded, app| {
                    let reference = loaded.get(app).await?;
                    let reference = reference.ok_or_else(|| Error::ResourceNoGetRef {
                        id: R::id().to_owned(),
                    })?;
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

    pub fn stop<R: Resource>(self) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.resources_mut().stop::<R>().call(self.app)
    }

    pub fn stop_id(self, id: ComponentIdPath) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.resources_mut().stop_id(id).call(self.app)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{app::map::RequestSet, prelude::*};
    use auditeur_macros::pure_resource;

    use super::{Exit, Init};

    #[tokio::test]
    async fn init_specialized() -> anyhow::Result<()> {
        #[derive(Default)]
        struct A {
            initialized: bool,
        }

        impl Init for A {
            async fn init(&mut self) -> anyhow::Result<()> {
                self.initialized = true;
                Ok(())
            }
        }

        let mut a = A::default();
        init_specialization!(&mut a).await?;
        assert!(a.initialized);

        Ok(())
    }

    #[tokio::test]
    async fn init_unspecialized() -> anyhow::Result<()> {
        struct B;

        let mut b = B;
        init_specialization!(&mut b).await?;
        Ok(())
    }

    #[tokio::test]
    async fn exit_specialized() -> anyhow::Result<()> {
        #[derive(Default)]
        struct A {
            exited: bool,
        }

        impl Exit for A {
            async fn exit(&mut self) -> anyhow::Result<()> {
                self.exited = true;
                Ok(())
            }
        }

        let mut a = A::default();
        exit_specialization!(&mut a).await?;
        assert!(a.exited);

        Ok(())
    }

    #[tokio::test]
    async fn exit_unspecialized() -> anyhow::Result<()> {
        struct B;

        let mut b = B;
        exit_specialization!(&mut b).await?;
        Ok(())
    }

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
            .registries_mut()
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
            .map_mut()
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
