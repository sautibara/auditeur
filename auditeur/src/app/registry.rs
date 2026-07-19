use std::{fmt::Debug, marker::PhantomData};

use hashbrown::HashMap;
use tokio::sync::oneshot;

use crate::{
    app::{
        AddedComponentId, AppFn, AppMutFnOnce, AppMutFuture, OneshotSender,
        map::{Dependency, DependencyType, LoadedComponentId},
    },
    component::{Component, ComponentIdPath, IdentifiedComponent, IdentifiedType, RegistryOf},
    prelude::*,
};

type RegistryId = &'static NamespacedIdRef;

/// Represents a grouping of [`Identified`] types that can be added as [`Component`]s to an [`App`].
///
/// Components are [loaded](Loader::load) on demand when other components
/// [`get`](crate::app::Registries::get) them. When all dependents release their dependency,
/// they're also automatically [unloaded](Loaded::unload). The component then stays in the app,
/// ready to load again if another component needs it.
///
/// Components are added to an [`App`] as [`Children`] using the
/// [`add`](crate::app::Registries::load) method. They are only removed from the app if someone
/// calls [`remove`](crate::app::Registries::remove), which prevents other components from
/// ever loading the component again.
///
/// Notable registries include:
/// - [`Registries`],
/// - [`Resources`],
/// - [`Bundles`],
/// - [`TakeMessages`],
/// - [`ListenMessages`],
pub trait Registry: IdentifiedType + 'static {
    type Loader: Loader<Loaded = Self::Loaded>;
    type Loaded: Loaded;
}

/// A value that can load a specific erased [`Component`] on demand.
pub trait Loader<L: Loader<Loaded = Self::Loaded> = Self>:
    IdentifiedComponent + Sized + Send + Sync + 'static
{
    type Loaded: Loaded;

    /// Loads the underlying component using an app handle.
    ///
    /// The app handle is guaranteed to have the component as the dependent.
    fn load(&self, app: &mut App) -> impl Future<Output = Result<Self::Loaded, Error>> + Send;

    /// A callback for when the component is added to the app.
    ///
    /// The app handle is guaranteed to have the app as the dependent.
    fn on_add(&self) -> impl LoaderCallback<L, Output = ()> {
        LoaderCallbacks::none()
    }

    /// A callback for when a component is initially loaded.
    ///
    /// This will only ever be run once per load of the component. That is, it will only run
    /// multiple times if the component unloads and then reloads. It also will be called even if
    /// the component only lazy loads.
    ///
    /// The default implementation is to load the component again to ensure that lazy loads fully
    /// load components. If changing the default implementation, make sure to load the component
    /// in some way, unless the loading and unloading of the component are unimportant.
    fn on_load(&self) -> impl LoaderCallback<L, Output = ()> {
        LoaderCallbacks::load(DependencyType::Transient).discard()
    }
}

/// An erased [`Component`] that has been loaded into a [`Registry`].
pub trait Loaded: crate::Any + Sized + Send + Sync + 'static {
    /// Unloads the underlying component using an app handle.
    ///
    /// The app handle is guaranteed to have the app as a dependent.
    fn unload(self, app: &mut App) -> impl Future<Output = Result<(), Error>> + Send {
        let _ = app;
        async { Ok(()) }
    }
}

pub trait LoaderCallback<L: Loader>: Sized + Send + Sync + 'static {
    type Output: Send + Sync;

    #[doc(hidden)]
    fn call(
        self,
        component: &mut DynComponentImpl<L>,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Output, Error>> + Send;

    fn and_then<T: Send + Sync>(
        self,
        func: impl AsyncFnOnce<(Self::Output,), Output = Result<T, Error>, CallOnceFuture: Send>
        + Send
        + Sync
        + 'static,
    ) -> impl LoaderCallback<L, Output = T> {
        struct AndThen<C, M> {
            callback: C,
            map: M,
        }

        impl<L, C, M, O> LoaderCallback<L> for AndThen<C, M>
        where
            L: Loader,
            C: LoaderCallback<L>,
            M: AsyncFnOnce<(C::Output,), Output = Result<O, Error>, CallOnceFuture: Send>
                + Send
                + Sync
                + 'static,
            O: Send + Sync,
        {
            type Output = O;
            async fn call(
                self,
                component: &mut DynComponentImpl<L>,
                app: &mut App,
            ) -> Result<Self::Output, Error> {
                let output = self.callback.call(component, app).await?;
                (self.map)(output).await
            }
        }

        AndThen {
            callback: self,
            map: func,
        }
    }

    fn discard(self) -> impl LoaderCallback<L, Output = ()> {
        struct Discard<C>(C);
        impl<L: Loader, C: LoaderCallback<L>> LoaderCallback<L> for Discard<C> {
            type Output = ();
            async fn call(
                self,
                component: &mut DynComponentImpl<L>,
                app: &mut App,
            ) -> Result<Self::Output, Error> {
                self.0.call(component, app).await?;
                Ok(())
            }
        }
        Discard(self)
    }

    fn boxed(self) -> BoxedCallback<L, Self::Output> {
        BoxedCallback::new(self)
    }
}

