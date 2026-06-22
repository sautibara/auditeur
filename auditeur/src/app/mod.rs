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

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use futures::FutureExt;
use tokio::sync::{broadcast, oneshot};

use crate::app::error::{AppFnErrors, AppFnErrorsMut};
use crate::app::map::AppFnMap;
use crate::app::registry::AppFnRegistriesMut;
use crate::component::IdentifiedComponentType;
use crate::prelude::*;
use crate::{
    app::{
        error::{AppErrors, AppErrorsMut, Errors},
        event::Event,
        map::{
            AddedComponentId, AppMap, AppMapMut, Dependency, DependencyPart, DependencyType, Map,
        },
        registry::{
            AppRegistriesMut, ErasedComponent, MarkerLoader, Registrar, Registries, Registry,
        },
    },
    children::HasChildren,
    component::{
        Component, RegistryOf,
        listen::{self, AppListenMut, ListenMessages, Listener},
        resource::{AppResourcesMut, Resources},
        take::{AppTakeMut, TakeMessages},
    },
};

const EXPECT_RECV: &str = "handle shouldn't try to access app after it closes";

pub mod error;
pub mod event;
pub mod map;
pub mod registry;

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

impl App {
    /// Start building a new app.
    #[must_use]
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Get a new handle to the app.
    #[must_use]
    pub async fn handle(&mut self) -> Self {
        let dependency_info = self.handle.dependency.info();
        let dependency_id = dependency_info.dependency.clone_inner();
        let dependent_id = dependency_info.dependent.clone_inner();
        let ty = dependency_info.ty;

        let dependency = self
            .map_mut()
            .increment_dependency(DependencyPart {
                dependent: dependent_id,
                dependency: dependency_id,
                ty,
            })
            .await;

        Self {
            variant: Inner::Handle,
            handle: HandleData {
                method_sender: self.handle.method_sender.clone(),
                dependency,
                new_dependency_type: self.handle.new_dependency_type,
            },
        }
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
    pub const fn dependent_mut(&mut self) -> DependentMut<'_> {
        DependentMut::new(self)
    }
}

/// Methods that access the main internals of the app: the [registries](registry), the [map] of the
/// added components, and the [error list](error).
impl App {
    #[must_use]
    pub const fn registries_mut(&mut self) -> AppRegistriesMut<'_> {
        AppRegistriesMut::new(self)
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
    pub const fn map(&mut self) -> AppMap<'_> {
        AppMap::new(self)
    }

    #[must_use]
    const fn map_mut(&mut self) -> AppMapMut<'_> {
        AppMapMut::new(self)
    }

    pub fn get_event_listener(
        &mut self,
    ) -> impl AppMutFuture<'_, Output = Result<Listener<Event>, Error>> {
        AppFn.get_event_listener().call(self)
    }

    fn get_event_listener_as(
        &mut self,
        dependent: AddedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFuture<'_, Output = Result<Listener<Event>, Error>> {
        AppFn.get_event_listener_as(dependent, ty).call(self)
    }
}

/// A message from a handle to run a method in the app.
enum Method {
    Registries(registry::Method),
    Errors(error::Method),
    Map(map::Method),
    GetEventListener {
        dependent: AddedComponentId,
        ty: DependencyType,
        sender: oneshot::Sender<Result<Listener<Event>, Error>>,
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
                    let _ = sender.send(listener);
                }
            }
        }
    }
}

struct AppRunner {
    app: App,
    method_receiver: flume::Receiver<Method>,
}

impl AppRunner {
    async fn run(mut self) {
        while let Ok(method) = self.method_receiver.recv_async().await {
            method.run(&mut self.app).await;
        }
    }
}

pub struct Builder {
    children: Children,
}

impl Builder {
    fn new() -> Self {
        Self {
            children: Children::new(),
        }
    }

