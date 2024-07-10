use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;

use crate::{app::Stopper, prelude::*};

pub trait Input<M: Message>: Send + 'static {
    type Output: Send;

    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(registry: &Registry<M>) -> Result<Self::Output, Error>;

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

impl<M: Message> Input<M> for Sender<M> {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.message_sender())
    }
}

impl<M: Message> Input<M> for Exiter {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.exiter())
    }
}

impl<M: Message> Input<M> for Stopper {
    type Output = Self;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        Ok(registry.stopper())
    }
}

pub struct Get<M: Message, T: Listener<M> + Clone> {
    phantom_data: PhantomData<(M, T)>,
}

impl<M: Message, T: Listener<M> + Clone> Input<M> for Get<M, T> {
    type Output = T;

    fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
        registry.get().cloned().ok_or_else(|| Error::NotFound {
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
            fn create(registry: &Registry<M>) -> Result<Self::Output, Error> {
                Ok(($($T::create(registry)?,)*))
            }

            fn constraints() -> Constraints {
                Constraints::empty()
                    $(.extend($T::constraints()))*
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);
