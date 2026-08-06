#![feature(unboxed_closures, async_fn_traits)]
#![recursion_limit = "512"]

pub mod app;
pub mod component;
pub mod input;
#[cfg(any(doc, feature = "path-any"))]
pub mod path_any;

pub mod test;

pub use namespaced_id;

mod macros;
pub use macros::*;

/// The form of [`Any`] that follows the current value of the `path-any` feature.
pub mod any {
    #[cfg(feature = "path-any")]
    pub use crate::path_any::PathAny as Any;
    #[cfg(not(feature = "path-any"))]
    pub use std::any::Any;

    #[cfg(feature = "path-any")]
    pub use crate::path_any::PathAnyConst as AnyConst;
    #[cfg(not(feature = "path-any"))]
    pub use std::any::Any as AnyConst;

    /// Attempts to downcast a box to a concrete type.
    ///
    /// # Errors
    ///
    /// - Returns the original `object` if the downcast failed
    pub fn downcast<T: Any>(object: Box<dyn Any>) -> Result<Box<T>, Box<dyn Any>> {
        #[cfg(feature = "path-any")]
        let res = super::path_any::downcast(object);
        #[cfg(not(feature = "path-any"))]
        let res = Box::<dyn Any>::downcast(object);
        res
    }
}

#[doc(hidden)]
pub use ::anyhow;
#[doc(hidden)]
pub use async_trait;
#[doc(hidden)]
pub use futures::try_join;
#[doc(hidden)]
pub use tokio;

/// Items that are commonly imported in auditeur-using code.
pub mod prelude {
    pub use crate::app::AppFnOnce as _;
    pub use crate::app::AppFuture as _;
    pub use crate::app::AppMutFnOnce as _;
    pub use crate::app::AppMutFuture as _;
    pub use crate::component::HasChildren as _;
    pub use crate::component::IdentifiedComponentType as _;
    pub use crate::component::IdentifiedType as _;
    pub use crate::{
        app::{App, AppFn, map::Dependency},
        component::{
            Children, ComponentId,
            bundle::Bundle,
            listen::Shouter,
            resource::{
                get::Get,
                listener::{Listen, Listener, ListenerDerive, OnAddIn, Take},
                watcher::Watcher,
            },
            take::Giver,
        },
        input::{Create, CreateDerive, Input},
        macros::{
            CloneIn, Input, create, identified, listen_message, listener, pure_resource, resource,
            respond, take_message,
        },
        namespaced_id::{self, NamespacedId, NamespacedIdRef, ident},
        new_bundle,
    };

    pub use crate::macros::Any;

    pub(crate) use crate::{Error, component::Identified};

    pub use tracing::{
        error, error_span, info, info_span, instrument, trace, trace_span, warn, warn_span,
    };

    pub use anyhow;
    pub use anyhow::Result;
    pub use std::ops::ControlFlow;
}

pub use app::App;
pub use app::error::Error;
