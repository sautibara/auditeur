use std::fmt::Debug;

use hashbrown::HashMap;
use tokio::sync::oneshot;
use tracing::{Instrument, debug};

use crate::{
    any,
    app::{
        AddedComponentId, AppComponentsMut, AppFn, AppFnComponentsMut, AppMutFnOnce, AppMutFuture,
        OneshotSender,
    },
    component::{
        ComponentIdPath, IdentifiedComponent, Register,
        registry::{Loaded, Loader, LoaderCallback, Registry},
    },
    dependency::{Dependency, LoadedComponentId},
    prelude::*,
};

/// A [`Component`](crate::component::Component) that is erased into a trait object.
///
/// This is an implementation detail that is public due to its use in
/// [`HasChildren`](crate::component::HasChildren).
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
    pub fn new<R: Register<C>, C>(input: R::ComponentInput) -> (Self, Children) {
        let (loader, children) = R::register(input);
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
pub trait DynComponent: IdentifiedComponent + Send {
    /// Gets a reference to the underlying component, loading it if necessary.
    ///
    /// # Errors
    ///
    /// - Propagated from [`Loader::load`].
    async fn get<'a, 'b>(
        &'a mut self,
        app: &'b mut App,
    ) -> Result<&'a mut (dyn any::Any + Send + 'static), Error>;
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
}

pub struct DynComponentImpl<L: Loader> {
    loader: L,
    loaded: Option<L::Loaded>,
}

impl<L: Loader> DynComponentImpl<L> {
    #[instrument(name = "get", skip_all, fields(id = self.component_id_dyn().to_string()))]
    pub async fn get_loaded(&mut self, app: &mut App) -> Result<&mut L::Loaded, Error> {
        if self.loaded.is_none() {
            debug!("loading...");
            let loaded = self.loader.load(app).instrument(info_span!("load")).await?;
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
    async fn get(&mut self, app: &mut App) -> Result<&mut (dyn any::Any + Send), Error> {
        self.get_loaded(app)
            .await
            .map(|reference| reference as &mut (dyn any::Any + Send))
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
}

#[derive(Default, Debug)]
pub(in crate::app) struct Registrar {
    components: HashMap<ComponentId, StoredComponent>,
}

#[derive(Debug)]
struct StoredComponent {
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
            StoredComponent {
                component: receiver,
            },
        );

        assert!(
            previous.is_none(),
            "a name collision should never happen in the registrar, as the map is checked first"
        );
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

    pub(super) fn remove_unchecked(&mut self, id: &ComponentId) {
        self.components.remove(id);
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
            Self::Add(children) => app.components_mut().add(children).await,
            Self::Borrow { dependency, sender } => {
                let component = app.components_mut().borrow(dependency).await;
                let () = sender.send(component);
            }
            Self::SpawnOnLoad(id) => {
                app.components_mut().spawn_on_load(id).await;
            }
            Self::SpawnUnload(id) => {
                app.components_mut().spawn_unload(id).await;
            }
            Self::SpawnOnAdd(id) => app.components_mut().spawn_on_add(id).await,
        }
    }
}

impl AppFnComponentsMut {
    /// See [`AppComponentsMut::add`].
    #[allow(clippy::should_implement_trait)]
    pub const fn add(self, mut children: Children) -> impl AppMutFnOnce<Output = ()> {
        AppFn::using_handle_mut(move |handle| {
            children.adopt_by(handle.dependent_id().component.clone_id());
            AppFn::spawn_mut(
                children,
                |children, _| super::Method::Registries(Method::Add(children)),
                |children, owned, handle| {
                    owned.add_children(children, &handle.method_sender);
                },
            )
        })
    }

    /// See [`AppComponentsMut::load_lazy`].
    pub const fn load_lazy(
        self,
        id: ComponentId,
    ) -> impl AppMutFnOnce<Output = Result<Dependency<()>, Error>> {
        AppFn::using_handle_mut(|handle| {
            AppFn.components_mut().try_increment_dependency(
                id,
                handle.dependent_id(),
                handle.new_dependency_type,
            )
        })
    }
}

impl<'a> AppComponentsMut<'a> {
    /// Add components to the app as children of the current dependent.
    ///
    /// # Errors
    ///
    /// - Visible only in [`App::errors`] - not returned.
    #[allow(clippy::should_implement_trait)]
    #[instrument(skip(self))]
    pub fn add(self, children: Children) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.components_mut().add(children).call(self.app)
    }

    /// Gets a component from one of the [registries](Registry).
    ///
    /// This may initialize the component and any components it depends on.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the component with the id `id` doesn't exist.
    /// - [`Error::DowncastFailed`] if a [`crate::Any`] downcast fails.
    /// - Propagated from [`R::Loader::load`](Loader::load).
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
    /// - Propagated from [`R::Loader::load`](Loader::load).
    pub async fn load(self, id: ComponentId) -> Result<Dependency<()>, Error> {
        self.get_inner(id, async |_, _| Ok(())).await
    }

    async fn get_inner<T>(
        self,
        id: ComponentId,
        // WARN: The 'b lifetime will always be 'static, but for some reason the compiler gets
        // angry if we don't include it. I have no idea why.
        func: impl for<'b> AsyncFnOnce(&mut (dyn any::Any + Send + 'b), &mut App) -> Result<T, Error>,
    ) -> Result<Dependency<T>, Error>
    where
        T: Send + 'static,
    {
        let dependent = self.app.dependent().dependent_id();
        let ty = self.app.dependent().dependency_type();
        let dependency = (self.app.components_mut())
            .try_increment_dependency(id, dependent, ty)
            .await?;

        // First take the erased component from the registrar.
        let dependency = self.app.components_mut().borrow(dependency).await;

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

    /// Loads a component lazily, ensuring that (if it loads) it will stay loaded as long as the
    /// returned dependency is kept alive. This will not wait for the component to load, and it
    /// is not guaranteed that the component will eventually load.
    ///
    /// However, this method is generally faster than [`load`](Self::load), and it returns an
    /// [`AppMutFuture`], so it can be combined with other app futures. It can also be used to
    /// prevent dependency loops. For example, this is used in [connections](macro@crate::respond)
    /// to keep the resource loaded without actually loading it and causing a dependency loop.
    ///
    /// If this is called when another component has already loaded the dependency with a
    /// [`get`](Self::get) or [`load`](Self::load), this will keep the component alive. If no other
    /// component has loaded the dependency yet, this will ensure that [`Loader::on_load`] is
    /// called on the dependency's loader, which by default loads its component.
    ///
    /// # Errors
    ///
    /// - [`Error::DependencyNotFound`] if the component with the id `id` doesn't exist.
    pub fn load_lazy(
        self,
        id: ComponentId,
    ) -> impl AppMutFuture<'a, Output = Result<Dependency<()>, Error>> {
        AppFn.components_mut().load_lazy(id).call(self.app)
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
            |dependency, owned, _| {
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
            |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id.component) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.unload(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.components_mut().unload_finished(id).await;
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
            |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id.component) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.on_load(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.components_mut().on_load_finished(id).await;
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
            |id, owned, handle| {
                let Some(borrowed) = owned.registrar.borrow(&id) else {
                    return;
                };

                let mut app = handle.clone_in_owned(owned).into_handle();

                tokio::task::spawn(async move {
                    let res = borrowed
                        .modify(async |borrowed| borrowed.object.on_add(&mut app).await)
                        .await;
                    app.errors_mut().drain_result(res).await;
                    app.components_mut().on_add_finished(id).await;
                });
            },
        )
        .call(self.app)
        .await;
    }
}
