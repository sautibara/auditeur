#![allow(private_interfaces, private_bounds)]

macro_rules! app_future_def {
    // Mutable components.
    ( @ true (@($when_false:tt, $when_true:tt) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* $when_true ) $($rest)* );
    };
    ( @ false (@($when_false:tt, $when_true:tt) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* $when_false ) $($rest)* );
    };
    ( @ true (@mut $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* mut ) $($rest)* );
    };
    ( @ false (@mut $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* ) $($rest)* );
    };
    ( @ true (@AppFnOnce $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnce ) $($rest)* );
    };
    ( @ false (@AppFnOnce $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnce ) $($rest)* );
    };
    ( @ true (@AppFuture $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFuture ) $($rest)* );
    };
    ( @ false (@AppFuture $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFuture ) $($rest)* );
    };
    ( @ true (@AppFnOnceInner $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnceInner ) $($rest)* );
    };
    ( @ false (@AppFnOnceInner $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnceInner ) $($rest)* );
    };
    ( @ true (@AppFnOnceImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnceImpl ) $($rest)* );
    };
    ( @ false (@AppFnOnceImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnceImpl ) $($rest)* );
    };
    ( @ true (@AppFutureImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFutureImpl ) $($rest)* );
    };
    ( @ false (@AppFutureImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFutureImpl ) $($rest)* );
    };
    // Big components.
    ( @ $mut:tt (@fields $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            source: Source,
            handle_fn: HandleFn,
            owned_fn: OwnedFn
        ) $($rest)* );
    };
    ( @ $mut:tt (@generics_bounded $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut (
            Source: @AppFnOnce<Output = Output>,
            Output: Send + Sync + 'static,
            HandleFn: FnOnce(Source, &'a HandleData) -> (Method, HandleFut)
                + Send + Sync + 'static,
            HandleFut: Future<Output = Output> + Send + Sync + 'a,
            OwnedFn: FnOnce(Source, &'a @mut Owned, &'a HandleData) -> OwnedFut
                + Send + Sync + 'static,
            OwnedFut: Future<Output = Output> + Send + Sync + 'a
        $($in)*) ( $($out)* ) $($rest)*);
    };
    ( @ $mut:tt (@generics_unbounded $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            Source,
            Output,
            HandleFn,
            HandleFut,
            OwnedFn,
            OwnedFut
        ) $($rest)* );
    };
    ( @ $mut:tt (@type_defs $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut (
            type Source: @AppFnOnce<Output = Self::Output>;
            type Output: Send + Sync + 'static;
            type HandleFn: FnOnce(Self::Source, &'a HandleData) -> (Method, Self::HandleFut)
                + Send + Sync + 'static;
            type HandleFut: Future<Output = Self::Output> + Send + Sync + 'a;
            type OwnedFn: FnOnce(Self::Source, &'a @mut Owned, &'a HandleData)
                -> Self::OwnedFut
                + Send + Sync + 'static;
            type OwnedFut: Future<Output = Self::Output> + Send + Sync + 'a
        $($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt (@type_impls $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            type Source = Source;
            type Output = Output;
            type HandleFn = HandleFn;
            type HandleFut = HandleFut;
            type OwnedFn = OwnedFn;
            type OwnedFut = OwnedFut
        ) $($rest)*);
    };
    ( @ $mut:tt (@generics_from_types $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            Self::Source,
            Self::Output,
            Self::HandleFn,
            Self::HandleFut,
            Self::OwnedFn,
            Self::OwnedFut
        ) $($rest)*);
    };
    // Pushing a delimiter onto the delimiter stack.
    ( @ $mut:tt (($($next:tt)*) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () paren ($($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt ({$($next:tt)*} $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () curly ($($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt ([$($next:tt)*] $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () square ($($in)*) ($($out)*) $($rest)*);
    };
    // Pushing a default token.
    ( @ $mut:tt ($next:tt $($in:tt)*) ($($out:tt)*) $($rest:tt)* ) => {
        app_future_def!( @ $mut ($($in)*) ($($out)* $next) $($rest)* );
    };
    // Popping a delimiter off the delimiter stack.
    ( @ $mut:tt () ($($out:tt)*) paren ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* ($($out)*)) $($rest)* );
    };
    ( @ $mut:tt () ($($out:tt)*) curly ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* {$($out)*}) $($rest)* );
    };
    ( @ $mut:tt () ($($out:tt)*) square ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* [$($out)*]) $($rest)* );
    };
    // Base case.
    ( @ $mut:tt () ($($out:tt)*) ) => {
        $($out)*
    };
    // Initial entrance.
    ( mut: $mut:tt; $($in:tt)* ) => {
        app_future_def!( @ $mut ($($in)*) () );
    };
    ( $($in:tt)* ) => {
        app_future_def!( @ false ($($in)*) () );
        app_future_def!( @ true ($($in)*) () );
    };
}

use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use futures::FutureExt;
use tokio::sync::{broadcast, oneshot};

use crate::app::component::{AppComponentsMut, AppFnComponentsMut};
use crate::app::error::{AppFnErrors, AppFnErrorsMut};
use crate::app::map::{
    AppFnMap, Channels, DependencyHandle, LoadedComponentId, LoadedDependencyId, RequestSet,
};
use crate::component::bundle::Bundles;
use crate::component::registry::LoaderCallback;
use crate::component::resource::AppFnResourcesMut;
use crate::component::{IdentifiedComponent, IdentifiedComponentType};
use crate::prelude::*;
use crate::{
    app::{
        error::{AppErrors, AppErrorsMut, Errors},
        event::Event,
        map::{AddedComponentId, AppMap, Dependency, DependencyPart, DependencyType, Map, Waiter},
        registrar::Registrar,
    },
    children::HasChildren,
    component::{
        Component, RegistryOf,
        listen::{self, AppListenMut, ListenMessages, Listener},
        registry::{ErasedLoader, Loaded, Loader, LoaderCallbacks, Registries, Registry},
        resource::{AppResourcesMut, Resources},
        take::{AppTakeMut, TakeMessages},
    },
};

const EXPECT_RECV: &str = "handle shouldn't try to access app after it closes";

pub mod component;
pub mod error;
pub mod event;
pub mod map;
pub(crate) mod registrar;

pub use registrar::ErasedComponent;

/// A handle to the shared backend state of the app; the way that components interact.
///
/// This contains:
/// - A shared errors list, accessible through [`Self::errors`] and [`Self::errors_mut`],
/// - [Registries](registry::Registry), containing all added
///   [components](crate::component::Component), accessible through [`Self::registries_mut`].
/// - A map of the relationships and dependencies between components,
///   accessible through [`Self::map`].
#[identified("auditeur:app")]
pub struct App {
    variant: Inner,
    handle: HandleData,
}

struct HandleData {
    method_sender: flume::Sender<Method>,
    dependency: Dependency<()>,
    new_dependency_type: DependencyType,
}

enum Inner {
    Owned(Box<Owned>),
    Handle,
}

/// The actual shared state of the app.
struct Owned {
    registrar: Registrar,
    errors: Errors,
    map: Map,

    event_sender: broadcast::Sender<Event>,
    event_sender_dependency: Option<Dependency<()>>,
}

struct OwnedHandle<'a> {
    owned: &'a mut Owned,
    handle: &'a HandleData,
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

/// Methods relating to builtin components: [`Resource`]s, [`ListenMessage`]s, and [`TakeMessage`]s.
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

/// Methods that access the main internals of the app: the [registries](registry), the [map] of the
/// added components, and the [error list](error).
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

struct OneshotSender<T>(oneshot::Sender<T>);

impl<T> OneshotSender<T> {
    pub fn send(self, value: T) {
        let _ = self.0.send(value);
    }
}

impl<T> Debug for OneshotSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "oneshot::Sender(...)")
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

