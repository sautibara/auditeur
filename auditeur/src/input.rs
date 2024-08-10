use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;
use futures::Future;

use crate::prelude::*;

pub trait Input<M: Message>: Send + 'static {
    type Output: Send + Sync;

    /// Create [`Self::Output`] using the given [`Registry`] and id `as_resource`
    ///
    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(
        registry: &Registry<M>,
        as_resource: NamespacedIdRef,
    ) -> impl Future<Output = Result<Self::Output, Error>> + Send;

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

pub struct Get<M: Message, T: Listener<M> + GetRef> {
    phantom_data: PhantomData<(M, T)>,
}

impl<M: Message, T: Listener<M> + GetRef> Input<M> for Get<M, T> {
    type Output = T::Ref;

    async fn create(
        registry: &Registry<M>,
        as_resource: NamespacedIdRef<'_>,
    ) -> Result<Self::Output, Error> {
        registry
            .get_as::<T>(as_resource)
            .await
            .ok_or_else(|| Error::DowncastFailed {
                id: T::const_ident().into(),
            })
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_resource(T::const_ident())
    }
}

/// A type that allows getting a synchronized reference to the original type
///
/// Most of the time, this is implemented using the [derive macro](crate::GetRef). The macro falls
/// back to [`Clone`], which works for special pointers like [`Arc<_>`](std::sync::Arc).
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::{input, path_any};
/// # pub fn main() {
/// use auditeur::prelude::*;
/// use auditeur::input::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef, PathAny)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let reference: Shared = shared.get_ref();
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
pub trait GetRef {
    /// A synchronized reference to the original [`Resource`]
    type Ref: Send + Sync + crate::Any;
    /// Get a synchronized reference to `Self`
    fn get_ref(&self) -> Self::Ref;
}

/// A trait that returns [`Some(R::Ref)`](GetRef::Ref) if the type implements [`GetRef`], or [`None`] otherwise
///
/// This can be automatically implemented for all types using autoderef-based specialization,
/// allowing types that don't know if they even implement [`GetRef`] to try to create a
/// synchronized reference. See [`crate::app::GetRequest`] for why this is necessary.
///
/// To implement the trait, use [`#[resource]`](macro@resource) or
/// [`#[derive(MaybeGetRef)]`](macro@crate::MaybeGetRef)
#[cfg(feature = "runtime-loading")]
pub trait MaybeGetRef {
    fn maybe_get_ref(&self) -> Option<Box<dyn crate::Any + Send + Sync>>;
}

#[doc(hidden)]
#[cfg(feature = "runtime-loading")]
pub mod maybe_get_ref {
    pub trait Specialization {
        fn specialization(&self) -> Option<Box<dyn crate::Any + Send + Sync>>;
    }

    impl<T> Specialization for &T {
        fn specialization(&self) -> Option<Box<dyn crate::Any + Send + Sync>> {
            None
        }
    }

    impl<T: super::GetRef + crate::Any> Specialization for &mut &T {
        fn specialization(&self) -> Option<Box<dyn crate::Any + Send + Sync>> {
            Some(Box::new(self.get_ref()))
        }
    }
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<M: Message, $($T: Input<M>),*> Input<M> for ($($T,)*) {
            type Output = ($($T::Output,)*);

            #[allow(unused_variables)]
            async fn create(registry: &Registry<M>, as_resource: NamespacedIdRef<'_>) -> Result<Self::Output, Error> {
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
