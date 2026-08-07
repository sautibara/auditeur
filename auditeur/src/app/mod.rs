#![allow(private_interfaces, private_bounds)]
//! The backend of the library; the way that components interact.
//!
//! All interactions between [`Component`](crate::component::Component)s start with an [`App`]
//! handle. With one, one could:
//! - add, load, and remove components using [`App::components_mut`],
//! - access the current state of the app using  [`App::map`],
//! - wholly exit the app using [`App::exit`],
//! - access the global error list using [`App::errors`] and [`App::errors_mut`],
//!
//! Many [registries](crate::component) also provide extensions onto [`App`] to manage components
//! within their registry, such as:
//! - [`App::resources_mut`],
//! - [`App::listen_mut`],
//! - [`App::take_mut`],
//!
//! Every component is given access to an app handle when loading, allowing them to access app
//! state if necessary. This is how components are allowed to recursively depend on each other, as
//! one component loading could load many more components down the line. Handles can also be
//! duplicated using [`App::handle`], so components can also access app state while they're already
//! running. In all, using app handles is made to be as flexible as possible.
//!
//! ### Examples
//!
//! ```
//! use auditeur::component::bundle::Bundles;
//! use auditeur::component::registry::Registries;
//!
//! use auditeur::prelude::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), auditeur::Error> {
//! let app = App::builder().spawn();
//!
//! // We could query the current state of the app using its map.
//! // [`AppMap::get`] gets a clone of the state, which we could then access ourselves.
//! // It's an async method, since it sends a message to the app and waits for its response.
//! let map = app.map().get().await;
//!
//! // The [`Registries`] registry should be loaded, since it's used to hold itself.
//! assert!(map.is_loaded(&Registries::component_id()));
//!
//! // The [`App`] component should also be loaded, since we have an app handle.
//! assert!(map.is_loaded(&App::component_id()));
//!
//! // The [`Bundles`] registry should be added but not loaded, since nothing is in it.
//! assert!(map.is_added(&Bundles::component_id()));
//! assert!(!map.is_loaded(&Bundles::component_id()));
//!
//! auditeur::test::finish(app, false).await
//! # }
//! ```
//!
//! ## Dependents
//!
//! Every app handle has a set dependent component, accessed through [`App::dependent`] and
//! [`App::dependent_mut`]. This is the component that will be used as the dependent when
//! dependencies are created from the handle. That is, the app will treat the component as the one
//! keeping the dependencies alive. This means that if the dependency is requested to be removed,
//! the dependent may also be requested to be removed (depending on the [`DependencyType`]).
//!
//! In a sense, the dependent component is always meant to be the "current" component - the
//! component that is currently accessing the state of the app. When a component is loaded, the
//! dependent is set to the component being loaded. When a handle is obtained, the dependent is
//! copied over. When components are added using a handle, their parent will be the current
//! dependent.
//!
//! ### Examples
//!
//! ```
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # #[allow(clippy::items_after_statements)]
//! # async fn main() -> Result<(), auditeur::Error> {
//! let mut app = App::builder().spawn();
//!
//! // The dependent of the app handle obtained from [`AppBuilder::spawn`] is always the [`App`]
//! // component, so dependencies obtained here are responsibilities of the whole app.
//! assert_eq!(**app.dependent().dependent_id(), App::component_id());
//!
//! // Meanwhile, resources are created using the app handle of the loader,
//! #[resource("test:resource")]
//! #[listener(weak, options)]
//! #[create(default)]
//! struct Resource {
//!     // and app handles copy over the dependent from their source,
//!     #[create(input)]
//!     app: App,
//! }
//!
//! impl ListenerDerive for Resource {
//!     async fn init(&mut self) -> anyhow::Result<()> {
//!         // so we know that the handle's dependent should be the current resource.
//!         assert_eq!(**self.app.dependent().dependent_id(), Self::component_id());
//!         Ok(())
//!     }
//! }
//!
//! // Now, after adding the component, we could see that
//! app.components_mut()
//!     .add(Children::new().with::<Resource>(()))
//!     .await;
//!
//! // [`AppComponentsMut::load`] uses the current dependent for its returned dependency,
//! let dependency = app.components_mut().load(Resource::component_id()).await?;
//! assert_eq!(
//!     **Dependency::info_of(&dependency).id.dependent(),
//!     App::component_id()
//! );
//!
//! // and [`AppComponentsMut::add`] uses the current dependent as the parent of added components.
//! let map = app.map().get().await;
//! assert_eq!(
//!     map.parent_of(&Resource::component_id()).map(|id| &**id),
//!     Some(&App::component_id())
//! );
//!
//! drop(dependency);
//! auditeur::test::finish(app, false).await
//! # }
//! ```
//!
//! ## App Functions / Futures
//!
//! All methods that can be completed by accessing the app once return [`AppFuture`]s. Effectively,
//! this means all methods except the ones that load components like [`AppComponentsMut::get`].
//! These futures have the advantage that they can be combined together to get all the data
//! necessary in one go, avoiding race conditions if the state changes in between accesses
//! to the app.
//!
//! This module provides the [`AppFnOnce`] and [`AppMutFnOnce`] traits that act as app methods
//! without the given app reference. Once given a reference, they turn into [`AppFuture`]s or
//! [`AppMutFuture`]s respectively. As `FnOnce`s, they can be combined using [`AppFnOnce::then`] or
//! [`AppMutFnOnce::then`]. See [`AppFn`] for the `FnOnce`s available.
//!
//! ### Examples
//!
//! ```
//! use auditeur::app::event::ComponentLifecycle;
//! use auditeur::component::bundle::Bundles;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # #[allow(clippy::items_after_statements)]
//! # async fn main() -> Result<(), auditeur::Error> {
//! // Since the app can be accessed from multiple threads, we have to be weary of race conditions.
//!
//! // Let's imagine we want to listen to when bundles are added to the app.
//!
//! let mut app = App::builder()
//!     // Here's a bundle added before we start listening.
//!     .with::<Bundle>(new_bundle!("test:bundle_1"))
//!     .spawn();
//!
//! // Now, to start listening, we get a map and event listener from the app.
//! // For this, we use [`AppFn`], which mirrors [`App`]'s api without accessing a handle.
//! let (map, event_listener) = (AppFn.map().get().using_mut())
//!     .then(AppFn.get_event_listener())
//!     // Only once we finish building up our request do we have to use our app handle,
//!     .call(&mut app)
//!     // allowing us to complete the request all at once.
//!     .await;
//! let mut event_listener = event_listener?;
//!
//! // Through the map, we could find the bundle added before we started listening.
//! assert!(map.is_added(&ComponentId::inn::<Bundles>(ident!("test:bundle_1"))));
//!
//! // And if we add a bundle after we start listening,
//! app.components_mut()
//!     .add(Children::new().with::<Bundle>(new_bundle!("test:bundle_2")))
//!     .await;
//!
//! // we can find it using the event listener.
//! loop {
//!     if let auditeur::app::Event::Component { id, lifecycle } = event_listener.listen().await
//!         && lifecycle == ComponentLifecycle::Add
//!         && id == ComponentId::inn::<Bundles>(ident!("test:bundle_2"))
//!     {
//!         break;
//!     }
//! }
//!
//! // If we didn't use the [`AppFn`] api imagine if we (or another thread):
//! // - got a map,
//! // - added a bundle now, and
//! // - got an event listener after.
//! //
//! // Since the bundle was added after the map was obtained, there's no way that it could've been
//! // included in the map. Similarly, since event listeners only return events from after they're
//! // obtained, there's no way it would show the event. Because of these reasons, the bundle added
//! // would've been missed completely.
//! //
//! // Instead, since we got the map and event listener in the same request, there's no way that
//! // a bundle could've been added in between the two actions, so there was no risk of a race
//! // condition.
//!
//! drop(event_listener);
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use futures::FutureExt;
use tokio::sync::broadcast;
use tracing::debug;