#[must_use = "builder does nothing unless it is run"]
pub struct Builder {
    children: Children,
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

// Returns a 2-tuple of the owned app and handle app.

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

impl Registry for App {
    type Loader = AppLoader;
    type Loaded = AppLoaded;
}

impl Component<Self> for App {
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
            async |(dependent, ty), owned, handle| {
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
            async |(), owned, handle| {
                if let Some(dependency) = owned.event_sender_dependency.take() {
                    Dependency::drop_in_owned(dependency, owned, &handle.method_sender);
                }
            },
        )
    }
}

impl AppFn {
    app_future_def! {
        const fn @(spawn, spawn_mut)<I: Send + Sync + 'static>(
            input: I,
            method: impl FnOnce(I, &HandleData) -> Method + Send + Sync + 'static,
            owned: impl for<'a> AsyncFnOnce<
                (I, &'a @mut Owned, &'a HandleData),
                Output = (),
                CallOnceFuture: Send + Sync,
            > + Send
            + Sync
            + 'static,
        ) -> impl @AppFnOnce<Output = ()> {
            struct Spawn<I, M, O> {
                input: I,
                method: M,
                owned: O,
            }

            impl<
                I: Send + Sync + 'static,
                M: FnOnce(I, &HandleData) -> Method + Send + Sync + 'static,
                O: for<'a> AsyncFnOnce<
                        (I, &'a @mut Owned, &'a HandleData),
                        CallOnceFuture: Send + Sync,
                        Output = (),
                    > + Send
                    + Sync
                    + 'static,
            > @AppFnOnce for Spawn<I, M, O>
            {
                type Output = ();
                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| ((this.method)(this.input, handle), async {}),
                        owned_fn: |this, owned, handle| (this.owned)(this.input, owned, handle),
                    }
                }
            }

            Spawn {
                input,
                method,
                owned,
            }
        }
    }

    app_future_def! {
        const fn @(oneshot, oneshot_mut)<I: Send + Sync + 'static, O: Send + Sync + 'static>(
            input: I,
            method: impl FnOnce(I, OneshotSender<O>, &HandleData)
                -> Method + Send + Sync + 'static,
            owned: impl for<'a> AsyncFnOnce<
                (I, &'a @mut Owned, &'a HandleData),
                Output = O,
                CallOnceFuture: Send + Sync,
            > + Send
            + Sync
            + 'static,
        ) -> impl @AppFnOnce<Output = O> {
            struct Oneshot<I, M, O> {
                input: I,
                method: M,
                owned: O,
            }

            impl<
                I: Send + Sync + 'static,
                Out: Send + Sync + 'static,
                M: FnOnce(I, OneshotSender<Out>, &HandleData) -> Method + Send + Sync + 'static,
                O: for<'a> AsyncFnOnce<
                        (I, &'a @mut Owned, &'a HandleData),
                        CallOnceFuture: Send + Sync,
                        Output = Out,
                    > + Send
                    + Sync
                    + 'static,
            > @AppFnOnce for Oneshot<I, M, O>
            {
                type Output = Out;
                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| {
                            let (sender, receiver) = oneshot::channel();
                            let sender = OneshotSender(sender);
                            ((this.method)(this.input, sender, handle), async {
                                receiver.await.expect("app shouldn't drop our message")
                            })
                        },
                        owned_fn: |this, owned, handle| (this.owned)(this.input, owned, handle),
                    }
                }
            }

            Oneshot {
                input,
                method,
                owned,
            }
        }
    }

    app_future_def! {
        const fn @(using_handle, using_handle_mut)<
            O: Send + Sync + 'static,
            F: @AppFnOnce<Output = O>
        >(
            mapping: impl FnOnce(&HandleData) -> F + Send + Sync + 'static,
        ) -> impl @AppFnOnce<Output = O> {
            struct UsingHandle<M> {
                mapping: M,
            }

            impl<
                M: FnOnce(&HandleData) -> F + Send + Sync + 'static,
                F: @AppFnOnce<Output = O>,
                O: Send + Sync + 'static,
            > @AppFnOnce for UsingHandle<M>
            {
                type Output = O;

                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| {
                            let fn_once: F = (this.mapping)(handle);
                            let inner: @AppFnOnceImpl<_, _, _, _, _, _>
                                = fn_once.into_inner().into();
                            (inner.handle_fn)(inner.source, handle)
                        },
                        owned_fn: |this, owned, handle| {
                            let fn_once: F = (this.mapping)(handle);
                            let inner: @AppFnOnceImpl<_, _, _, _, _, _>
                                = fn_once.into_inner().into();
                            (inner.owned_fn)(inner.source, owned, handle)
                        },
                    }
                }
            }

            UsingHandle { mapping }
        }
    }
}

