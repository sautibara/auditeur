#![allow(private_interfaces, private_bounds)]
//! The backend of the library; the way that components actually interact.

use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use futures::FutureExt;
use tokio::sync::broadcast;
use tracing::debug;

use crate::app::component::{AppComponentsMut, AppFnComponentsMut};
use crate::app::error::{AppFnErrors, AppFnErrorsMut};
use crate::app::event::FlushRequest;
use crate::app::map::{
    AppFnMap, Channels, DependencyHandle, LoadedComponentId, LoadedDependencyId, RequestSet,
};
use crate::component::bundle::Bundles;
use crate::component::registry::LoaderCallback;
use crate::component::resource::AppFnResourcesMut;
use crate::component::{IdentifiedComponent, IdentifiedComponentType, Register};
use crate::input::CloneIn;
use crate::prelude::*;
use crate::{
    app::{
        error::{AppErrors, AppErrorsMut, Errors},
        map::{AddedComponentId, AppMap, Dependency, DependencyPart, DependencyType, Map, Waiter},
        registrar::Registrar,
    },
    component::{
        HasChildren, RegistryOf,
        listen::{self, AppListenMut, ListenMessages, Listener},
        registry::{ErasedLoader, Loaded, Loader, LoaderCallbacks, Registries, Registry},
        resource::{AppResourcesMut, Resources},
        take::{AppTakeMut, TakeMessages},
    },
};

pub mod component;
pub mod error;
pub mod event;
pub mod map;
pub(crate) mod registrar;

mod future;
pub use future::*;

pub use event::Event;
pub use registrar::ErasedComponent;

/// A handle to the shared backend state of the app; the way that components interact.
///
/// This contains:
/// - A shared errors list, accessible through [`Self::errors`] and [`Self::errors_mut`],
/// - [Registries](crate::component::registry::Registry), containing all added
///   [components](crate::component::Component), accessible through [`Self::components_mut`].
/// - A map of the relationships and dependencies between components,
///   accessible through [`Self::map`].
///
/// The [`App`] component manages the lifecycle of the app. Every [`App`] handle holds a dependency
/// on the component, so keeping a handle alive will keep the app alive. If all handles are dropped,
/// the app will automatically attempt to exit by requesting its removal. This will unload all
/// components in the app (since the app component is the parent of all other components), and
/// finally report a successful exit once all unloads are finished.
#[identified("auditeur:app")]
#[derive(Debug, crate::Any)]
pub struct App {
    variant: Inner,
    handle: HandleData,
}

#[derive(Debug)]
struct HandleData {
    method_sender: flume::Sender<Method>,
    dependency: Dependency<()>,
    new_dependency_type: DependencyType,
}

#[derive(Debug)]
enum Inner {
    Owned(Box<Owned>),
    Handle,
}

/// The actual shared state of the app.
#[derive(Debug)]
struct Owned {
    registrar: Registrar,
    errors: Errors,
    map: Map,

    event_sender: broadcast::Sender<Event>,
    event_sender_dependency: Option<Dependency<()>>,
}

impl App {
    /// Start building a new app.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Get a new handle to the app.
    pub fn handle(&mut self) -> impl AppMutFuture<'_, Output = Self> {
        AppFn.handle().call(self)
    }

    /// Tell the app to exit.
    ///
    /// This unloads and removes all components in the app, leading it to exit. Once that succeeds,
    /// the [`Waiter`] returned will be released.
    ///
    /// WARNING: The app will only exit once all components are unloaded. App handles keep the app
    /// loaded, so awaiting the returned waiter while holding on to an app handle will deadlock.
    pub fn exit(&mut self) -> impl AppMutFuture<'_, Output = Waiter> {
        AppFn.exit().call(self)
    }

    /// Consumes this handle and waits for the app to exit.
    ///
    /// Note that this doesn't request for the app to exit, it just waits for it. As such, this is
    /// most useful to be called right after [`Builder::spawn`].
    pub async fn wait_for_exit(mut self) {
        let waiter = (self.components_mut())
            .wait_for(RequestSet::remove(Self::component_id()))
            .await;
        drop(self);

        waiter.wait().await;
    }
}

