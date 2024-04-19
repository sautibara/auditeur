use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;

use crate::prelude::*;

pub trait Input<M> {
    type Output: Send;

    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(app: &mut App<M>) -> Result<Self::Output, Error>;

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

impl<M: Message> Input<M> for Sender<M> {
    type Output = Self;

    fn create(app: &mut App<M>) -> Result<Self::Output, Error> {
        Ok(app.sender())
    }
}

pub struct Get<M, T: Listener<M> + Clone> {
    phantom_data: PhantomData<(M, T)>,
}

impl<M: Message, T: Listener<M> + Clone> Input<M> for Get<M, T> {
    type Output = T;

    fn create(app: &mut App<M>) -> Result<Self::Output, Error> {
        app.get().ok_or(Error::GetNotFound)
    }

    fn constraints() -> Constraints {
        Constraints::empty().run_after(T::const_ident())
    }
}

#[non_exhaustive]
pub enum Error {
    GetNotFound,
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<M: Message, $($T: Input<M>),*> Input<M> for ($($T,)*) {
            type Output = ($($T::Output,)*);

            #[allow(unused_variables)]
            fn create(app: &mut App<M>) -> Result<Self::Output, InputError> {
                Ok(($($T::create(app)?,)*))
            }

            fn constraints() -> Constraints {
                Constraints::empty()
                    $(.extend($T::constraints()))*
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);
