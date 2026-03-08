use auditeur_macros::cfg_supertrait;
use thiserror::Error;

pub mod app;
pub mod bundle;
pub mod input;
pub mod namespaced_id;
#[cfg(feature = "path-any")]
pub mod path_any;
pub mod resource;

mod macros;
pub use macros::*;

#[cfg(feature = "path-any")]
pub use path_any::PathAny as Any;
#[cfg(not(feature = "path-any"))]
pub use std::any::Any;

#[cfg(feature = "path-any")]
pub mod any {
    pub use super::Any;

    /// Attempt to downcast the box to a concrete type.
    ///
    /// # Errors
    ///
    /// - Returns the original `object` if the downcast failed
    pub fn downcast<T: Any>(object: Box<dyn Any>) -> Result<Box<T>, Box<dyn Any>> {
        super::path_any::downcast(object)
    }
}

#[cfg(not(feature = "path-any"))]
pub mod any {
    pub use std::any::Any;

    /// Attempt to downcast the box to a concrete type.
    ///
    /// # Errors
    ///
    /// - Returns the original `object` if the downcast failed
    pub fn downcast<T: Any>(object: Box<dyn Any>) -> Result<Box<T>, Box<dyn Any>> {
        Box::<dyn std::any::Any + 'static>::downcast(object)
    }
}

#[doc(hidden)]
pub use anyhow;
#[doc(hidden)]
pub use async_trait;
#[doc(hidden)]
pub use futures::try_join;
#[doc(hidden)]
pub use tokio;

pub mod prelude {
    pub use crate::resource::Identifiable as _;
    pub use crate::{
        app::{
            App, ComponentId, ComponentIdRef, Registry,
            channels::{Channels, ChannelsRef, Shouter, Stopper},
            event::RegistryEvent,
        },
        bundle::Bundle,
        input::{Constraints, get::Get},
        macros::{
            GetRef, Input, Runnable, create, ident, identifiable, listener, message, resource,
            respond,
        },
        namespaced_id::{NamespacedId, NamespacedIdRef},
        // this is a #[macro_export] macro so it can't be in macros
        new_bundle,
        resource::{
            Create, CreateDerive, Runnable,
            listener::{Listen, ListenToRegistry, Listener, OnLoad, Take},
            watcher::Watcher,
        },
    };

    #[cfg(feature = "path-any")]
    pub use crate::macros::PathAny;
    pub(crate) use crate::{
        Error, ListenMessage, app::channels::EventSender, input::Input, resource::Identifiable,
        resource::Resource,
    };

    pub use crate::resource::Identifiable as _;

    pub(crate) use tracing::Instrument as _;
    pub use tracing::{
        error, error_span, info, info_span, instrument, trace, trace_span, warn, warn_span,
    };

    pub use anyhow;
    pub use anyhow::Result;
    pub use std::ops::ControlFlow;

    // TODO: remove from prelude once listener and watcher are refactored
    pub use async_trait::async_trait;
}

#[cfg_supertrait(feature = "path-any", crate::path_any::PathAnyConst)]
pub trait Message: Identifiable + Send + Sync + 'static {}

pub trait ListenMessage: Message + Clone {}

impl<M: Message + Clone> ListenMessage for M {}

#[macro_export]
macro_rules! bake_message {
    ($message:ty) => {
        pub type App = $crate::app::App<$message>;
        pub type Registry = $crate::app::Registry<$message>;
        pub type Get<T> = $crate::input::get::Get<T>;
        pub type Bundle = $crate::bundle::Bundle<$message>;
    };
}

use prelude::*;

#[derive(Error, Debug)]
pub enum Error {
    #[error("loader dependency '{id}' not found, has it been loaded?")]
    LoaderNotFound { id: NamespacedId },
    #[error("bundle dependency '{id}' not found, has it been loaded?")]
    BundleNotFound { id: NamespacedId },
    #[error("downcast of resource '{id}' failed, has it been loaded?")]
    DowncastFailed { id: NamespacedId },
    #[error("resource '{id}' not found, has it been loaded?")]
    ResourceNotFound { id: NamespacedId },
    #[error("attempted to Get a resource, but it doesn't implement GetRef")]
    ResourceNoGetRef,
    #[error("resource '{id}' had an error: {error:?}")]
    FromResource {
        id: NamespacedId,
        #[source]
        error: anyhow::Error,
    },
    #[error("could not find `Listen` message with id '{id}', has it been added to a bundle?")]
    ListenMessageNotFound { id: NamespacedId },
    #[error("could not find `Take` message with id '{id}', has it been added to a bundle?")]
    TakeMessageNotFound { id: NamespacedId },
    #[error("could not find message with id '{id}', has it been added to a bundle?")]
    MessageNotFound { id: NamespacedId },
    #[error("tried to take ownership of `Taker` with id '{id}', but it had already been taken")]
    TakeTakerTaken { id: NamespacedId },
}

/// A utility enum for when you have two different values that you may want to return.
///
/// This is best used for when the return type is opaque, such as `impl Iterator`.
enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R, I> Iterator for Either<L, R>
where
    L: Iterator<Item = I>,
    R: Iterator<Item = I>,
{
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(left) => left.next(),
            Self::Right(right) => right.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Left(left) => left.size_hint(),
            Self::Right(right) => right.size_hint(),
        }
    }
}