use crate::app::map::DependencyHandle;
use crate::{
    app::{
        error::Errors,
        event::FlushRequest,
        id::{AddedComponentId, DependencyPart, LoadedComponentId, LoadedDependencyId},
        map::{Channels, Map, RequestSet, Waiter},
        registrar::Registrar,
    },
    component::{
        HasChildren, IdentifiedComponent, IdentifiedComponentType, Register, RegistryOf,
        bundle::Bundles,
        listen::{self, AppListenMut, ListenMessages, Listener},
        registry::{
            ErasedLoader, Loaded, Loader, LoaderCallback, LoaderCallbacks, Registries, Registry,
        },
        resource::{AppFnResourcesMut, AppResourcesMut, Resources},
        take::{AppTakeMut, TakeMessages},
    },
    input::CloneIn,
    prelude::*,
};

pub mod error;
pub mod event;
pub mod id;
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
    pub(crate) handle: HandleData,
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
    /// Starts building a new app.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// Gets a new handle to the app.
    pub fn handle(&mut self) -> impl AppMutFuture<'_, Output = Self> {
        AppFn.handle().call(self)
    }

    /// Tells the app to exit.
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

/// Methods relating to this handle's current [dependent](self#dependents): the component that will
/// keep alive all dependencies returned by this app handle.
impl App {
    #[must_use]
    pub const fn dependent(&self) -> AppDependent<'_> {
        AppDependent::new(self)
    }

    #[must_use]
    pub const fn dependent_mut(&mut self) -> AppDependentMut<'_, (), ()> {
        AppDependentMut::new(self)
    }
}

