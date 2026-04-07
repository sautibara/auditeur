//! Items that can be added to an [`App`] and potentially run

use crate::prelude::*;

use anyhow::Result;
use auditeur_macros::cfg_supertrait;
use futures::Future;

pub mod listener;
pub(crate) mod non_runnable;
pub mod watcher;

/// An item that can be added to an [`App`]
///
/// Resources are [`Identifiable`] by a specific [`NamespacedId`] that uniquely represents them.
/// If they are a [`Listener`] or [`Watcher`], they may also run code in the main async loop, and
/// if they implement [`GetRef`], other resources may obtain synchronized references of them.
///
/// To implement this trait, use [`#[resource]`](macro@crate::resource)
#[cfg_supertrait(feature = "runtime-loading", crate::input::get::MaybeGetRef)]
pub trait Resource: Identifiable + crate::Any + Send + Sync + 'static {}

#[cfg(feature = "runtime-loading")]
impl<T: Identifiable + crate::Any + crate::input::get::MaybeGetRef + Send + Sync + 'static> Resource
    for T
{
}
#[cfg(not(feature = "runtime-loading"))]
impl<T: Identifiable + crate::Any + Send + Sync + 'static> Resource for T {}

pub trait Runnable {
    fn init(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    fn exit(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }
}

pub trait Create: Sized {
    type RegistryInput: Input;
    type ExternalInput: Send;

    // TODO: accept a Result
    #[allow(opaque_hidden_inferred_bound)] // false positive?
    fn create(
        registry: Self::RegistryInput,
        external: Self::ExternalInput,
    ) -> impl Future<Output = Result<Self, anyhow::Error>> + Send;
}

impl<T: CreateDerive> Create for T {
    type ExternalInput = <Self as CreateDeriveTypes>::External;
    type RegistryInput = <Self as CreateDeriveTypes>::Input;

    async fn create(
        registry: Self::RegistryInput,
        external: Self::ExternalInput,
    ) -> Result<Self, anyhow::Error> {
        let plain = <Self as CreateDerive>::create(external, &registry).await?;
        Self::combine(registry, plain).await
    }
}

pub trait CreateDeriveTypes: Sized {
    type External: Send;

    type Input: Input;
    type Plain: Send;

    fn combine(
        input: Self::Input,
        plain: Self::Plain,
    ) -> impl Future<Output = anyhow::Result<Self>> + Send + Sync;
}

pub trait CreateDerive: CreateDeriveTypes {
    fn create(
        _: Self::External,
        _: &Self::Input,
    ) -> impl Future<Output = anyhow::Result<Self::Plain>> + Send;
}
