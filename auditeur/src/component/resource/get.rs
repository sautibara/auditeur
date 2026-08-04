//! Support for getting handles to [`Resource`]s as dependencies.
//!
//! If a resource wants to allow other components to access its data, it can implement [`GetRef`].
//! This will allow other components to access a reference to the other component using the [`Get`]
//! input. A component can also use a default implementation for `GetRef` using [`GetRefWith`].
//!
//! Default implementations include:
//! - [`GetClone`], which gives clones of the resource using [`Clone`],
//! - [`GetCloneIn`], which gives clones of the resource using [`CloneIn`],
//! - [`GetInput`], which gives a type that implements [`Input`],
//! - [`GetCreate`], which gives a type that implements [`Create`] with the resource as external
//!   input,
//!
//! # Examples
//!
//! ```
//! use std::sync::Arc;
//! use std::sync::atomic::{AtomicUsize, Ordering};
//!
//! use auditeur::component::resource::get::{GetClone, GetRefWith};
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! #[resource("test:resource")]
//! #[pure_resource]
//! #[create(default)]
//! #[derive(Clone)]
//! struct Resource {
//!     number: Arc<AtomicUsize>,
//! }
//!
//! // We can use [`GetRefWith`] to use a default implementation for [`GetRef`].
//! impl GetRefWith for Resource {
//!     // The [`GetClone`] implementation simply clones the entire resource type
//!     // and gives it as the handle.
//!     type Method = GetClone;
//! }
//!
//! let mut app = App::builder().with::<Resource>(()).spawn();
//!
//! // Since we used `#[create(default)]`, the stored number starts out at `0`.
//! let resource_1 = app.resources_mut().get::<Resource>().await?;
//! assert_eq!(resource_1.number.load(Ordering::Relaxed), 0);
//!
//! // But since the stored number is shared and atomic, we could change it.
//! resource_1.number.store(1, Ordering::Relaxed);
//!
//! // Now, another component could also get the resource,
//! let resource_2 = app.resources_mut().get::<Resource>().await?;
//! // and find that the stored number is now `1`.
//! assert_eq!(resource_2.number.load(Ordering::Relaxed), 1);
//!
//! drop((resource_1, resource_2));
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use async_trait::async_trait;

use crate::any;
use crate::input::{CloneIn, CreateDeriveTypes};
use crate::{app::map::Dependency, component::resource::Resource, prelude::*};

/// An [`Input`] that uses [`GetRef`] to obtain a handle to another component.
///
/// This wraps a dependency and provides immutable and mutable access to the inner handle.
///
/// You can also obtain one of these using [`AppResourcesMut::get`](super::AppResourcesMut::get).
///
/// See the [module documentation](self) for more information.
pub struct Get<T: GetRef> {
    reference: Dependency<T::Ref>,
}

impl<T: Resource + GetRef> Get<T> {
    pub(crate) const fn new(reference: Dependency<T::Ref>) -> Self {
        Self { reference }
    }

    /// Returns the dependency that this handle wraps.
    pub const fn dependency_of(this: &Self) -> &Dependency<T::Ref> {
        &this.reference
    }
}

impl<T: Resource + GetRef> Input for Get<T> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.resources_mut().get::<T>().await
    }
}

impl<T: GetRef> Deref for Get<T> {
    type Target = T::Ref;
    fn deref(&self) -> &T::Ref {
        &self.reference
    }
}

impl<T: GetRef> DerefMut for Get<T> {
    fn deref_mut(&mut self) -> &mut T::Ref {
        &mut self.reference
    }
}

/// A [`Resource`] that provides handles accessible through a [`Get`] input.
///
/// `R` is the type of the resource being accessed. If this is being implemented manually, this
/// should be [`Self`]. If a default implementation is being created for [`GetRefWith`], this
/// should be the type of the resource.
///
/// See the [module documentation](self) for more information.
#[allow(clippy::module_name_repetitions)] // no
pub trait GetRef<R: GetRef = Self>: Send + Sync {
    type Loader: Send + Sync;
    type Ref: any::Any + Send;