/// Methods that access the main internals of the app: the [components](AppComponentsMut), the
/// [map] of added components, and the [error list](mod@error).
impl App {
    #[must_use]
    pub const fn components_mut(&mut self) -> AppComponentsMut<'_> {
        AppComponentsMut::new(self)
    }

    #[must_use]
    pub const fn map(&self) -> AppMap<'_> {
        AppMap::new(self)
    }

    #[must_use]
    pub const fn errors(&self) -> AppErrors<'_> {
        AppErrors::new(self)
    }

    #[must_use]
    pub const fn errors_mut(&mut self) -> AppErrorsMut<'_> {
        AppErrorsMut::new(self)
    }
}

impl App {
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

/// Return type of [`AppFn::components_mut`](AppFn::components_mut), see [`AppComponentsMut`].
#[must_use]
pub struct AppFnComponentsMut;

/// Modify data related to [components](crate::component) in an [`App`] handle.
pub struct AppComponentsMut<'a> {
    app: &'a mut App,
}

impl<'a> AppComponentsMut<'a> {
    const fn new(app: &'a mut App) -> Self {
        Self { app }
    }
}

/// Return type of [`AppFn::errors`], see [`AppErrors`].
///
/// See [`AppFnErrorsMut`] for mutable methods.
#[must_use]
pub struct AppFnErrors;

/// Access data related to [errors](mod@error) in an [`App`] handle.
///
/// See [`AppErrorsMut`] for mutable methods.
pub struct AppErrors<'a> {
    app: &'a App,
}

/// Return type of [`AppFn::errors_mut`], see [`AppErrorsMut`].
///
/// See [`AppFnErrors`] for immutable methods.
#[must_use]
pub struct AppFnErrorsMut;

/// Modify data related to [errors](mod@error) in an [`App`] handle.
///
/// See [`AppErrors`] for immutable methods.
pub struct AppErrorsMut<'a> {
    app: &'a mut App,
}

/// Return type of [`AppFn::map`], see [`AppMap`].
#[must_use]
pub struct AppFnMap;

/// Access the [map](mod@map) in an [`App`] handle.
pub struct AppMap<'a> {
    app: &'a App,
}

/// Access the [dependent](self#dependents) of an [`App`] handle.
///
/// See [`AppDependentMut`] for mutable methods.
pub struct AppDependent<'a> {
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

