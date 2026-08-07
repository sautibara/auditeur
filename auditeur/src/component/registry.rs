//! Groupings of [`Identified`] values that can be added as [`Component`]s to an [`App`].
//!
//! Components are [loaded](Loader::load) on demand when other components
//! [`get`](crate::app::AppComponentsMut::get) them. When all dependents release their
//! dependencies, the component gets automatically [unloaded](Loaded::unload). The component then
//! stays added in the app, ready to load again if another component needs it.
//!
//! Notable registries include:
//! - [`Registries`],
//! - [`Resources`](crate::component::resource::Resources),
//! - [`Bundles`](crate::component::bundle::Bundles),
//! - [`TakeMessages`](crate::component::take::TakeMessages),
//! - [`ListenMessages`](crate::component::listen::ListenMessages),

use std::marker::PhantomData;

use crate::{
    any,
    app::{id::AddedComponentId, registrar::DynComponentImpl},
    component::{ComponentIdPath, IdentifiedComponent, IdentifiedType, Register, RegistryOf},
    prelude::*,
};
use async_trait::async_trait;

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

impl<R: Registry> Register<R> for Registries {
    type ComponentInput = ();

    fn register((): Self::ComponentInput) -> (<Self as Registry>::Loader, Children) {
        (MarkerLoader::new(R::id()), R::children(Children::new()))
    }
}

/// Groupings of [`Identified`] values that can be added as [`Component`]s to an [`App`].
///
/// See the [module documentation](self) for more information.
pub trait Registry: IdentifiedType + 'static {
    type Loader: Loader<Loaded = Self::Loaded>;
    type Loaded: Loaded;

    /// Provides any children that this registry should be added with.
    ///
    /// This could be backend components that are necessary for the registry to work.
    #[must_use]
    fn children(children: Children) -> Children {
        children
    }
}

/// A value that can load a specific erased [`Component`] on demand.
///
/// The generic parameter `L` represents the loader of the [`Registry`] this loader is loading
/// for. It will most likely be [`Self`], but can be [`ErasedLoader`] if they're being used. See
/// [`ErasedLoader`]'s documentation for more information.
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

/// An erased [`Component`] that has been loaded in a [`Registry`], ready to be used.
pub trait Loaded: any::Any + Sized + Send + 'static {
    /// Unloads the underlying component using an app handle.
    ///
    /// The app handle is guaranteed to have the app as a dependent.
    fn unload(self, app: &mut App) -> impl Future<Output = Result<(), Error>> + Send {
        let _ = app;
        async { Ok(()) }
    }
}

/// A callback that can be run in response to a specific event occurring in a [`Registry`].
///
/// See [`LoaderCallbacks`] for a list of these.
pub trait LoaderCallback<L: Loader>: Sized + Send + 'static {
    type Output: Send;

    #[doc(hidden)]
    fn call(
        self,
        component: &mut DynComponentImpl<L>,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Output, Error>> + Send;

    /// Map the return value of this callback with a fallible async function.
    fn and_then<T: Send>(
        self,
        func: impl AsyncFnOnce<(Self::Output,), Output = Result<T, Error>, CallOnceFuture: Send>
        + Send
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
                + 'static,
            O: Send,
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

    /// Discard the return value of this callback, making it return `()` instead.
    ///
    /// This will not suppress errors.
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

    /// Wrap this callback in a boxed trait object, which uses dynamic dispatch to call.
    fn boxed(self) -> BoxedCallback<L, Self::Output> {
        BoxedCallback::new(self)
    }

    /// Wrap this in an [`EitherCallback`] as the left branch.
    fn left<T>(self) -> EitherCallback<Self, T> {
        EitherCallback::Left(self)
    }

    /// Wrap this in an [`EitherCallback`] as the right branch.
    fn right<T>(self) -> EitherCallback<T, Self> {
        EitherCallback::Right(self)
    }
}

/// A grouping of pre-defined [`LoaderCallback`]s.
pub struct LoaderCallbacks;
impl LoaderCallbacks {
    /// A callback that does nothing but return `()`.
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

    /// A callback that [`load`](crate::app::AppComponentsMut::load)s the component
    /// held in the loader and returns its [`Dependency`] with a [`DependencyType`] of `ty`.
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

    /// A callback that [lazily loads](crate::app::AppComponentsMut::load_lazy)s the
    /// component held in the loader and returns its [`Dependency`] with a [`DependencyType`] of
    /// `ty`.
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
                    .components_mut()
                    .increment_loaded_dependency(component_id, app_id, self.0)
                    .await;

                Ok(dependency)
            }
        }
        Load(ty)
    }

    /// A callback that [`get`](crate::app::AppComponentsMut::get)s the component
    /// held in the loader and returns its [`Dependency`] with a [`DependencyType`] of `ty`.
    pub fn get<L: Loader, T: Send>(
        ty: DependencyType,
        func: impl for<'a, 'b> AsyncFnOnce<
            (&'a mut L::Loaded, &'b mut App),
            Output = Result<T, Error>,
            CallOnceFuture: Send,
        > + Send
        + 'static,
    ) -> impl LoaderCallback<L, Output = Dependency<T>> {
        struct Get<F> {
            ty: DependencyType,
            func: F,
        }

        impl<L, T, F> LoaderCallback<L> for Get<F>
        where
            L: Loader,
            T: Send,
            F: for<'a, 'b> AsyncFnOnce<
                    (&'a mut L::Loaded, &'b mut App),
                    Output = Result<T, Error>,
                    CallOnceFuture: Send,
                > + Send
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

/// A [`LoaderCallback`] that runs one of two different callback variants.
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
trait DynLoaderCallback<L: Loader>: Send {
    type Output: Send;
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

/// A [`LoaderCallback`] that uses dynamic dispatch to run.
pub struct BoxedCallback<L: Loader, O: Send> {
    callback: Box<dyn DynLoaderCallback<L, Output = O>>,
}

impl<L: Loader, O: Send> BoxedCallback<L, O> {
    fn new<C: LoaderCallback<L, Output = O>>(callback: C) -> Self {
        Self {
            callback: Box::new(callback),
        }
    }
}

impl<L: Loader, O: Send + 'static> LoaderCallback<L> for BoxedCallback<L, O> {
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

/// A ready-to-use [`Loader`] implementation for components with no useful loaded data.
pub struct MarkerLoader<R: Registry> {
    id: ComponentIdPath,
    pd: PhantomData<fn() -> R>,
}

impl<R: Registry> MarkerLoader<R> {
    /// Creates a new loader that uses `id` for its [`ComponentId`].
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

/// A ready-to-use [`Loader`] implementation that uses dynamic dispatch.
///
/// Note that loaders to be erased must specifically implement [`Loader<ErasedLoader<_>>`] rather
/// than the default. This is a workaround to allow [`LoaderCallback`]s to work with erased
/// loaders.
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