// TODO: ability to turn an AppFnOnceInner back into an AppFnOnce
// this should be easy if we replace Input with what should be Self
// That is, Input will implement AppFnOnce, so converting would be as simple as dropping the
// functions

#[must_use]
pub trait AppFuture<'a>:
    IntoFuture<IntoFuture: Send + Sync, Output: Send + Sync> + Send + Sync + Sized
{
    fn into_parts(self) -> (&'a App, impl AppFnOnce<Output = Self::Output>);

    fn into_fn_once(self) -> impl AppFnOnce<Output = Self::Output> {
        let (_, fn_once) = self.into_parts();
        fn_once
    }
}

#[must_use]
pub trait AppMutFuture<'a>:
    IntoFuture<IntoFuture: Send + Sync, Output: Send + Sync> + Send + Sync + Sized
{
    fn into_parts(self) -> (&'a mut App, impl AppMutFnOnce<Output = Self::Output>);

    fn into_fn_once(self) -> impl AppMutFnOnce<Output = Self::Output> {
        let (_, fn_once) = self.into_parts();
        fn_once
    }

    fn with_event_listener(
        self,
    ) -> impl AppMutFuture<'a, Output = (Self::Output, Result<Listener<Event>, Error>)> {
        let (app, fn_once) = self.into_parts();
        let fn_once = fn_once.then(AppFn.get_event_listener());
        fn_once.call(app)
    }
}

app_future_def! {
    struct @AppFutureImpl<'a, F: @AppFnOnceInner<'a>> {
        app: &'a @mut App,
        func: F,
    }

    impl<'a, F: @AppFnOnceInner<'a>> IntoFuture for @AppFutureImpl<'a, F> {
        type Output = F::Output;
        type IntoFuture = futures::future::Either<F::HandleFut, F::OwnedFut>;

        fn into_future(self) -> Self::IntoFuture {
            let func: @AppFnOnceImpl<_, _, _, _, _, _> = self.func.into();
            match & @mut self.app.variant {
                Inner::Handle => {
                    let (method, future) = (func.handle_fn)(func.source, &self.app.handle);
                    let _ = self.app.handle.method_sender.send(method);
                    futures::future::Either::Left(future)
                }
                Inner::Owned(owned) => {
                    futures::future::Either::Right((func.owned_fn)(func.source, owned, &self.app.handle))
                }
            }
        }
    }

    impl<'a, F: @AppFnOnceInner<'a>> @AppFuture<'a> for @AppFutureImpl<'a, F> {
        fn into_parts(self) -> (&'a @mut App, impl @AppFnOnce<Output = Self::Output>) {
            (self.app, self.func.into().source)
        }
    }
}