impl<'a> AppDependent<'a> {
    const fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Returns the id of the component that [dependencies](Dependency) obtained from this app will
    /// have as the dependent.
    #[must_use]
    pub fn dependent_id(self) -> LoadedComponentId {
        self.app.handle.dependent_id()
    }

    /// Returns the id of the app component, which this app handle is keeping alive.
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

/// Modify the [dependent](self#dependents) of an [`App`] handle.
///
/// This type implements [`Deref`] and [`DerefMut`] to the underlying [`App`], so it can be used as
/// one.
///
/// See [`AppDependent`] for immutable methods.
pub struct AppDependentMut<'a, D, T> {
    inner: DependentMutInner<'a>,
    pd: PhantomData<(D, T)>,
}

struct DependentMutInner<'a> {
    app: &'a mut App,
    dependency: Option<Dependency<()>>,
    ty: Option<DependencyType>,
}

impl<'a> AppDependentMut<'a, (), ()> {
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

impl<'a, T> AppDependentMut<'a, (), T> {
    /// Temporarily changes the dependent of this [`App`] handle to the component depended on in
    /// `dependency`.
    ///
    /// This cannot be called multiple times. If you want to change the dependent again, call
    /// [`App::dependent_mut`] another time.
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur::prelude::*;
    /// # use auditeur::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), auditeur::Error> {
    /// #[resource("test:resource")]
    /// #[pure_resource]
    /// #[create(default)]
    /// struct Resource;
    ///
    /// let mut app = App::builder().with::<Resource>(()).spawn();
    ///
    /// // If we get a dependency to the added [`Resource`],
    /// let dependency = app.components_mut().load(Resource::component_id()).await?;
    ///
    /// // we could use it as the dependent in our handle.
    /// let with_dependent = app.dependent_mut().with_dependent(&dependency).await;
    ///
    /// // Now, since [`DependentMut`] can be used as a normal app handle,
    /// // we can verify that the dependent is the added [`Resource`].
    /// assert_eq!(
    ///     **with_dependent.dependent().dependent_id(),
    ///     Resource::component_id()
    /// );
    ///
    /// // [`DependentMut`] holds a mutable reference onto the handle, so we have to drop it.
    /// drop((dependency, with_dependent));
    /// auditeur::test::finish(app, false).await
    /// # }
    /// ```
    pub async fn with_dependent<D: Send + Sync>(
        self,
        dependency: &Dependency<D>,
    ) -> AppDependentMut<'a, Present, T> {
        let dependent = Dependency::info_of(dependency).id.dependency();
        self.with_dependent_inner(dependent).await
    }

    async fn with_dependent_inner(
        self,
        dependent: LoadedComponentId,
    ) -> AppDependentMut<'a, Present, T> {
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

    fn with_dependency(mut self, dependency: Dependency<()>) -> AppDependentMut<'a, Present, T> {
        self.inner.dependency = Some(std::mem::replace(
            &mut self.inner.app.handle.dependency,
            dependency,
        ));

        AppDependentMut {
            inner: self.inner,
            pd: PhantomData,
        }
    }
}

