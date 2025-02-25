//! Items that can be added to an [`App`] and potentially run

use crate::prelude::*;

use anyhow::Result;
use auditeur_macros::cfg_supertrait;
use futures::Future;

pub mod builtin;
pub mod listener;

/// A type that has an associated [`NamespacedId`] that uniquely represents it
pub trait Identifiable {
    fn ident(&self) -> &'static NamespacedIdRef;
    fn const_ident() -> &'static NamespacedIdRef
    where
        Self: Sized;
}

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
    type InitInput<'s>: Send;
    type ExitInput<'s>: Send;

    fn init(
        &mut self,
        input: Self::InitInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn exit(
        &mut self,
        input: Self::ExitInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// A [`Resource`] that runs its own method in the main async loop
///
/// The main use of `Watcher`s is that they can ask for [`Input`]s from the rest of the [`App`]
#[async_trait]
pub trait Watcher: Resource + Send {
    type Input: Input;

    async fn run(&mut self, input: &mut Self::Input) -> anyhow::Result<()>;

    async fn exit(&mut self, input: &mut Self::Input) -> anyhow::Result<()> {
        let _ = input;
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
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

// #[resource("rpc:client")]
// #[listener(on(message::Audio))]
// struct Client {
//     ...
// }
//
// impl Runnable for Client {
//     #[resource_method]
//     async fn init(&mut self) -> Result<()> {}
//
//     #[resource_method]
//     async fn exit(&mut self) -> Result<()> {}
// }
//
// impl Listener for Client {
//     type Components = (ListenComponent<Client, message::Audio>);
// }
//
// impl Listen<message::Audio> for Client {
//     #[resource_method]
//     async fn on(&mut self, message: message::Audio, library: &mut Get<Library>) {}
// }
