use std::{assert_matches, fmt::Debug, marker::PhantomData, ops::Deref};

use hashbrown::HashMap;
use tokio::sync::oneshot;

use crate::{
    app::{AppFn, AppMutFnOnce, AppMutFuture, Inner, map::Dependency},
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

/// A value that can load a specific erased [`Component`]s on demand.
pub trait Loader: IdentifiedComponent + Send + Sync + 'static {
    type Loaded;

    fn load(&self, app: &mut App) -> impl Future<Output = Result<Self::Loaded, Error>> + Send;
}

/// An erased [`Component`] that has been loaded into a [`Registry`].
pub trait Loaded: crate::Any + Sized + Send + Sync + 'static {
    fn unload(self, app: &mut App) -> impl Future<Output = Result<(), Error>> + Send {
        let _ = app;
        async { Ok(()) }
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
pub struct ErasedLoader<R: Registry> {
    loader: Box<dyn DynLoader<R::Loaded>>,
}

impl<R: Registry> ErasedLoader<R> {
    #[must_use]
    pub fn new<LR: Loader<Loaded = R::Loaded>>(loader: LR) -> Self {
        Self {
            loader: Box::new(loader),
        }
    }
}

impl<R: Registry> Loader for ErasedLoader<R> {
    type Loaded = R::Loaded;
    async fn load(&self, app: &mut App) -> Result<Self::Loaded, Error> {
        self.loader.load_dyn(app).await
    }
}

impl<R: Registry> IdentifiedComponent for ErasedLoader<R> {
    fn component_id_dyn(&self) -> ComponentId {
        self.loader.component_id_dyn()
    }
}

#[async_trait]
trait DynLoader<L>: IdentifiedComponent + Send + Sync {
    async fn load_dyn(&self, app: &mut App) -> Result<L, Error>;
}

#[async_trait]
impl<L: Loader> DynLoader<L::Loaded> for L {
    async fn load_dyn(&self, app: &mut App) -> Result<L::Loaded, Error> {
        <Self as Loader>::load(self, app).await
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
            object: Box::new(DynComponentImpl::<R> {
                loader,
                loaded: None,
            }),
        };
        (component, children)
    }

    #[allow(clippy::similar_names)]
    pub(super) fn from_loader<R: Registry>(loader: R::Loader) -> Self {
        Self {
            object: Box::new(DynComponentImpl::<R> {
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
    /// Returns `true` iff the component is currently loaded
    fn is_loaded(&self) -> bool;
}

struct DynComponentImpl<R: Registry> {
    loader: R::Loader,
    loaded: Option<R::Loaded>,
}

impl<R: Registry> IdentifiedComponent for DynComponentImpl<R> {
    fn component_id_dyn(&self) -> ComponentId {
        self.loader.component_id_dyn()
    }
}

#[async_trait::async_trait]
impl<R: Registry> DynComponent for DynComponentImpl<R> {
    async fn get(&mut self, app: &mut App) -> Result<&mut (dyn crate::Any + Send + Sync), Error> {
        if self.loaded.is_none() {
            let loaded = self.loader.load(app).await?;
            self.loaded = Some(loaded);
        }

        // me when problem case 3
        // SAFETY: always some b/c it gets initialized if it's none
        Ok(unsafe { self.loaded.as_mut().unwrap_unchecked() })
    }

    async fn unload(&mut self, app: &mut App) -> Result<(), Error> {
        if let Some(loaded) = self.loaded.take() {
            loaded.unload(app).await?;
        }
        Ok(())
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

        let entry = self.components.entry(id);
        assert_matches!(
            entry,
            hashbrown::hash_map::Entry::Vacant(_),
            "a name collision should never happen in the registrar, as the map is checked first"
        );

        let (sender, receiver) = oneshot::channel();
        let _ = sender.send(component);

        entry.insert(Register {
            component: receiver,
        });
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

pub(super) enum Method {
    Add(Children),
    Borrow {
        dependency: Dependency<()>,
        sender: oneshot::Sender<Dependency<BorrowedComponent>>,
    },
    SpawnLoad {
        dependency: Dependency<()>,
    },
    SpawnUnload {
        dependency: Dependency<()>,
    },
}

impl Method {
    pub async fn run(self, app: &mut App) {
        match self {
            Self::Add(children) => app.registries_mut().add(children).await,
            Self::Borrow { dependency, sender } => {
                let component = app.registries_mut().borrow(dependency).await;
                let _ = sender.send(component);
            }
            Self::SpawnLoad { dependency } => {
                app.registries_mut().spawn_load(dependency).await;
            }
            Self::SpawnUnload { dependency } => {
                app.registries_mut().spawn_unload(dependency).await;
            }
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
            children.adopt_by(handle.dependent_added_id().deref().clone());
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
        let dependent = self.app.dependent().dependent().clone();
        let ty = self.app.dependent().dependency_type();
        let dependency = (self.app.map_mut())
            .try_increment_dependency(id, dependent, ty)
            .await?;

        // First take the erased component from the registrar.
        let dependency = self.app.registries_mut().borrow(dependency).await;

        // Then get the values from the dependency necessary to modify the app's dependent.
        let dependency_id = Dependency::info_of(&dependency).dependency.clone_inner();
        let ty = Dependency::info_of(&dependency).ty;

        let component = dependency.value;
        let handle = dependency.handle;

        // Then initialize the dependency with with a map that has the dependency as the dependent.
        //
        // This means that dependencies of the dependency have the correct dependent.
        let mut app = (self.app.dependent_mut())
            .with_ty(ty)
            .with_added_dependent(dependency_id)
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
                    .borrow(&dependency.info().dependency)
                    .expect("component should be added to its registry, since we have a dependency to it");

                dependency.insert(component)
            },
        )
        .call(self.app)
    }

    pub(super) async fn spawn_load(self, dependency: Dependency<()>) {
        // First take the erased component from the registrar.
        let component = self.app.registries_mut().borrow(dependency).await;

        // Then, with an app handle, we continue loading the component.
        let fut = async move |app: &mut App| {
            let mut app = (app.dependent_mut())
                .with_ty(Dependency::info_of(&component).ty)
                .with_added_dependent(Dependency::info_of(&component).dependent.clone_inner())
                .await;

            Dependency::consume(component, async move |component| {
                component
                    .modify(async |component| {
                        // Then load the component in the current app.
                        let res = component.object.get(&mut app).await;
                        let _ = app.errors_mut().drain_result(res).await;
                    })
                    .await;
            })
            .await;
        };

        // But we can't load it if we're the owned app - it would deadlock in the method queue.
        if matches!(self.app.variant, Inner::Handle) {
            fut(self.app).await;
        } else {
            // So if we are the owned app, we spawn another task that runs it with a handle.
            let mut handle = self.app.handle().await;
            tokio::task::spawn(async move {
                fut(&mut handle).await;
            });
        }
    }

    pub(super) async fn spawn_unload(self, dependency: Dependency<()>) {
        // First take the erased component from the registrar.
        let component = self.app.registries_mut().borrow(dependency).await;

        // Then, with an app handle, we continue unloading the component.
        let fut = async move |app: &mut App| {
            Dependency::consume(component, async move |component| {
                component
                    .modify(async |component| {
                        // Then unload the component in the current app.
                        //
                        // The current app is expected to have the app as the dependent, but it's
                        // not necessary.
                        let res = component.object.unload(app).await;
                        app.errors_mut().drain_result(res).await;
                    })
                    .await;
            })
            .await;
        };

        // But we can't load it if we're the owned app - it would deadlock in the method queue.
        if matches!(self.app.variant, Inner::Handle) {
            fut(self.app).await;
        } else {
            // So if we are the owned app, we spawn another task that runs it with a handle.
            let mut handle = self.app.handle().await;
            tokio::task::spawn(async move {
                fut(&mut handle).await;
            });
        }
    }
}