impl<'a, D> AppDependentMut<'a, D, ()> {
    /// Temporarily changes the dependency type of this [`App`] handle to `ty`.
    ///
    /// This cannot be called multiple times. If you want to change the dependent again, call
    /// [`App::dependent_mut`] another time.
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur::app::map::{DependencyType, ListenerId, RequestSet};
    ///
    /// use auditeur::prelude::*;
    /// # use auditeur::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), auditeur::Error> {
    /// #[resource("test:resource")]
    /// #[pure_resource]
    /// #[create(default)]
    /// struct Resource;
    ///
    /// let mut app = App::builder().with::<Resource>(()).spawn();
    ///
    /// // App handles' default dependency type is usually [`DependencyType::Unremovable`],
    /// // but [`AppBuilder::spawn`]'s default is [`DependencyType::Transient`].
    /// assert_eq!(app.dependent().dependency_type(), DependencyType::Transient);
    ///
    /// // Using [`AppDependentMut`], we could change the dependency type.
    /// let mut with_dependent = app.dependent_mut().with_ty(DependencyType::Unremovable);
    ///
    /// // Now, if we get a dependency to [`Resource`],
    /// let dependency = with_dependent
    ///     .components_mut()
    ///     .load(Resource::component_id())
    ///     .await?;
    /// // its dependency type will be [`DependencyType::Unremovable`].
    /// assert_eq!(dependency.info().ty, DependencyType::Unremovable);
    ///
    /// // And if we request [`Resource`]'s removal,
    /// drop(with_dependent);
    /// app.components_mut()
    ///     .request(RequestSet::remove(Resource::component_id()))
    ///     .await;
    /// // the request will propagate to the app, since it's `dependency`'s dependent.
    /// let map = app.map().get().await;
    /// assert!(
    ///     !map.listeners[&ListenerId::deactivate_component(app.dependent().app_id())]
    ///         .requests
    ///         .is_empty()
    /// );
    ///
    /// drop(dependency);
    /// auditeur::test::finish(app, false).await
    /// # }
    /// ```
    #[must_use]
    pub fn with_ty(mut self, ty: DependencyType) -> AppDependentMut<'a, D, Present> {
        self.inner.ty = Some(std::mem::replace(
            &mut self.inner.app.handle.new_dependency_type,
            ty,
        ));

        AppDependentMut {
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

impl<D, T> Deref for AppDependentMut<'_, D, T> {
    type Target = App;
    fn deref(&self) -> &Self::Target {
        self.inner.app
    }
}

impl<D, T> DerefMut for AppDependentMut<'_, D, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.app
    }
}

/// A way to spawn or run an [`App`].
///
/// Use the [`HasChildren`] trait, which this implements, to add children to the app before running
/// it.
///
/// Create one of these using [`App::builder`].
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

    /// Starts running the [`App`] as a [`tokio::task`] and returns an app handle.
    ///
    /// This app handle is what initially keeps the app alive. If dropped, the app will
    /// automatically exit unless other handles, such as those created in
    /// [`on_add`](crate::component::registry::Loader::on_add) hooks, are keeping the app alive.
    ///
    /// The [dependent](self#dependents) of the returned handle will be the [`App`] component, and
    /// its [`DependencyType`] will default to [`DependencyType::Transient`].
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

    /// Starts running the [`App`] as a [`tokio::task`] and waits for it to exit.
    ///
    /// This is identical to `self.spawn().wait_for_exit().await`.
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

/// Provides implementations of [`AppFnOnce`] and [`AppMutFnOnce`] for [`App`] methods.
pub struct AppFn;
impl AppFn {
    /// See [`App::components_mut`].
    pub const fn components_mut(self) -> AppFnComponentsMut {
        AppFnComponentsMut
    }

    /// See [`App::map`].
    pub const fn map(self) -> AppFnMap {
        AppFnMap
    }

    /// See [`App::errors`].
    pub const fn errors(self) -> AppFnErrors {
        AppFnErrors
    }

    /// See [`App::errors_mut`].
    pub const fn errors_mut(self) -> AppFnErrorsMut {
        AppFnErrorsMut
    }
}

impl AppFn {
    /// See [`App::resources_mut`].
    pub const fn resources_mut(self) -> AppFnResourcesMut {
        AppFnResourcesMut
    }
}

impl AppFn {
    /// See [`App::handle`].
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

    /// See [`App::exit`].
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
pub(crate) enum Method {
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

#[doc(hidden)]
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
#[doc(hidden)]
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

/// A value that keeps a dependency alive as long as it lives.
///
/// Values from other components will typically be wrapped in this type to keep the other component
/// alive. A dependency is incremented when a value of this type is created and decremented when it
/// is dropped. Only when all individual dependency parts are dropped will the whole dependency be
/// removed in the app.
#[derive(Debug)]
pub struct Dependency<T> {
    pub(super) value: T,
    pub(super) handle: map::DependencyHandle,
}

impl<T> Dependency<T> {
    pub const fn info_of(this: &Self) -> &DependencyPart {
        &this.handle.id
    }
}

impl Dependency<()> {
    #[must_use]
    pub const fn info(&self) -> &DependencyPart {
        Self::info_of(self)
    }

