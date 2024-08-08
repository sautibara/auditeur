use auditeur_macros::cfg_supertrait;
use thiserror::Error;

pub mod app;
pub mod bundle;
pub mod input;
pub mod namespaced_id;
pub mod resource;

#[cfg(feature = "path-any")]
pub mod path_any;
/// A macro that automatically provides an implementation of [`PathAny`](crate::path_any::PathAny)
/// by implementing [`PathAnyConst`](crate::path_any::PathAnyConst)
#[cfg(feature = "path-any")]
pub use auditeur_macros::PathAny;

#[cfg(feature = "resource-map")]
pub mod resource_map;

pub use auditeur_macros::{ident, resource};

/// A macro to automate implementing [`GetRef`](crate::input::GetRef) using [`Clone`]
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::input;
/// # pub fn main() {
/// use auditeur::GetRef;
/// use auditeur::input::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let reference: Shared = shared.get_ref();
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
pub use auditeur_macros::GetRef;

pub mod prelude {
    pub use crate::{
        app::{Constraints, RegistryEvent, Stopper},
        bundle::Bundle,
        ident,
        namespaced_id::{NamespacedId, NamespacedIdRef},
        new_bundle, resource,
        resource::{DynamicResource, Listener, Resource, Watcher},
        Error as AuditeurError, GetRef,
    };

    #[cfg(feature = "path-any")]
    pub use crate::PathAny;

    pub(crate) use crate::Error;
    pub(crate) use crate::{
        app::{App, Registry},
        input::Input,
        Message,
    };

    pub use anyhow;
    pub use anyhow::Result;
    pub use async_trait::async_trait;
    pub use std::ops::ControlFlow;

    pub(crate) use tokio::sync::broadcast::Sender;
}

#[cfg_supertrait(feature = "path-any", path_any::PathAnyConst)]
pub trait Message: Clone + Send + 'static {}

#[macro_export]
macro_rules! bake_message {
    ($message:ty) => {
        pub type App = $crate::app::App<$message>;
        pub type Registry = $crate::app::Registry<$message>;
        pub type Get<T> = $crate::input::Get<$message, T>;
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
}