pub struct LoaderCallbacks;
impl LoaderCallbacks {
    #[must_use]
    pub fn none<L: Loader>() -> impl LoaderCallback<L, Output = ()> {
        struct Empty;
        impl<L: Loader> LoaderCallback<L> for Empty {
            type Output = ();
            async fn call(
                self,
                _: &mut DynComponentImpl<L>,
                _: &mut App,
            ) -> Result<Self::Output, Error> {
                Ok(())
            }
        }
        Empty
    }

    #[must_use]
    pub fn load<L: Loader>(ty: DependencyType) -> impl LoaderCallback<L, Output = Dependency<()>> {
        struct Load(DependencyType);
        impl<L: Loader> LoaderCallback<L> for Load {
            type Output = Dependency<()>;
            async fn call(
                self,
                component: &mut DynComponentImpl<L>,
                app: &mut App,
            ) -> Result<Self::Output, Error> {
                let dependency = LoaderCallbacks::load_lazy(self.0)
                    .call(component, app)
                    .await?;

                let mut app = app.dependent_mut().with_dependent(&dependency).await;
                let _ = component.get_loaded(&mut app).await?;

                Ok(dependency)
            }
        }
        Load(ty)
    }

    #[must_use]
    pub fn load_lazy<L: Loader>(
        ty: DependencyType,
    ) -> impl LoaderCallback<L, Output = Dependency<()>> {
        struct Load(DependencyType);
        impl<L: Loader> LoaderCallback<L> for Load {
            type Output = Dependency<()>;
            async fn call(
                self,
                component: &mut DynComponentImpl<L>,
                app: &mut App,
            ) -> Result<Self::Output, Error> {
                // NOTE: we know that the component is added b/c we have its ErasedComponent
                let component_id = AddedComponentId::new_inner(component.component_id_dyn());
                let app_id = app.dependent().app_id();
                let dependency = app
                    .map_mut()
                    .increment_loaded_dependency(component_id, app_id, self.0)
                    .await;

                Ok(dependency)
            }
        }
        Load(ty)
    }

    pub fn get<L: Loader, T: Send + Sync>(
        ty: DependencyType,
        func: impl for<'a, 'b> AsyncFnOnce<
            (&'a mut L::Loaded, &'b mut App),
            Output = Result<T, Error>,
            CallOnceFuture: Send,
        > + Send
        + Sync
        + 'static,
    ) -> impl LoaderCallback<L, Output = Dependency<T>> {
        struct Get<F> {
            ty: DependencyType,
            func: F,
        }

        impl<L, T, F> LoaderCallback<L> for Get<F>
        where
            L: Loader,
            T: Send + Sync,
            F: for<'a, 'b> AsyncFnOnce<
                    (&'a mut L::Loaded, &'b mut App),
                    Output = Result<T, Error>,
                    CallOnceFuture: Send,
                > + Send
                + Sync
                + 'static,
        {
            type Output = Dependency<T>;

            async fn call(
                self,
                component: &mut DynComponentImpl<L>,
                app: &mut App,
            ) -> Result<Self::Output, Error> {
                let dependency = LoaderCallbacks::load_lazy(self.ty)
                    .call(component, app)
                    .await?;

                let mut app = app.dependent_mut().with_dependent(&dependency).await;
                let loaded = component.get_loaded(&mut app).await?;
                let val = (self.func)(loaded, &mut app).await?;
                Ok(dependency.insert(val))
            }
        }

        Get { ty, func }
    }
}

pub enum EitherCallback<L, R> {
    Left(L),
    Right(R),
}