    pub(crate) fn insert<T>(self, value: T) -> Dependency<T> {
        Dependency {
            value,
            handle: self.handle,
        }
    }
}

impl<T: Send> Dependency<T> {
    pub async fn clone_with<U: Send>(
        this: &Self,
        app: &mut App,
        func: impl AsyncFnOnce(&T, &mut App) -> U + Send,
    ) -> Dependency<U>
    where
        T: Sync,
    {
        let dependency = app
            .components_mut()
            .increment_dependency(this.handle.id.clone())
            .await;
        let value = func(&this.value, app).await;
        Dependency {
            value,
            handle: dependency.handle,
        }
    }

    fn clone_with_owned<U: Send>(
        this: &Self,
        owned: &mut Owned,
        sender: &flume::Sender<crate::app::Method>,
        func: impl FnOnce(&T, &mut Owned) -> U + Send,
    ) -> Dependency<U> {
        let dependency = owned.increment_dependency(this.handle.id.clone(), sender);
        let value = func(&this.value, owned);
        Dependency {
            value,
            handle: dependency.handle,
        }
    }

    fn clone_in_owned(
        this: &Self,
        owned: &mut Owned,
        sender: &flume::Sender<crate::app::Method>,
    ) -> Self
    where
        T: Clone,
    {
        Self::clone_with_owned(this, owned, sender, |val, _| val.clone())
    }

    pub(crate) fn release_unsafe(this: &mut Self) {
        this.handle.release_unsafe();
    }

    /// Drops this dependency using a specific app handle.
    ///
    /// This can be more efficient than the default drop implementation, but only if the app handle
    /// is the owned app handle. Otherwise, it will do the same exact thing as the automatic drop.
    ///
    /// # Panics
    ///
    /// - If `app` is not the app that this dependency was obtained in.
    pub fn drop_in(this: Self, app: &mut App) -> impl AppMutFuture<'_, Output = ()> {
        drop(this.value);
        this.handle.drop_in(app)
    }

    /// Drops this dependency in an owned app.
    ///
    /// # Panics
    ///
    /// - Potential panic if `owned` is not the app that this dependency was obtained in.
    fn drop_in_owned(this: Self, owned: &mut Owned, sender: &flume::Sender<crate::app::Method>) {
        drop(this.value);
        this.handle.drop_in_owned(owned, sender);
    }
}

impl<T> Deref for Dependency<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Dependency<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: CloneIn> CloneIn for Dependency<T> {
    async fn clone_in(&self, app: &mut App) -> Self
    where
        Self: Sized,
    {
        Self::clone_with(self, app, async |inner, app| inner.clone_in(app).await).await
    }
}

/// A value that determines if a [`Dependency`] can be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyType {
    /// The dependency cannot be removed. If this dependency is requested to be removed, the request
    /// will propagate to the dependent.
    Unremovable,
    /// The dependency can be removed, but the app has to ask. The request will not propagate to
    /// the dependent, and instead an [`Event::RequestDependencyRemoval`] will be sent.
    Removable,
    /// The dependency is short-lived and will be removed automatically without the app asking. The
    /// request will not propagate to the dependent.
    Transient,
    /// The dependency should not keep the depended-on component alive. Instead, this dependency
    /// is expected to automatically be removed when the **depended-on component** is unloaded.
    ///
    /// Note: Don't use this unless you know what you're doing. It's very rare that you don't want
    /// to keep the depended-on component alive. Additionally, the depended-on component has to
    /// know about the dependency, which is even more rare. Instead, use [`Self::Transient`] as a
    /// good default.
    Weak,
}

impl DependencyType {
    /// Returns `true` if the dependency type is [`Weak`].
    ///
    /// [`Weak`]: DependencyType::Weak
    #[must_use]
    pub const fn is_weak(&self) -> bool {
        matches!(self, Self::Weak)
    }
}