    /// Creates a [`Self::Loader`], who can create [`Self::Ref`]s when necessary.
    ///
    /// This is called exactly once when the resource is
    /// [loaded](crate::component::registry::Loader::load), right before its task is started.
    fn create_loader(
        resource: &R,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Loader, Error>> + Send;

    /// Creates a [`Self::Ref`] using a [`Self::Loader`].
    ///
    /// This is called every time that another component wants to get a handle using [`Get`].
    fn load_ref(
        loader: &Self::Loader,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Ref, Error>> + Send;
}

/// A type that uses a default implementation of [`GetRef`].
pub trait GetRefWith: Send + Sync + Sized {
    type Method: GetRef<Self>;
}

impl<R: GetRefWith> GetRef for R {
    type Loader = <R::Method as GetRef<Self>>::Loader;
    type Ref = <R::Method as GetRef<Self>>::Ref;

    async fn create_loader(resource: &Self, app: &mut App) -> Result<Self::Loader, Error> {
        <R::Method as GetRef<Self>>::create_loader(resource, app).await
    }

    async fn load_ref(loader: &Self::Loader, app: &mut App) -> Result<Self::Ref, Error> {
        <R::Method as GetRef<Self>>::load_ref(loader, app).await
    }
}

/// A [`GetRefWith`] method that gives clones of the resource using [`Clone`].
///
/// Note that the resource is only ever directly cloned once in the loader.
/// All other clones come from that initial clone.
///
/// # Examples
///
/// See the [module documentation](self).
pub struct GetClone;
impl<R: GetRef + Clone + any::Any> GetRef<R> for GetClone {
    type Loader = R;
    type Ref = R;

    async fn create_loader(resource: &R, _: &mut App) -> Result<Self::Loader, Error> {
        Ok(resource.clone())
    }

    async fn load_ref(loader: &Self::Loader, _: &mut App) -> Result<Self::Ref, Error> {
        Ok(loader.clone())
    }
}

/// A [`GetRefWith`] method that gives clones of the resource using [`CloneIn`].
///
/// Note that the resource is only ever directly cloned once in the loader.
/// All other clones come from that initial clone.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// use auditeur::component::resource::get::{GetCloneIn, GetRefWith};
/// use auditeur::prelude::*;
/// # use auditeur::*;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Error> {
/// #[resource("test:resource")]
/// #[pure_resource]
/// #[create(default)]
/// #[derive(CloneIn)]
/// struct Resource {
///     // Values that implement [`Clone`] will automatically use clone instead.
///     number: Arc<AtomicUsize>,
///
///     // But [`CloneIn`] is necessary for values that need an app handle to create.
///     #[create(input)]
///     app: App,
/// }
///
/// impl GetRefWith for Resource {
///     type Method = GetCloneIn;
/// }
///
/// let mut app = App::builder().with::<Resource>(()).spawn();
///
/// // We could use the normally-cloned values just fine.
/// let resource_1 = app.resources_mut().get::<Resource>().await?;
/// assert_eq!(resource_1.number.load(Ordering::Relaxed), 0);
///
/// resource_1.number.store(1, Ordering::Relaxed);
///
/// let resource_2 = app.resources_mut().get::<Resource>().await?;
/// assert_eq!(resource_2.number.load(Ordering::Relaxed), 1);
///
/// // And we can also use the values that need to be cloned with an app handle.
/// let map = resource_1.app.map().get().await;
/// assert!(map.is_added(&Resource::component_id()));
///
/// drop((resource_1, resource_2));
/// auditeur::test::finish(app, false).await
/// # }
/// ```
pub struct GetCloneIn;
impl<R: GetRef + CloneIn + any::Any> GetRef<R> for GetCloneIn {
    type Loader = R;
    type Ref = R;

    async fn create_loader(resource: &R, app: &mut App) -> Result<Self::Loader, Error> {
        Ok(resource.clone_in(app).await)
    }

    async fn load_ref(loader: &Self::Loader, app: &mut App) -> Result<Self::Ref, Error> {
        Ok(loader.clone_in(app).await)
    }
}

/// A [`GetRefWith`] method that gives values of a type that implements [`Input`].
///
/// This doesn't access any data of the resource or otherwise compute anything when the resource is
/// initially loaded. It just calls [`Input::create`] when another component wants to get a handle.
///
/// # Examples
///
/// ```
/// use auditeur::component::resource::get::{GetInput, GetRefWith};
/// use auditeur::prelude::*;
/// # use auditeur::*;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Error> {
/// #[resource("test:resource")]
/// #[pure_resource]
/// #[create(default)]
/// struct Resource;
///
/// impl GetRefWith for Resource {
///     // [`GetInput`] inputs don't have to be related to the resource,
///     // so here we use it to get an [`App`] as the "handle".
///     type Method = GetInput<App>;
/// }
///
/// let mut app = App::builder().with::<Resource>(()).spawn();
///
/// let resource = app.resources_mut().get::<Resource>().await?;
///
/// // Since the [`Get`] just wraps the [`Input`], we could use it however we want.
/// let map = resource.map().get().await;
/// assert!(map.is_added(&Resource::component_id()));
///
/// drop(resource);
/// auditeur::test::finish(app, false).await
/// # }
/// ```
pub struct GetInput<I: Input>(PhantomData<fn() -> I>);
impl<I: Input + any::Any, R: GetRef> GetRef<R> for GetInput<I> {
    type Loader = ();
    type Ref = I;

    async fn create_loader(_: &R, _: &mut App) -> Result<Self::Loader, Error> {
        Ok(())
    }

    async fn load_ref((): &Self::Loader, app: &mut App) -> Result<Self::Ref, Error> {
        I::create(app).await
    }
}

/// A [`GetRefWith`] method that gives values of a type that implements [`Create`].
///
/// The type must implement [`Create`] using the attribute macro [`#[create]`](macro@crate::create),
/// the macro's plain type must implement [`Clone`], and its external input must be the resource.
/// To do this, use `#[create(derive_for_plain(Clone), external_input(<ResourceType>))]`.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// use auditeur::component::resource::get::{GetCreate, GetRefWith};
/// use auditeur::prelude::*;
/// # use auditeur::*;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Error> {
/// #[resource("test:resource")]
/// #[pure_resource]
/// #[create(default)]
/// struct Resource {
///     number: Arc<AtomicUsize>,
/// }
///
/// #[create(external_input(Resource), derive_for_plain(Clone))]
/// #[derive(auditeur::Any)]
/// struct Ref {
///     // The handle can have both values from the original resource,
///     number: Arc<AtomicUsize>,
///
///     // and extra values obtained through the app.
///     #[create(input)]
///     app: App,
/// }
///
/// impl CreateDerive for Ref {
///     async fn create(resource: &Resource, _: &mut App) -> Result<Self::Plain, Error> {
///         Ok(Self::Plain {
///             // To obtain values from the resource, use the external input.
///             number: resource.number.clone(),
///         })
///     }
/// }
///
/// impl GetRefWith for Resource {
///     type Method = GetCreate<Ref>;
/// }
///
/// let mut app = App::builder().with::<Resource>(()).spawn();
///
/// let resource = app.resources_mut().get::<Resource>().await?;
///
/// // Now we can both use values obtained from the original resource,
/// assert_eq!(resource.number.load(Ordering::Relaxed), 0);
///
/// // and values obtained through the app.
/// let map = resource.app.map().get().await;
/// assert!(map.is_added(&Resource::component_id()));
///
/// drop(resource);
/// auditeur::test::finish(app, false).await
/// # }
/// ```
pub struct GetCreate<C>(PhantomData<fn() -> C>);
impl<C, R> GetRef<R> for GetCreate<C>
where
    C: CreateDerive<External = R, Plain: Send + Sync + Clone> + any::Any,
    R: GetRef,
{
    type Loader = C::Plain;
    type Ref = C;

    async fn create_loader(resource: &R, app: &mut App) -> Result<Self::Loader, Error> {
        <C as CreateDerive>::create(resource, app).await
    }

    async fn load_ref(loader: &Self::Loader, app: &mut App) -> Result<Self::Ref, Error> {
        let plain = loader.clone();
        <C as CreateDeriveTypes>::finish(plain, app).await
    }
}

/// A trait that returns [`Some(R::Ref)`](GetRef::Ref) iff the type implements [`GetRef`]
///
/// This can be automatically implemented for all types using autoderef-based specialization,
/// allowing types that don't know if they even implement [`GetRef`] to try to create a
/// synchronized reference.
///
/// To implement the trait, use [`#[resource]`](macro@resource) or
/// [`#[derive(MaybeGetRef)]`](macro@crate::MaybeGetRef)
pub trait MaybeGetRef {
    fn maybe_get_ref(
        &self,
        app: &mut App,
    ) -> impl Future<Output = Result<Option<Box<dyn DynGetRefLoader>>, crate::Error>> + Send;
}

/// An erased [`GetRef::Loader`] implementation, used in [`MaybeGetRef`].
#[async_trait]
pub trait DynGetRefLoader: Send + Sync {
    async fn load_ref(&self, app: &mut App) -> Result<Box<dyn any::Any>, Error>;
}

struct DynGetRefLoaderImpl<R: GetRef>(R::Loader);

#[async_trait]
impl<R: GetRef> DynGetRefLoader for DynGetRefLoaderImpl<R> {
    async fn load_ref(&self, app: &mut App) -> Result<Box<dyn any::Any>, Error> {
        R::load_ref(&self.0, app)
            .await
            .map(|value| Box::new(value) as Box<dyn any::Any>)
    }
}

#[doc(hidden)]
pub mod maybe_get_ref {
    use crate::app::App;

    use super::{DynGetRefLoader, DynGetRefLoaderImpl};

    pub trait Specialization {
        fn specialization(
            &self,
            app: &mut App,
        ) -> impl Future<Output = Result<Option<Box<dyn DynGetRefLoader>>, crate::Error>> + Send;
    }

    impl<T: Send + Sync> Specialization for &T {
        async fn specialization(
            &self,
            _: &mut App,
        ) -> Result<Option<Box<dyn DynGetRefLoader>>, crate::Error> {
            Ok(None)
        }
    }

    impl<R: super::GetRef + 'static> Specialization for &mut &R {
        async fn specialization(
            &self,
            app: &mut App,
        ) -> Result<Option<Box<dyn DynGetRefLoader>>, crate::Error> {
            R::create_loader(self, app)
                .await
                .map(DynGetRefLoaderImpl::<R>)
                .map(Box::new)
                .map(|value| value as Box<dyn DynGetRefLoader>)
                .map(Some)
        }
    }
}
