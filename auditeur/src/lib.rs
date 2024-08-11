use auditeur_macros::cfg_supertrait;
use thiserror::Error;

pub mod app;
pub mod bundle;
pub mod input;
pub mod namespaced_id;
#[cfg(feature = "path-any")]
pub mod path_any;
pub mod resource;

/// A macro that returns a parsed [`NamespacedId`]
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::namespaced_id;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// assert_eq!(
///     ident!("namespace:id"),
///     NamespacedIdRef {
///         namespace: "namespace",
///         id: "id",
///     },
/// );
/// # }
/// ```
pub use auditeur_macros::ident;

/// A macro to implement [`Resource`] for a type
///
/// This automatically implements [`Identifiable`], [`Any`], and if necessary, [`MaybeGetRef`]
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::{namespaced_id, resource, path_any};
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[resource("test:useful_resource")]
/// struct UsefulResource;
///
/// assert_eq!(UsefulResource.ident(), ident!("test:useful_resource"));
/// # }
/// ```
pub use auditeur_macros::resource;

pub use auditeur_macros::create;
pub use auditeur_macros::Create;

/// A macro to automate implementing [`GetRef`](crate::input::GetRef) using [`Clone`]
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::{input, path_any};
/// # pub fn main() {
/// use auditeur::prelude::*;
/// use auditeur::input::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef, PathAny)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let reference: Shared = shared.get_ref();
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
pub use auditeur_macros::GetRef;

/// A macro that automatically provides an implementation of
/// [`MaybeGetRef`](crate::input::MaybeGetRef)
///
/// See [`MaybeGetRef`](crate::input::MaybeGetRef) for more information on why this is necessary
#[cfg(feature = "runtime-loading")]
pub use auditeur_macros::MaybeGetRef;

/// A macro that automatically provides an implementation of [`PathAny`]
/// by implementing [`PathAnyConst`](crate::path_any::PathAnyConst)
#[cfg(feature = "path-any")]
pub use auditeur_macros::PathAny;

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

pub mod prelude {
    pub use crate::{
        app::{Constraints, RegistryEvent, Stopper},
        bundle::Bundle,
        create, ident,
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
pub trait Message: Clone + Send + Sync + 'static {}

#[macro_export]
macro_rules! bake_message {
    ($message:ty) => {
        pub type App = $crate::app::App<$message>;
        pub type Registry = $crate::app::Registry<$message>;
        pub type Get<T> = $crate::input::Get<T>;
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
