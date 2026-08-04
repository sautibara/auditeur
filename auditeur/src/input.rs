use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;
use futures::Future;

// TODO: implement Input for NamespacedId, make it the resource's id

use crate::prelude::*;

/// Values that can be obtained from an [`App`] using no external data.
///
/// # Dependencies
///
/// These must hold [`dependency::Handle`]s for all dependencies that these inputs contain. If this
/// is derived, then it is already handled for you. But if it's done manually, then please make
/// sure that this is the case.
pub trait Input: Sized + Send + 'static {
    /// Create `Self` using the given [`Registry`] and id `as_resource`
    ///
    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(app: &mut App) -> impl Future<Output = Result<Self, Error>> + Send;
}

impl<T: Send + 'static> Input for PhantomData<T> {
    async fn create(_: &mut App) -> Result<Self, Error> {
        Ok(Self)
    }
}

impl Input for App {
    async fn create(app: &mut App) -> Result<Self, Error> {
        Ok(app.handle().await)
    }
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<$($T: Input),*> Input for ($($T,)*) {
            #[allow(unused_variables)]
            async fn create(app: &mut App) -> Result<Self, Error> {
                Ok(($($T::create(app).await?,)*))
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);

pub trait Create: Sized + Send {
    type ExternalInput: Send + Sync;

    // TODO: accept a Result
    #[allow(opaque_hidden_inferred_bound)] // false positive?
    fn create(
        external: &Self::ExternalInput,
        app: &mut App,
    ) -> impl Future<Output = Result<Self, Error>> + Send;
}

impl<T: CreateDerive> Create for T {
    type ExternalInput = <Self as CreateDeriveTypes>::External;

    async fn create(external: &Self::ExternalInput, app: &mut App) -> Result<Self, Error> {
        let plain = <Self as CreateDerive>::create(external, app).await?;
        Self::finish(plain, app).await
    }
}

pub trait CreateDeriveTypes: Sized + Send {
    type External: Send + Sync;
    type Plain: Send;

    fn finish(
        plain: Self::Plain,
        app: &mut App,
    ) -> impl Future<Output = Result<Self, Error>> + Send;
}

pub trait CreateDerive: CreateDeriveTypes {
    fn create(
        _: &Self::External,
        _: &mut App,
    ) -> impl Future<Output = Result<Self::Plain, Error>> + Send;
}

// TODO: docs
pub trait CloneIn: Send + Sync + Sized {
    #[must_use]
    fn clone_in(&self, app: &mut App) -> impl Future<Output = Self> + Send;
}

impl CloneIn for () {
    async fn clone_in(&self, _: &mut App) -> Self {}
}

#[doc(hidden)]
pub mod clone_in {
    use crate::app::App;

    pub trait Specialization<T> {
        fn specialization(&self, app: &mut App) -> impl Future<Output = T> + Send;
    }

    impl<T: Clone + Send + Sync> Specialization<T> for &T {
        async fn specialization(&self, _: &mut App) -> T {
            (*self).clone()
        }
    }

    impl<T: super::CloneIn> Specialization<T> for &mut &T {
        async fn specialization(&self, app: &mut App) -> T {
            self.clone_in(app).await
        }
    }
}
