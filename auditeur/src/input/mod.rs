use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;
use futures::Future;

use crate::prelude::*;

pub mod get;
pub mod subset;

pub trait Input: Sized + Send + Sync + 'static {
    /// Create [`Self::Output`] using the given [`Registry`] and id `as_resource`
    ///
    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(
        registry: &Registry,
        as_resource: &NamespacedIdRef,
    ) -> impl Future<Output = Result<Self, Error>> + Send;

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

impl<I: Input> Input for Option<I> {
    async fn create(registry: &Registry, as_resource: &NamespacedIdRef) -> Result<Self, Error> {
        I::create(registry, as_resource).await.map(Some)
    }

    fn constraints() -> Constraints {
        I::constraints()
    }
}

impl<T: Send + Sync + 'static> Input for PhantomData<T> {
    async fn create(_: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self)
    }

    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<$($T: Input),*> Input for ($($T,)*) {
            #[allow(unused_variables)]
            async fn create(registry: &Registry, as_resource: &NamespacedIdRef) -> Result<Self, Error> {
                futures::try_join!($($T::create(registry, as_resource),)*)
            }

            fn constraints() -> Constraints {
                Constraints::empty()
                    $(.extend($T::constraints()))*
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);