/// Methods relating to builtin components:
/// [`Resource`](crate::component::resource::Resource)s,
/// [`ListenMessage`](crate::component::listen::ListenMessage)s, and
/// [`TakeMessage`](crate::component::take::TakeMessage)s.
impl App {
    #[must_use]
    pub const fn resources_mut(&mut self) -> AppResourcesMut<'_> {
        AppResourcesMut::new(self)
    }

    #[must_use]
    pub const fn listen_mut(&mut self) -> AppListenMut<'_> {
        AppListenMut::new(self)
    }

    #[must_use]
    pub const fn take_mut(&mut self) -> AppTakeMut<'_> {
        AppTakeMut::new(self)
    }
}

/// Methods relating to this handle's current [`Dependent`]: the component that will keep alive all
/// dependencies returned by this app handle.
impl App {
    #[must_use]
    pub const fn dependent(&self) -> Dependent<'_> {
        Dependent::new(self)
    }

    #[must_use]
    pub const fn dependent_mut(&mut self) -> DependentMut<'_, (), ()> {
        DependentMut::new(self)
    }
}

/// Methods that access the main internals of the app: the [registries](component), the [map] of
/// added components, and the [error list](mod@error).
impl App {
    #[must_use]
    pub const fn components_mut(&mut self) -> AppComponentsMut<'_> {
        AppComponentsMut::new(self)
    }

    #[must_use]
    pub const fn errors(&self) -> AppErrors<'_> {
        AppErrors::new(self)
    }

    #[must_use]
    pub const fn errors_mut(&mut self) -> AppErrorsMut<'_> {
        AppErrorsMut::new(self)
    }

    #[must_use]
    pub const fn map(&self) -> AppMap<'_> {
        AppMap::new(self)
    }

    /// Gets an event listener from the app.
    ///
    /// This is identical to [`AppListenMut::listener`], except that this returns an
    /// [`AppMutFuture`], allowing it to be composed with other methods.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the event message channel has been unloaded.
    ///     - This is only possible if the app has unloaded and is thus in the process of exiting.
    pub fn get_event_listener(
        &mut self,
    ) -> impl AppMutFuture<'_, Output = Result<Listener<Event>, Error>> {
        AppFn.get_event_listener().call(self)
    }

    fn get_event_listener_as(
        &mut self,
        dependent: LoadedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFuture<'_, Output = Result<Listener<Event>, Error>> {
        AppFn.get_event_listener_as(dependent, ty).call(self)
    }

    fn release_event_listener(&mut self) -> impl AppMutFuture<'_, Output = ()> {
        AppFn.release_event_listener().call(self)
    }

    #[cfg(test)]
    pub(crate) fn release_dependency_unsafe(&mut self) {
        Dependency::release_unsafe(&mut self.handle.dependency);
    }
}

impl CloneIn for App {
    async fn clone_in(&self, app: &mut App) -> Self {
        app.handle().await
    }
}

pub struct Dependent<'a> {
    app: &'a App,
}

impl HandleData {
    fn dependent_id(&self) -> LoadedComponentId {
        self.dependency.info().id.dependent()
    }

    fn app_id(&self) -> LoadedComponentId {
        self.dependency.info().id.dependency()
    }
}

impl<'a> Dependent<'a> {
    const fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Returns the id of the component that [dependencies](Dependency) obtained from this app will
    /// have as the dependent.
    #[must_use]
    pub fn dependent_id(self) -> LoadedComponentId {
        self.app.handle.dependent_id()
    }

    /// Returns the id of the app component.
    #[must_use]
    pub fn app_id(self) -> LoadedComponentId {
        self.app.handle.app_id()
    }

    /// Returns the [`DependencyType`] that [dependencies](Dependency) obtained from this app will
    /// use.
    #[must_use]
    pub const fn dependency_type(self) -> DependencyType {
        self.app.handle.new_dependency_type
    }
}

#[doc(hidden)]
pub struct Present;

pub struct DependentMut<'a, D, T> {
    inner: DependentMutInner<'a>,
    pd: PhantomData<(D, T)>,
}

pub struct DependentMutInner<'a> {
    app: &'a mut App,
    dependency: Option<Dependency<()>>,
    ty: Option<DependencyType>,
}

impl<'a> DependentMut<'a, (), ()> {
    const fn new(app: &'a mut App) -> Self {
        Self {
            inner: DependentMutInner {
                app,
                dependency: None,
                ty: None,
            },
            pd: PhantomData,
        }
    }
}