impl<O, Lo, L, R> LoaderCallback<Lo> for EitherCallback<L, R>
where
    O: Send + Sync,
    Lo: Loader,
    L: LoaderCallback<Lo, Output = O>,
    R: LoaderCallback<Lo, Output = O>,
{
    type Output = O;
    async fn call(
        self,
        component: &mut DynComponentImpl<Lo>,
        app: &mut App,
    ) -> Result<Self::Output, Error> {
        match self {
            Self::Left(left) => left.call(component, app).await,
            Self::Right(right) => right.call(component, app).await,
        }
    }
}

#[async_trait]
trait DynLoaderCallback<L: Loader>: Send + Sync {
    type Output: Send + Sync;
    async fn call_dyn(
        self: Box<Self>,
        component: &mut DynComponentImpl<L>,
        app: &mut App,
    ) -> Result<Self::Output, Error>;
}

#[async_trait]
impl<L: Loader, C: LoaderCallback<L>> DynLoaderCallback<L> for C {
    type Output = C::Output;
    async fn call_dyn(
        self: Box<Self>,
        component: &mut DynComponentImpl<L>,
        app: &mut App,
    ) -> Result<Self::Output, Error> {
        self.call(component, app).await
    }
}

pub struct BoxedCallback<L: Loader, O: Send + Sync> {
    callback: Box<dyn DynLoaderCallback<L, Output = O>>,
}

impl<L: Loader, O: Send + Sync> BoxedCallback<L, O> {
    fn new<C: LoaderCallback<L, Output = O>>(callback: C) -> Self {
        Self {
            callback: Box::new(callback),
        }
    }
}

impl<L: Loader, O: Send + Sync + 'static> LoaderCallback<L> for BoxedCallback<L, O> {
    type Output = O;

    async fn call(
        self,
        component: &mut DynComponentImpl<L>,
        app: &mut App,
    ) -> Result<Self::Output, Error> {
        self.callback.call_dyn(component, app).await
    }

    fn boxed(self) -> BoxedCallback<L, Self::Output> {
        self
    }
}

/// A loader implementation for marker components.
pub struct MarkerLoader<R: Registry> {
    id: ComponentIdPath,
    pd: PhantomData<fn() -> R>,
}

impl<R: Registry> MarkerLoader<R> {
    #[must_use]
    pub fn new(id: impl Into<ComponentIdPath>) -> Self {
        Self {
            id: id.into(),
            pd: PhantomData,
        }
    }
}

impl<R: Registry> IdentifiedComponent for MarkerLoader<R> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId {
            registry: R::id(),
            component: self.id.clone(),
        }
    }
}

impl<R: Registry> Loader for MarkerLoader<R> {
    type Loaded = ();
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, Error> {
        Ok(())
    }
}

impl Loaded for () {}

/// A loader implementation that uses trait objects.
pub struct ErasedLoader<D: Loaded> {
    loader: Box<dyn DynLoader<D>>,
}

impl<D: Loaded> ErasedLoader<D> {
    #[must_use]
    pub fn new<L: Loader<Self, Loaded = D>>(loader: L) -> Self {
        Self {
            loader: Box::new(loader),
        }
    }
}

impl<D: Loaded> Loader for ErasedLoader<D> {
    type Loaded = D;

    async fn load(&self, app: &mut App) -> Result<Self::Loaded, Error> {
        self.loader.load_dyn(app).await
    }

    fn on_add(&self) -> impl LoaderCallback<Self, Output = ()> {
        self.loader.on_add_dyn()
    }

    fn on_load(&self) -> impl LoaderCallback<Self, Output = ()> {
        self.loader.on_load_dyn()
    }
}

impl<D: Loaded> IdentifiedComponent for ErasedLoader<D> {
    fn component_id_dyn(&self) -> ComponentId {
        self.loader.component_id_dyn()
    }
}

#[async_trait]
trait DynLoader<D: Loaded>: IdentifiedComponent + Send + Sync {
    async fn load_dyn(&self, app: &mut App) -> Result<D, Error>;
    fn on_add_dyn(&self) -> BoxedCallback<ErasedLoader<D>, ()>;
    fn on_load_dyn(&self) -> BoxedCallback<ErasedLoader<D>, ()>;
}

#[async_trait]
impl<D: Loaded, L: Loader<ErasedLoader<D>, Loaded = D>> DynLoader<D> for L {
    async fn load_dyn(&self, app: &mut App) -> Result<D, Error> {
        <Self as Loader<ErasedLoader<D>>>::load(self, app).await
    }