    fn run(self) -> App {
        const NO_NAME_COLLISION: &str =
            "there shouldn't be a name collision when initializing the app";

        // Initialize the owned app.
        let event_sender = broadcast::Sender::new(1024);
        let mut owned = Owned {
            registrar: Registrar::default(),
            errors: Errors::default(),
            map: Map::default(),
            event_sender,
            event_sender_dependency: None,
        };

        let (method_sender, method_receiver) = flume::unbounded();

        // Construct the app component first as the parent of all.
        let (app_component, _) = ErasedComponent::new::<App, App>(());
        let app_component_add =
            map::AddChild::begin(&mut owned, app_component).expect(NO_NAME_COLLISION);

        // Construct the handle dependency first to keep the app alive.
        let handle_dependency = DependencyPart {
            dependent: app_component_add.id().clone_inner(),
            dependency: app_component_add.id().clone_inner(),
            // Transient b/c main thread is expected to drop the handle.
            ty: DependencyType::Transient,
        };
        let handle_dependency = owned.increment_dependency(handle_dependency, &method_sender);

        // Then the owned dependency now that the app is being kept alive.
        let owned_dependency = DependencyPart {
            dependent: app_component_add.id().clone_inner(),
            dependency: app_component_add.id().clone_inner(),
            // Weak so that the owned handle doesn't keep itself alive.
            ty: DependencyType::Weak,
        };
        let owned_dependency = owned.increment_dependency(owned_dependency, &method_sender);

        // And construct the rest of our running handle.
        let owned_handle = HandleData {
            method_sender: method_sender.clone(),
            dependency: owned_dependency,
            new_dependency_type: DependencyType::Unremovable,
        };

        // Now add a bunch of the registries.
        // WARN: The Registries registry *has* to be added first to be put in itself.
        let app_as_parent = || Some(app_component_add.id().clone_inner());
        owned
            .add_registry::<Registries>(app_as_parent(), &owned_handle)
            .expect(NO_NAME_COLLISION);
        let app_registry = owned
            .add_registry::<App>(app_as_parent(), &owned_handle)
            .expect(NO_NAME_COLLISION);
        let listen_registry = owned
            .add_registry::<ListenMessages>(app_as_parent(), &owned_handle)
            .expect(NO_NAME_COLLISION);

        // Now we could finish adding the app component, now that its registry is added.
        let app_id =
            app_component_add.finish_validated(&mut owned, None, app_registry, &owned_handle);

        // Add the app event channel, so that we could get a dependency to it.
        let event_component =
            ErasedComponent::from_loader::<ListenMessages>(registry::ErasedLoader::new(
                listen::PersistentChannelLoader::new(owned.event_sender.clone()),
            ));
        let event_component_id = map::AddChild::begin(&mut owned, event_component)
            .expect(NO_NAME_COLLISION)
            .finish_validated(
                &mut owned,
                Some(app_id.clone_inner()),
                listen_registry,
                &owned_handle,
            );
        let event_dependency = owned.increment_dependency(
            DependencyPart {
                dependent: app_id.clone_inner(),
                dependency: event_component_id,
                ty: DependencyType::Unremovable,
            },
            &method_sender,
        );
        owned.event_sender_dependency = Some(event_dependency);

        // Then add the rest of the children.
        let children = self
            .children
            .with::<Resources>(())
            // ListenMessages was already added above
            .with::<TakeMessages>(())
            .adopted_by(app_id.deref().clone());

        owned.add_children(children, &owned_handle);

        // Get our handles.
        let handle = App {
            variant: Inner::Handle,
            handle: HandleData {
                method_sender: method_sender.clone(),
                dependency: handle_dependency,
                new_dependency_type: DependencyType::Unremovable,
            },
        };

        let owned = App {
            variant: Inner::Owned(Box::new(owned)),
            handle: owned_handle,
        };

        // And start the owned app running in the background.
        let runner = AppRunner {
            app: owned,
            method_receiver,
        };
        tokio::task::spawn(runner.run());

        handle
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
    const fn dependent_added_id(&self) -> &AddedComponentId {
        &self.dependency.info().dependent
    }

    const fn app_id(&self) -> &AddedComponentId {
        &self.dependency.info().dependency
    }
}

impl<'a> Dependent<'a> {
    const fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Returns the id of the component that [dependencies](Dependency) obtained from this app will
    /// have as the dependent.
    #[must_use]
    pub fn dependent(self) -> &'a ComponentId {
        self.dependent_added_id()
    }

    /// Returns the id of the app component.
    #[must_use]
    pub fn app_id(self) -> &'a ComponentId {
        self.app.handle.app_id()
    }

    const fn dependent_added_id(self) -> &'a AddedComponentId {
        self.app.handle.dependent_added_id()
    }

    /// Returns the [`DependencyType`] that [dependencies](Dependency) obtained from this app will
    /// use.
    #[must_use]
    pub const fn dependency_type(self) -> DependencyType {
        self.app.handle.new_dependency_type
    }
}

