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
pub use futures::try_join;

pub mod prelude {
    pub use crate::resource::Identifiable as _;
    pub use crate::{
        app::{App, Constraints, Registry, RegistryEvent, Stopper},
        bundle::Bundle,
        input::get::Get,
        macros::{
            component, create, has_input, ident, identifiable, listener, message, resource, Create,
            GetRef, PathAny,
        },
        namespaced_id::{NamespacedId, NamespacedIdRef},
        // this is a #[macro_export] macro so it can't be in macros
        new_bundle,
        // TODO: remove Listener from the prelude
        resource::{Create, Listener, Watcher},
    };

    #[cfg(feature = "path-any")]
    pub(crate) use crate::path_any::PathAny;
    pub(crate) use crate::{
        input::Input, resource::Identifiable, resource::Resource, Any, Error, Message,
    };
    pub(crate) use tokio::sync::broadcast::Sender;

    pub use anyhow;
    pub use anyhow::Result;
    pub use std::ops::ControlFlow;

    // TODO: remove from prelude once listener and watcher are refactored
    pub use async_trait::async_trait;
}

#[cfg_supertrait(feature = "path-any", path_any::PathAnyConst)]
pub trait Message: Identifiable + Clone + Send + Sync + 'static {}

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
    #[error("resource '{id}' had an error: {error:?}")]
    FromResource {
        id: NamespacedId,
        #[source]
        error: anyhow::Error,
    },
}
