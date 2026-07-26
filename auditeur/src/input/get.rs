use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use async_trait::async_trait;

use crate::any;
use crate::input::CreateDeriveTypes;
use crate::{app::map::Dependency, component::resource::Resource, prelude::*};

pub struct Get<T: GetRef> {
    reference: Dependency<T::Ref>,
}

impl<T: Resource + GetRef> Get<T> {
    pub(crate) const fn new(reference: Dependency<T::Ref>) -> Self {
        Self { reference }
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

/// A type that allows getting a synchronized reference to the original type
#[allow(clippy::module_name_repetitions)] // no
pub trait GetRef<R: GetRef = Self>: Send + Sync {
    type Loader: Send + Sync;
    type Ref: any::Any + Send + Sync;

    fn create_loader(
        resource: &R,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Loader, Error>> + Send;
    fn load_ref(
        loader: &Self::Loader,
        app: &mut App,
    ) -> impl Future<Output = Result<Self::Ref, Error>> + Send;
}

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

pub struct GetInput<I: Input>(PhantomData<I>);
impl<I: Input + any::Any + Sync, R: GetRef> GetRef<R> for GetInput<I> {
    type Loader = ();
    type Ref = I;

    async fn create_loader(_: &R, _: &mut App) -> Result<Self::Loader, Error> {
        Ok(())
    }

    async fn load_ref((): &Self::Loader, app: &mut App) -> Result<Self::Ref, Error> {
        I::create(app).await
    }
}

pub struct GetCreate<C>(PhantomData<C>);
impl<C, R> GetRef<R> for GetCreate<C>
where
    C: CreateDerive<External = R, Plain: Sync + Clone> + any::Any + Sync,
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
    use crate::{app::App, input::get::DynGetRefLoaderImpl};

    use super::DynGetRefLoader;

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

    impl<R: super::GetRef + Send + Sync + 'static> Specialization for &mut &R {
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
