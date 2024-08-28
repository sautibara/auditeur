//! Items that can be added to an [`App`] and potentially run

use crate::{app::RegistryEvent, prelude::*};

use anyhow::Result;
use auditeur_macros::cfg_supertrait;
use futures::Future;

pub mod builtin;

/// A type that has an associated [`NamespacedId`] that uniquely represents it
pub trait Identifiable: Send + Sync + 'static {
    fn ident(&self) -> NamespacedIdRef<'static>;
    fn const_ident() -> NamespacedIdRef<'static>
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
#[cfg_supertrait(feature = "runtime-loading", crate::input::MaybeGetRef)]
pub trait Resource: Identifiable + crate::Any {}

#[cfg(feature = "runtime-loading")]
impl<T: Identifiable + crate::Any + crate::input::MaybeGetRef> Resource for T {}
#[cfg(not(feature = "runtime-loading"))]
impl<T: Identifiable + crate::Any> Resource for T {}

/// A [`Resource`] that listens to the main [`Message`] channel
///
/// [`Input`]s are the way that [`Listener`]s are able to communicate with the rest of the [`App`]
#[async_trait]
pub trait Listener<M: Message>: Resource + Send {
    type Input: Input<M>;

    async fn handle(&mut self, message: M, input: &mut Self::Input) -> Result<()>;

    async fn init(&mut self, input: &mut Self::Input) -> Result<()> {
        let _ = input;
        Ok(())
    }

    async fn exit(&mut self, input: &mut Self::Input) -> Result<()> {
        let _ = input;
        Ok(())
    }

    async fn registry_event(
        &mut self,
        event: RegistryEvent,
        input: &mut Self::Input,
    ) -> Result<()> {
        let _ = (event, input);
        Ok(())
    }
}

/// A [`Resource`] that runs its own method in the main async loop
///
/// The main use of `Watcher`s is that they can ask for [`Input`]s from the rest of the [`App`]
#[async_trait]
pub trait Watcher<M: Message>: Resource + Send {
    type Input: Input<M>;

    async fn run(&mut self, input: &mut Self::Input) -> anyhow::Result<()>;

    async fn exit(&mut self, input: &mut Self::Input) -> anyhow::Result<()> {
        let _ = input;
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}

pub trait Create<M: Message> {
    type RegistryInput: Input<M>;
    type ExternalInput: Send;

    #[allow(opaque_hidden_inferred_bound)] // false positive?
    fn create(
        registry: Self::RegistryInput,
        external: Self::ExternalInput,
    ) -> impl Future<Output = Self> + Send;
}