#[must_use]
pub trait AppFnOnce: Send + Sync + Sized + 'static {
    type Output: Send + Sync + 'static;

    #[doc(hidden)]
    fn into_inner<'a>(self) -> impl AppFnOnceInner<'a, Output = Self::Output>;

    fn call(self, app: &App) -> impl AppFuture<'_, Output = Self::Output> {
        AppFutureImpl {
            app,
            func: self.into_inner(),
        }
    }

    fn using_mut(self) -> impl AppMutFnOnce<Output = Self::Output> {
        struct UsingMut<A: AppFnOnce>(A);

        impl<A: AppFnOnce> AppMutFnOnce for UsingMut<A> {
            type Output = A::Output;
            fn into_inner<'a>(self) -> impl AppMutFnOnceInner<'a, Output = Self::Output> {
                AppMutFnOnceImpl {
                    pd: PhantomData,
                    source: self,
                    handle_fn: |this, handle| {
                        let this: AppFnOnceImpl<_, _, _, _, _, _> = this.0.into_inner().into();
                        (this.handle_fn)(this.source, handle)
                    },
                    owned_fn: move |this, owned, handle| {
                        let this: AppFnOnceImpl<_, _, _, _, _, _> = this.0.into_inner().into();
                        (this.owned_fn)(this.source, owned, handle)
                    },
                }
            }
        }

        UsingMut::<Self>(self)
    }

    fn then<U: Send + Sync + 'static>(
        self,
        other: impl AppFnOnce<Output = U>,
    ) -> impl AppFnOnce<Output = (Self::Output, U)> {
        Then {
            before: self,
            after: other,
        }
    }

    fn map<U: Send + Sync + 'static>(
        self,
        mapping: impl FnOnce(Self::Output) -> U + Send + Sync + 'static,
    ) -> impl AppFnOnce<Output = U> {
        AppFnMapping {
            inner: self,
            mapping,
        }
    }
}

