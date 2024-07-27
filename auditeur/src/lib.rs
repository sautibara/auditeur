use thiserror::Error;

pub mod app;
pub mod bundle;
pub mod input;
pub mod namespaced_id;
pub mod resource;

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
        resource::{Listener, Resource, Watcher},
        Error as AuditeurError, GetRef,
    };

    pub(crate) use crate::Error;
    pub(crate) use crate::{
        app::{App, Registry},
        input::Input,
        Message,
    };

    pub use anyhow::Result;
    pub use async_trait::async_trait;
    pub use std::ops::ControlFlow;

    pub(crate) use tokio::sync::broadcast::Sender;
}

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
}