impl<'a, T> DependentMut<'a, (), T> {
    pub async fn with_dependent<D: Send + Sync>(
        self,
        dependency: &Dependency<D>,
    ) -> DependentMut<'a, Present, T> {
        let dependent = Dependency::info_of(dependency).id.dependency();
        self.with_dependent_inner(dependent).await
    }

    async fn with_dependent_inner(
        self,
        dependent: LoadedComponentId,
    ) -> DependentMut<'a, Present, T> {
        let dependency = self.inner.app.handle.app_id();
        let part = DependencyPart {
            id: LoadedDependencyId::new(dependent, dependency),
            // All handles have an unremovable dependency on the app.
            ty: DependencyType::Unremovable,
        };
        let dependency = (self.inner.app)
            .components_mut()
            .increment_dependency(part)
            .await;
        self.with_dependency(dependency)
    }

    fn with_dependency(mut self, dependency: Dependency<()>) -> DependentMut<'a, Present, T> {
        self.inner.dependency = Some(std::mem::replace(
            &mut self.inner.app.handle.dependency,
            dependency,
        ));

        DependentMut {
            inner: self.inner,
            pd: PhantomData,
        }
    }
}

impl<'a, D> DependentMut<'a, D, ()> {
    #[must_use]
    pub fn with_ty(mut self, ty: DependencyType) -> DependentMut<'a, D, Present> {
        self.inner.ty = Some(std::mem::replace(
            &mut self.inner.app.handle.new_dependency_type,
            ty,
        ));

        DependentMut {
            inner: self.inner,
            pd: PhantomData,
        }
    }
}

impl Drop for DependentMutInner<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.dependency.take() {
            self.app.handle.dependency = id;
        }

        if let Some(ty) = self.ty.take() {
            self.app.handle.new_dependency_type = ty;
        }
    }
}

impl<D, T> Deref for DependentMut<'_, D, T> {
    type Target = App;
    fn deref(&self) -> &Self::Target {
        self.inner.app
    }
}

impl<D, T> DerefMut for DependentMut<'_, D, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.app
    }
}

#[must_use = "an app builder does nothing unless it is run"]
pub struct Builder {
    children: Children,
}

impl Builder {
    fn new() -> Self {
        Self {
            children: Children::new(),
        }
    }

    #[expect(clippy::missing_panics_doc)]
    #[allow(clippy::must_use_candidate)] // no?
    pub fn spawn(self) -> App {
        let mut owned = Owned::default();
        let (method_sender, method_receiver) = flume::unbounded();
        let app_loaded_id = owned
            .initialize(self.children, &method_sender)
            .expect("app shouldn't fail to initialize");

        // Construct the handle dependency first to keep the app alive.
        let handle_dependency = DependencyPart {
            id: LoadedDependencyId::new(app_loaded_id.clone(), app_loaded_id),
            // Transient b/c main thread is expected to drop the handle.
            ty: DependencyType::Transient,
        };

        let handle_app = App {
            handle: HandleData {
                method_sender: method_sender.clone(),
                dependency: owned.increment_dependency(handle_dependency, &method_sender),
                new_dependency_type: DependencyType::Transient,
            },
            variant: Inner::Handle,
        };

        // Then the owned dependency now that the app is being kept alive.
        let dependency = Dependency {
            value: (),
            handle: DependencyHandle::empty_unsafe(),
        };

        let owned_app = App {
            handle: HandleData {
                method_sender,
                dependency,
                new_dependency_type: DependencyType::Unremovable,
            },
            variant: Inner::Owned(Box::new(owned)),
        };

        // And start the owned app running in the background.
        let runner = AppRunner {
            app: owned_app,
            method_receiver,
        };
        tokio::task::spawn(runner.run());

        handle_app
    }

    pub async fn run(self) {
        self.spawn().wait_for_exit().await;
    }
}

impl HasChildren for Builder {
    fn with_erased(mut self, component: ErasedComponent, children: Children) -> Self {
        self.children = self.children.with_erased(component, children);
        self
    }
}

pub struct AppFn;
impl AppFn {
    pub const fn components_mut(self) -> AppFnComponentsMut {
        AppFnComponentsMut
    }

    pub const fn map(self) -> AppFnMap {
        AppFnMap
    }

    pub const fn errors(self) -> AppFnErrors {
        AppFnErrors
    }

    pub const fn errors_mut(self) -> AppFnErrorsMut {
        AppFnErrorsMut
    }

    pub const fn resources_mut(self) -> AppFnResourcesMut {
        AppFnResourcesMut
    }
}

