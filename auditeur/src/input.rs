//! Values that can be created using an [`App`].
//!
//! Generally, values that are simple resources obtained from the app should implement [`Input`].
//! Meanwhile, values that have some runtime data associated with them should implement [`Create`].
//!
//! As such, [`Create`] is used in situations like [`Resource`](mod@crate::component::resource)'s
//! [`Register`](crate::component::Register) implementation, where it is used to allow
//! them to be initialized with some external input. On the other hand, [`Input`] is mainly used in
//! the [`#[create]`](macro@crate::create) macro.

use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;
use futures::Future;

use crate::prelude::*;

/// Values that can be obtained from an [`App`] using no external input.
///
/// This trait can be implemented using the [`Input`](macro@crate::Input) derive macro.
///
/// See the [module documentation](self) for more information.
pub trait Input: Sized + Send + 'static {
    /// Obtains `Self` from the given [`App`].
    ///
    /// # Errors
    ///
    /// - If any dependencies fail to be obtained.
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

/// Values that can be created in an [`App`] using external input.
///
/// This trait can be implemented using the [`#[create]`](macro@crate::create) attribute macro.
///
/// See the [module documentation](self) for more information.
pub trait Create: Sized + Send {
    type ExternalInput: Send + Sync;

    /// Creates `Self` in the given [`App`], using the external input.
    ///
    /// # Errors
    ///
    /// - If any dependencies fail to be obtained.
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

/// The parts of the [`#[create]`](macro@crate::create) macro that are automatically generated.
pub trait CreateDeriveTypes: Sized + Send {
    /// The [`ExternalInput`](Create::ExternalInput) of the resulting [`Create`] implementation.
    type External: Send + Sync;
    /// The parts of the resulting type that aren't gathered from the [`App`].
    type Plain: Send;

    /// Finishes the rest of the value by adding in the values that can be gathered from the
    /// [`App`].
    fn finish(
        plain: Self::Plain,
        app: &mut App,
    ) -> impl Future<Output = Result<Self, Error>> + Send;
}

/// The parts of the [`#[create]`](macro@crate::create) macro that are manually implemented.
pub trait CreateDerive: CreateDeriveTypes {
    /// Create the parts of the resulting type that aren't gathered from the [`App`].
    ///
    /// # Errors
    ///
    /// - If any parts of the type can't be created.
    fn create(
        _: &Self::External,
        _: &mut App,
    ) -> impl Future<Output = Result<Self::Plain, Error>> + Send;
}

/// Values that can be cloned using an [`App`] handle.
pub trait CloneIn: Send + Sync + Sized {
    /// Creates a clone of `self` using the app handle `app`.
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
