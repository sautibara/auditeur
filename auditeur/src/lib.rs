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

#[cfg(all(feature = "runtime-loading", feature = "path-any"))]
pub(crate) fn downcast<T: Any>(object: Box<dyn Any>) -> Result<Box<T>, Box<dyn Any>> {
    path_any::downcast(object)
}
#[cfg(all(feature = "runtime-loading", not(feature = "path-any")))]
pub(crate) fn downcast<T: Any>(object: Box<dyn Any>) -> Result<Box<T>, Box<dyn Any>> {
    Box::<dyn std::any::Any + 'static>::downcast(object)
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
            App, Constraints, Registry,
            channels::{Channels, ChannelsRef, Shouter, Stopper},
            event::{ComponentId, ComponentIdRef, RegistryEvent},
        },
        bundle::Bundle,
        input::get::Get,
        macros::{
            Create, GetRef, Input, Runnable, component, create, has_input, ident, identifiable,
            listener, message, resource, respond,
        },
        namespaced_id::{NamespacedId, NamespacedIdRef},
        // this is a #[macro_export] macro so it can't be in macros
        new_bundle,
        resource::{
            Create, Runnable,
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
    pub(crate) use tracing::{error, instrument, trace, warn};

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