impl AppFn {
    pub const fn handle(self) -> impl AppMutFnOnce<Output = App> {
        Self::using_handle_mut(|handle| {
            let method_sender = handle.method_sender.clone();
            let new_dependency_type = handle.new_dependency_type;
            let part = handle.dependency.info().clone();

            Self.components_mut()
                .increment_dependency(part)
                .map(move |dependency| {
                    HandleData {
                        method_sender,
                        dependency,
                        new_dependency_type,
                    }
                    .into_handle()
                })
        })
    }

    pub const fn exit(self) -> impl AppMutFnOnce<Output = Waiter> {
        Self::using_handle_mut(|_| {
            Self.components_mut()
                .request(RequestSet::remove(App::component_id()))
        })
    }
}

impl AppFn {
    /// See [`App::get_event_listener`].
    pub const fn get_event_listener(
        self,
    ) -> impl AppMutFnOnce<Output = Result<Listener<Event>, Error>> {
        Self::using_handle_mut(|handle| {
            Self.get_event_listener_as(handle.dependent_id(), handle.new_dependency_type)
        })
    }

    #[expect(clippy::unused_self)]
    const fn get_event_listener_as(
        self,
        dependent: LoadedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFnOnce<Output = Result<Listener<Event>, Error>> {
        Self::oneshot_mut(
            (dependent, ty),
            |(dependent, ty), sender, _| Method::GetEventListener {
                dependent,
                ty,
                sender,
            },
            |(dependent, ty), owned, handle| {
                // Get the dependency id that was stored when the app was created.
                let dependency = owned.event_sender_dependency.as_ref().ok_or_else(|| {
                    // It could be None if the app is in the process of exiting.
                    Error::DependencyNotFound {
                        dependency: Event::component_id(),
                        dependent: dependent.component.clone_id(),
                    }
                })?;
                let dependency = dependency.info().id.dependency();

                let part = DependencyPart {
                    id: LoadedDependencyId::new(dependent, dependency),
                    ty,
                };

                // With the dependency, we can manually create the listener.
                let Dependency { handle, .. } =
                    owned.increment_dependency(part, &handle.method_sender);
                let receiver = owned.event_sender.subscribe();

                Ok(Listener {
                    receiver: Dependency {
                        value: receiver,
                        handle,
                    },
                })
            },
        )
    }

    #[expect(clippy::unused_self)]
    const fn release_event_listener(self) -> impl AppMutFnOnce<Output = ()> {
        Self::spawn_mut(
            (),
            |(), _| Method::ReleaseEventListener,
            |(), owned, handle| {
                if let Some(dependency) = owned.event_sender_dependency.take() {
                    Dependency::drop_in_owned(dependency, owned, &handle.method_sender);
                }
            },
        )
    }
}

struct AppRunner {
    app: App,
    method_receiver: flume::Receiver<Method>,
}

impl AppRunner {
    async fn run(mut self) {
        while let Ok(method) = self.method_receiver.recv_async().await
            && !method.is_exit()
        {
            method.run(&mut self.app).await;
        }
    }
}

/// A message from a handle to run a method in the app.
#[derive(Debug)]
enum Method {
    Registries(registrar::Method),
    Errors(error::Method),
    Map(map::Method),

    Exit,
    ReleaseEventListener,

    GetEventListener {
        dependent: LoadedComponentId,
        ty: DependencyType,
        sender: OneshotSender<Result<Listener<Event>, Error>>,
    },
    Then {
        before: Box<Self>,
        after: Box<Self>,
    },

    CheckForStragglers,
    Flush(ComponentId),
}

impl Method {
    #[expect(clippy::manual_async_fn)] // doesn't work with boxed() for some reason
    fn run(self, app: &mut App) -> impl Future<Output = ()> + Send {
        async {
            match self {
                Self::Registries(method) => method.run(app).await,
                Self::Errors(method) => method.run(app).await,
                Self::Map(method) => method.run(app).await,
                // caller is expected to check for it
                Self::Exit => (),
                Self::ReleaseEventListener => app.release_event_listener().await,
                Self::Then {
                    before: first,
                    after,
                } => {
                    first.run(app).boxed().await;
                    after.run(app).boxed().await;
                }
                Self::GetEventListener {
                    dependent,
                    ty,
                    sender,
                } => {
                    let listener = app.get_event_listener_as(dependent, ty).await;
                    sender.send(listener);
                }
                Self::CheckForStragglers => {
                    let map = app.map().get().await;
                    warn!("app is taking a long time to exit...");
                    debug!("current state of map: {map:#?}");
                }
                Self::Flush(id) => {
                    AppFn::spawn_mut(
                        id,
                        |id, _| Self::Flush(id),
                        |id, owned, _| {
                            owned.registrar.remove_unchecked(&id);

                            let id = id.into();
                            let event_sender = owned.event_sender.clone();
                            let request = FlushRequest::new(id, event_sender);
                            let _ = owned.event_sender.send(Event::Flush(request));
                        },
                    )
                    .call(app)
                    .await;
                }
            }
        }
    }

