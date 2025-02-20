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
    pub use crate::{
        app::{App, Constraints, Registry, RegistryEvent, Stopper},
        bundle::Bundle,
        create, has_input, ident, identifiable,
        input::get::Get,
        listener, listener_component, message,
        namespaced_id::{NamespacedId, NamespacedIdRef},
        new_bundle, resource,
        resource::{Create, Identifiable, Listener, Resource, Watcher},
        Error as AuditeurError, GetRef,
    };

    pub use auditeur_macros::Create;

    #[cfg(feature = "path-any")]
    pub use crate::path_any::PathAny;
    #[allow(unused_imports)] // false positive?
    pub(crate) use crate::Any;
    #[cfg(feature = "path-any")]
    pub use auditeur_macros::PathAny;

    pub(crate) use crate::Error;
    pub(crate) use crate::{input::Input, Message};

    pub use anyhow;
    pub use anyhow::Result;
    pub use async_trait::async_trait;
    pub use std::ops::ControlFlow;

    pub(crate) use tokio::sync::broadcast::Sender;
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
