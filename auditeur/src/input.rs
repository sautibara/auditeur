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

pub struct Get<M: Message, T: Listener<M> + Clone> {
    phantom_data: PhantomData<(M, T)>,
}

impl<M: Message, T: Listener<M> + Clone> Input<M> for Get<M, T> {
    type Output = T;

    fn create(registry: &Registry<M>, as_resource: NamespacedIdRef) -> Result<Self::Output, Error> {
        registry.get_as(as_resource).ok_or_else(|| Error::NotFound {
            id: T::const_ident().into(),
        })
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_resource(T::const_ident())
    }
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