    fn on_add_dyn(&self) -> BoxedCallback<ErasedLoader<D>, ()> {
        <Self as Loader<ErasedLoader<D>>>::on_add(self).boxed()
    }

    fn on_load_dyn(&self) -> BoxedCallback<ErasedLoader<D>, ()> {
        <Self as Loader<ErasedLoader<D>>>::on_load(self).boxed()
    }
}

/// A [`Registry`] for all registries added to an [`App`].
#[identified("auditeur:registry")]
pub struct Registries;

impl Registry for Registries {
    type Loader = MarkerLoader<Self>;
    type Loaded = ();
}

impl RegistryOf for Registries {
    type Registry = Self;
}

impl<R: Registry> Component<Registries> for R {
    type ComponentInput = ();

    fn register((): Self::ComponentInput) -> (<Registries as Registry>::Loader, Children) {
        (MarkerLoader::new(R::id()), Children::new())
    }
}

pub struct ErasedComponent {
    pub(crate) object: Box<dyn DynComponent>,
}

impl Debug for ErasedComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedComponent")
            .field("id", &self.object.component_id_dyn())
            .finish_non_exhaustive()
    }
}

impl ErasedComponent {
    pub fn new<R: Registry, C: Component<R>>(input: C::ComponentInput) -> (Self, Children) {
        let (loader, children) = C::register(input);
        let component = Self {
            object: Box::new(DynComponentImpl::<R::Loader> {
                loader,
                loaded: None,
            }),
        };
        (component, children)
    }

    #[allow(clippy::similar_names)]
    pub(super) fn from_loader<R: Registry>(loader: R::Loader) -> Self {
        Self {
            object: Box::new(DynComponentImpl::<R::Loader> {
                loader,
                loaded: None,
            }),
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait DynComponent: IdentifiedComponent + Send + Sync {
    /// Gets a reference to the underlying component, loading it if necessary.
    ///
    /// # Errors
    ///
    /// - Propagated from [`Loader::load`].
    async fn get<'a, 'b>(
        &'a mut self,
        app: &'b mut App,
    ) -> Result<&'a mut (dyn crate::Any + Send + Sync + 'static), Error>;
    /// Unloads the component from the app.
    ///
    /// # Errors
    ///
    /// - Propagated from [`Loaded::unload`].
    async fn unload(&mut self, app: &mut App) -> Result<(), Error>;
    /// Runs code intended for when the component is added to the app.
    async fn on_add(&mut self, app: &mut App) -> Result<(), Error>;
    /// Runs code intended for when the component is loaded in the app.
    async fn on_load(&mut self, app: &mut App) -> Result<(), Error>;
    /// Returns `true` iff the component is currently loaded
    fn is_loaded(&self) -> bool;
}

struct DynComponentImpl<L: Loader> {
    loader: L,
    loaded: Option<L::Loaded>,
}

impl<L: Loader> DynComponentImpl<L> {
    async fn get_loaded(&mut self, app: &mut App) -> Result<&mut L::Loaded, Error> {
        if self.loaded.is_none() {
            let loaded = self.loader.load(app).await?;
            self.loaded = Some(loaded);
        }

        // me when problem case 3
        // SAFETY: always some b/c it gets initialized if it's none
        Ok(unsafe { self.loaded.as_mut().unwrap_unchecked() })
    }
}

impl<L: Loader> IdentifiedComponent for DynComponentImpl<L> {
    fn component_id_dyn(&self) -> ComponentId {
        self.loader.component_id_dyn()
    }
}

#[async_trait::async_trait]
impl<L: Loader> DynComponent for DynComponentImpl<L> {
    async fn get(&mut self, app: &mut App) -> Result<&mut (dyn crate::Any + Send + Sync), Error> {
        self.get_loaded(app)
            .await
            .map(|reference| reference as &mut (dyn crate::Any + Send + Sync))
    }

    async fn unload(&mut self, app: &mut App) -> Result<(), Error> {
        if let Some(loaded) = self.loaded.take() {
            loaded.unload(app).await?;
        }
        Ok(())
    }

    async fn on_add(&mut self, app: &mut App) -> Result<(), Error> {
        let callback = self.loader.on_add();
        callback.call(self, app).await
    }

    async fn on_load(&mut self, app: &mut App) -> Result<(), Error> {
        let callback = self.loader.on_load();
        callback.call(self, app).await
    }

    fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }
}

#[derive(Default)]
pub(in crate::app) struct Registrar {
    components: HashMap<ComponentId, Register>,
}

#[derive(Debug)]
struct Register {
    component: oneshot::Receiver<ErasedComponent>,
}

impl Registrar {
    /// Adds a component into this registrar.
    ///
    /// # Panics
    ///
    /// - If a component with the same id has already been added.
    pub(super) fn add(&mut self, component: ErasedComponent) {
        let id = component.object.component_id_dyn();

        let (oneshot_sender, receiver) = oneshot::channel();
        let _ = oneshot_sender.send(component);

        let previous = self.components.insert(
            id,
            Register {
                component: receiver,
            },
        );

        assert!(
            previous.is_none(),
            "a name collision should never happen in the registrar, as the map is checked first"
        );
    }