pub struct DependentMut<'a> {
    app: &'a mut App,

    dependency: Option<Dependency<()>>,
    ty: Option<DependencyType>,
}

impl<'a> DependentMut<'a> {
    const fn new(app: &'a mut App) -> Self {
        Self {
            app,
            dependency: None,
            ty: None,
        }
    }

    /// # Errors
    ///
    /// - [`Error::DependentNotFound`] if the component with id `dependent` does not exist.
    pub async fn with_dependent(self, dependent: ComponentId) -> Result<Self, Error> {
        let dependency = self.app.handle.app_id().clone_inner();
        let dependency = (self.app.map_mut())
            // All handles have an unremovable dependency on the app.
            .try_increment_dependency(dependency, dependent, DependencyType::Unremovable)
            .await?;
        Ok(self.with_dependency(dependency))
    }

    async fn with_added_dependent(self, dependent: AddedComponentId) -> Self {
        let dependency = self.app.handle.app_id().clone_inner();
        let part = DependencyPart {
            dependency,
            dependent,
            // All handles have an unremovable dependency on the app.
            ty: DependencyType::Unremovable,
        };
        let dependency = self.app.map_mut().increment_dependency(part).await;
        self.with_dependency(dependency)
    }

    fn with_dependency(mut self, dependency: Dependency<()>) -> Self {
        self.dependency = Some(std::mem::replace(
            &mut self.app.handle.dependency,
            dependency,
        ));
        self
    }

    #[must_use]
    pub const fn with_ty(mut self, ty: DependencyType) -> Self {
        self.ty = Some(std::mem::replace(
            &mut self.app.handle.new_dependency_type,
            ty,
        ));
        self
    }
}

impl Drop for DependentMut<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.dependency.take() {
            self.app.handle.dependency = id;
        }

        if let Some(ty) = self.ty.take() {
            self.app.handle.new_dependency_type = ty;
        }
    }
}

impl Deref for DependentMut<'_> {
    type Target = App;
    fn deref(&self) -> &Self::Target {
        self.app
    }
}

impl DerefMut for DependentMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.app
    }
}

impl Registry for App {
    type Loader = MarkerLoader<Self>;
    type Loaded = ();
}

impl Component<Self> for App {
    type ComponentInput = ();
    fn register((): Self::ComponentInput) -> (<Self as Registry>::Loader, Children) {
        (MarkerLoader::new(Self::id()), Children::new())
    }
}

impl RegistryOf for App {
    type Registry = Self;
}

pub struct AppFn;
impl AppFn {
    #[expect(clippy::unused_self)]
    const fn registries_mut(self) -> AppFnRegistriesMut {
        AppFnRegistriesMut
    }

    #[expect(clippy::unused_self)]
    const fn map(self) -> AppFnMap {
        AppFnMap
    }

    #[expect(clippy::unused_self)]
    const fn errors(self) -> AppFnErrors {
        AppFnErrors
    }

    #[expect(clippy::unused_self)]
    const fn errors_mut(self) -> AppFnErrorsMut {
        AppFnErrorsMut
    }

    /// See [`App::get_event_listener`].
    pub const fn get_event_listener(
        self,
    ) -> impl AppMutFnOnce<Output = Result<Listener<Event>, Error>> {
        Self::using_handle_mut(|handle| {
            Self.get_event_listener_as(
                handle.dependent_added_id().clone_inner(),
                handle.new_dependency_type,
            )
        })
    }

    #[expect(clippy::unused_self)]
    const fn get_event_listener_as(
        self,
        dependent: AddedComponentId,
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
                        dependent: dependent.deref().clone(),
                    }
                })?;
                let dependency = dependency.info().dependency.clone_inner();

                let part = DependencyPart {
                    dependent,
                    dependency,
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
            method: impl FnOnce(I, oneshot::Sender<O>, &HandleData)
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
                M: FnOnce(I, oneshot::Sender<Out>, &HandleData) -> Method + Send + Sync + 'static,
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

app_future_def! {
    trait @AppFnOnceInner<'a>: Into<@AppFnOnceImpl<'a, @generics_from_types>> + Send + Sync {
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
