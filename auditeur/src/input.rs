use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;

use crate::prelude::*;

pub trait Input<M: Message>: Send + 'static {
    type Output: Send;

    /// Create [`Self::Output`] using the given [`Registry`] and id `as_resource`
    ///
    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(registry: &Registry<M>, as_resource: NamespacedIdRef) -> Result<Self::Output, Error>;

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

    fn create(registry: &Registry<M>, as_resource: NamespacedIdRef) -> Result<Self::Output, Error> {
        registry
            .get_as::<T>(as_resource)
            .ok_or_else(|| Error::LoaderNotFound {
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
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::input;
/// # pub fn main() {
/// use auditeur::GetRef;
/// use auditeur::input::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef)]
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
    type Ref: Send;
    /// Get a synchronized reference to `Self`
    fn get_ref(&self) -> Self::Ref;
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<M: Message, $($T: Input<M>),*> Input<M> for ($($T,)*) {
            type Output = ($($T::Output,)*);

            #[allow(unused_variables)]
            fn create(registry: &Registry<M>, as_resource: NamespacedIdRef) -> Result<Self::Output, Error> {
                Ok(($($T::create(registry, as_resource)?,)*))
            }

            fn constraints() -> Constraints {
                Constraints::empty()
                    $(.extend($T::constraints()))*
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);