    /// Removes a component from this registrar.
    pub(super) fn remove_unchecked(
        &mut self,
        id: &ComponentId,
        callback: impl FnOnce() + Send + 'static,
    ) {
        if let Some(register) = self.components.remove(id) {
            tokio::task::spawn(async {
                let component = (register.component.await)
                    .expect("a component borrowed from the registry should never be dropped");
                assert!(!component.object.is_loaded());

                drop(component);

                callback();
            });
        } else {
            callback();
        }
    }

    /// Takes a component out of this registrar in order to modify it.
    pub(super) fn borrow(&mut self, id: &ComponentId) -> Option<BorrowedComponent> {
        let register = self.components.get_mut(id)?;

        let (sender, receiver) = oneshot::channel();
        // TODO: dependency loop check
        let receiver = std::mem::replace(&mut register.component, receiver);
        let borrowed = BorrowedComponent { sender, receiver };

        Some(borrowed)
    }
}

#[must_use = "borrowed component shouldn't ever be dropped"]
pub(super) struct BorrowedComponent {
    sender: oneshot::Sender<ErasedComponent>,
    receiver: oneshot::Receiver<ErasedComponent>,
}

impl BorrowedComponent {
    pub(super) async fn modify<T>(self, func: impl AsyncFnOnce(&mut ErasedComponent) -> T) -> T {
        let mut component = (self.receiver.await)
            .expect("a component borrowed from the registry should never be dropped");
        let value = func(&mut component).await;
        let _ = self.sender.send(component);
        value
    }
}

#[derive(Debug)]
pub(super) enum Method {
    Add(Children),
    Borrow {
        dependency: Dependency<()>,
        sender: OneshotSender<Dependency<BorrowedComponent>>,
    },
    SpawnOnLoad(LoadedComponentId),
    SpawnUnload(LoadedComponentId),
    SpawnOnAdd(AddedComponentId),
}

impl Method {
    pub async fn run(self, app: &mut App) {
        match self {
            Self::Add(children) => app.registries_mut().add(children).await,
            Self::Borrow { dependency, sender } => {
                let component = app.registries_mut().borrow(dependency).await;
                let () = sender.send(component);
            }
            Self::SpawnOnLoad(id) => {
                app.registries_mut().spawn_on_load(id).await;
            }
            Self::SpawnUnload(id) => {
                app.registries_mut().spawn_unload(id).await;
            }
            Self::SpawnOnAdd(id) => app.registries_mut().spawn_on_add(id).await,
        }
    }
}

/// Return type of [`AppFn::registries_mut()`], see [`AppRegistriesMut`].
#[must_use]
pub struct AppFnRegistriesMut;
impl AppFnRegistriesMut {
    /// See [`AppRegistriesMut::add`].
    #[allow(clippy::should_implement_trait)]
    pub const fn add(self, mut children: Children) -> impl AppMutFnOnce<Output = ()> {
        AppFn::using_handle_mut(move |handle| {
            children.adopt_by(handle.dependent_id().component.clone_id());
            AppFn::spawn_mut(
                children,
                |children, _| super::Method::Registries(Method::Add(children)),
                async |children, owned, handle| {
                    owned.add_children(children, &handle.method_sender);
                },
            )
        })
    }
}

/// Return type of [`App::registries_mut()`].
pub struct AppRegistriesMut<'a> {
    app: &'a mut App,
}