#[must_use]
pub trait AppMutFnOnce: Send + Sync + Sized + 'static {
    type Output: Send + Sync + 'static;

    #[doc(hidden)]
    fn into_inner<'a>(self) -> impl AppMutFnOnceInner<'a, Output = Self::Output>;

    fn call(self, app: &mut App) -> impl AppMutFuture<'_, Output = Self::Output> {
        AppMutFutureImpl {
            app,
            func: self.into_inner(),
        }
    }

    fn then<U: Send + Sync + 'static>(
        self,
        other: impl AppMutFnOnce<Output = U>,
    ) -> impl AppMutFnOnce<Output = (Self::Output, U)> {
        Then {
            before: self,
            after: other,
        }
    }

    fn map<U: Send + Sync + 'static>(
        self,
        mapping: impl FnOnce(Self::Output) -> U + Send + Sync + 'static,
    ) -> impl AppMutFnOnce<Output = U> {
        AppFnMapping {
            inner: self,
            mapping,
        }
    }
}

struct Then<B, A> {
    before: B,
    after: A,
}

app_future_def! {
    impl<
        BO: Send + Sync + 'static,
        AO: Send + Sync + 'static,
        B: @AppFnOnce<Output = BO>,
        A: @AppFnOnce<Output = AO>,
    > @AppFnOnce for Then<B, A>
    {
        type Output = (BO, AO);
        fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
            @AppFnOnceImpl {
                pd: PhantomData,
                source: self,
                handle_fn: move |this, handle| {
                    let before_inner: @AppFnOnceImpl<_, _, _, _, _, _> =
                        this.before.into_inner().into();
                    let after_inner: @AppFnOnceImpl<_, _, _, _, _, _> =
                        this.after.into_inner().into();

                    let (before_method, before_fut) =
                        (before_inner.handle_fn)(before_inner.source, handle);
                    let (after_method, after_fut) =
                        (after_inner.handle_fn)(after_inner.source, handle);
                    (
                        Method::Then {
                            before: Box::new(before_method),
                            after: Box::new(after_method),
                        },
                        async move { (before_fut.await, after_fut.await) },
                    )
                },
                owned_fn: |this, owned, handle| async move {
                    let @AppFnOnceImpl {
                        pd,
                        source,
                        handle_fn,
                        owned_fn,
                    } = this.before.into_inner().into();
                    let before = (owned_fn)(source, owned, handle).await;
                    drop((handle_fn, pd));

                    let after_inner: @AppFnOnceImpl<_, _, _, _, _, _> =
                        this.after.into_inner().into();
                    let after = (after_inner.owned_fn)(after_inner.source, owned, handle).await;
                    (before, after)
                },
            }
        }
    }
}

struct AppFnMapping<A, M> {
    inner: A,
    mapping: M,
}

app_future_def! {
    impl<
        A: @AppFnOnce<Output = O>,
        O: Send + Sync + 'static,
        U: Send + Sync + 'static,
        M: FnOnce(O) -> U + Send + Sync + 'static,
    > @AppFnOnce for AppFnMapping<A, M>
    {
        type Output = U;
        fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
            @AppFnOnceImpl {
                pd: PhantomData,
                source: self,
                handle_fn: move |this, handle| {
                    let inner: @AppFnOnceImpl<_, _, _, _, _, _> = this.inner.into_inner().into();
                    let (method, fut) = (inner.handle_fn)(inner.source, handle);
                    // WARN: doing this map manually (through an await or something)
                    // causes to an ICE for some reason...
                    let fut = futures::future::FutureExt::map(fut, this.mapping);
                    (method, fut)
                },
                owned_fn: |this, owned, handle| {
                    let inner: @AppFnOnceImpl<_, _, _, _, _, _> = this.inner.into_inner().into();
                    let fut = (inner.owned_fn)(inner.source, owned, handle);
                    futures::future::FutureExt::map(fut, this.mapping)
                },
            }
        }
    }
}

app_future_def! {
    #[doc(hidden)]
    pub trait @AppFnOnceInner<'a>: Into<@AppFnOnceImpl<'a, @generics_from_types>> + Send + Sync {
        @type_defs;
    }

    struct @AppFnOnceImpl<'a, @generics_bounded> {
        pd: PhantomData<fn(&'a ())>,
        @fields,
    }

    impl<'a, @generics_bounded> @AppFnOnceInner<'a> for @AppFnOnceImpl<'a, @generics_unbounded> {
        @type_impls;
    }
}