    /// Returns `true` if the method is [`Exit`].
    ///
    /// [`Exit`]: Method::Exit
    #[must_use]
    const fn is_exit(&self) -> bool {
        matches!(self, Self::Exit)
    }
}

impl Default for Owned {
    fn default() -> Self {
        Self {
            registrar: Registrar::default(),
            errors: Errors::default(),
            map: Map::default(),
            event_sender: broadcast::Sender::new(1024),
            event_sender_dependency: None,
        }
    }
}

impl Owned {
    fn initialize(
        &mut self,
        children: Children,
        sender: &flume::Sender<Method>,
    ) -> Result<LoadedComponentId, Error> {
        // The three common ancestors (registries, app registry, app).
        let (registries, _) = ErasedComponent::new::<Registries, Registries>(());
        let registries_id = self.add_child_validated(registries, None, sender)?;

        let (app_registry, _) = ErasedComponent::new::<Registries, App>(());
        let app_registry_id =
            self.add_child_validated(app_registry, Some(registries_id), sender)?;

        let (app_component, _) = ErasedComponent::new::<App, App>(());
        let app_id = self.add_child_validated(app_component, Some(app_registry_id), sender)?;

        // The main registries.
        let registries = Children::new()
            .with::<Resources>(())
            .with::<ListenMessages>(())
            .with::<TakeMessages>(())
            .with::<Bundles>(());
        self.add_children(registries, sender);
        assert!(
            self.errors.is_empty(),
            "adding main registries shouldn't fail"
        );

        let app_loaded_id = self.map.load_component(
            app_id.clone(),
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );

        // The event sender dependency.
        let event_channel = ErasedComponent::from_loader::<ListenMessages>(ErasedLoader::new(
            listen::PersistentChannelLoader::new(self.event_sender.clone()),
        ));
        let event_id = self.add_child_validated(event_channel, Some(app_id.clone()), sender)?;
        let event_loaded_id = self.map.load_component(
            event_id,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );
        let event_dependency = self.increment_dependency(
            DependencyPart {
                id: LoadedDependencyId::new(app_loaded_id.clone(), event_loaded_id),
                ty: DependencyType::Unremovable,
            },
            sender,
        );
        self.event_sender_dependency = Some(event_dependency);

        // Then add the rest of the children.
        let children = children.adopted_by(app_id.deref().clone());
        self.add_children(children, sender);

        Ok(app_loaded_id)
    }
}

impl Registry for App {
    type Loader = AppLoader;
    type Loaded = AppLoaded;
}

impl Register<Self> for App {
    type ComponentInput = ();
    fn register((): Self::ComponentInput) -> (<Self as Registry>::Loader, Children) {
        (AppLoader(()), Children::new())
    }
}

pub struct AppLoader(());
impl Loader for AppLoader {
    type Loaded = AppLoaded;
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, Error> {
        Ok(AppLoaded(()))
    }

    fn on_load(&self) -> impl LoaderCallback<Self, Output = ()> {
        // app has custom load / unload logic in the map
        LoaderCallbacks::none()
    }
}

impl IdentifiedComponent for AppLoader {
    fn component_id_dyn(&self) -> ComponentId {
        App::component_id()
    }
}

#[derive(Any)]
pub struct AppLoaded(());
impl Loaded for AppLoaded {}

impl RegistryOf for App {
    type Registry = Self;
}

impl HandleData {
    const fn into_handle(self) -> App {
        App {
            variant: Inner::Handle,
            handle: self,
        }
    }

    fn clone_in_owned(&self, owned: &mut Owned) -> Self {
        Self {
            method_sender: self.method_sender.clone(),
            dependency: Dependency::clone_in_owned(&self.dependency, owned, &self.method_sender),
            new_dependency_type: self.new_dependency_type,
        }
    }
}