impl<'a> AppRegistriesMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Add components to the app as children of the current dependent.
    ///
    /// # Errors
    ///
    /// - Visible only in [`App::errors`] - not returned.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, children: Children) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.registries_mut().add(children).call(self.app)
    }

    /// Gets a component from one of the [registries](Registry).
    ///
    /// This may initialize the component and any components it depends on.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the component with the id `id` doesn't exist.
    /// - [`Error::DowncastFailed`] if a [`crate::Any`] downcast fails.
    /// - Propagated from [`R::Loader::load`].
    /// - Propagated from `func`.
    pub async fn get<R: Registry, T>(
        self,
        id: ComponentIdPath,
        func: impl AsyncFnOnce(&mut R::Loaded, &mut App) -> Result<T, Error>,
    ) -> Result<Dependency<T>, Error>
    where
        T: Send + 'static,
    {
        let id = ComponentId {
            registry: R::id(),
            component: id,
        };

        self.get_inner(id.clone(), async move |reference, app| {
            let reference = reference
                .downcast_mut()
                .ok_or_else(|| Error::DowncastFailed { id })?;
            func(reference, app).await
        })
        .await
    }

    /// Ensures a component is loaded by getting a dependency to it.
    ///
    /// This will initialize the component and any components it depends on if they are not already
    /// initialized.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the component with the id `id` doesn't exist.
    /// - Propagated from [`R::Loader::load`].
    pub async fn load(self, id: ComponentId) -> Result<Dependency<()>, Error> {
        self.get_inner(id, async |_, _| Ok(())).await
    }

    async fn get_inner<T>(
        self,
        id: ComponentId,
        // WARN: The 'b lifetime will always be 'static, but for some reason the compiler gets
        // angry if we don't include it. I have no idea why.
        func: impl for<'b> AsyncFnOnce(
            &mut (dyn crate::Any + Send + Sync + 'b),
            &mut App,
        ) -> Result<T, Error>,
    ) -> Result<Dependency<T>, Error>
    where
        T: Send + 'static,
    {
        let dependent = self.app.dependent().dependent_id();
        let ty = self.app.dependent().dependency_type();
        let dependency = (self.app.map_mut())
            .try_increment_dependency(id, dependent, ty)
            .await?;

        // First take the erased component from the registrar.
        let dependency = self.app.registries_mut().borrow(dependency).await;

        // Then get the values from the dependency necessary to modify the app's dependent.
        let dependency_id = Dependency::info_of(&dependency).id.dependency();
        let ty = Dependency::info_of(&dependency).ty;

        let component = dependency.value;
        let handle = dependency.handle;

        // Then initialize the dependency with with a map that has the dependency as the dependent.
        //
        // This means that dependencies of the dependency have the correct dependent.
        let mut app = (self.app.dependent_mut())
            .with_ty(ty)
            .with_dependent_inner(dependency_id)
            .await;
        let value = component
            .modify(async |component| {
                let reference = component.object.get(&mut app).await?;
                func(reference, &mut app).await
            })
            .await?;

        // And wrap the component back up in the dependency.
        Ok(Dependency { value, handle })
    }

    /// Take an (erased) component from the registrar in order to call get on it.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the component has not been added to this registry.
    /// - [`Error::DependencyLoop`] if the component was already taken previously for a `get` call.
    fn borrow(
        self,
        dependency: Dependency<()>,
    ) -> impl AppMutFuture<'a, Output = Dependency<BorrowedComponent>> {
        AppFn::oneshot_mut(
            dependency,
            |dependency, sender, _| {
                super::Method::Registries(Method::Borrow { dependency, sender })
            },
            async |dependency, owned, _| {
                let component = owned
                    .registrar
                    .borrow(&dependency.info().id.dependency().component)
                    .expect("component should be added to its registry, since we have a dependency to it");

                dependency.insert(component)
            },
        )
        .call(self.app)
    }

    pub(super) async fn spawn_unload(self, id: LoadedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Registries(Method::SpawnUnload(id)),
            async |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id.component) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.unload(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.map_mut().unload_finished(id).await;
                });
            },
        )
        .call(self.app)
        .await;
    }

    async fn spawn_on_load(self, id: LoadedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Registries(Method::SpawnOnLoad(id)),
            async |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id.component) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.on_load(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.map_mut().on_load_finished(id).await;
                });
            },
        )
        .call(self.app)
        .await;
    }

    async fn spawn_on_add(self, id: AddedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Registries(Method::SpawnOnAdd(id)),
            async |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.on_add(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.map_mut().on_add_finished(id).await;
                });
            },
        )
        .call(self.app)
        .await;
    }
}
