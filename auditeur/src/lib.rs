#![feature(unboxed_closures, async_fn_traits)]
#![recursion_limit = "512"]

pub mod app;
pub mod children;
pub mod component;
pub mod input;
#[cfg(feature = "path-any")]
pub mod path_any;

pub mod test;

pub use ::namespaced_id;

mod macros;
pub use macros::*;

pub mod any {
    #[cfg(feature = "path-any")]
    pub use crate::path_any::PathAny as Any;
    #[cfg(not(feature = "path-any"))]
    pub use std::any::Any;

    #[cfg(feature = "path-any")]
    pub use crate::path_any::PathAnyConst as AnyConst;
    #[cfg(not(feature = "path-any"))]
    pub use std::any::Any as AnyConst;

    /// Attempt to downcast the box to a concrete type.
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

pub mod prelude {
    pub use crate::children::HasChildren as _;
    pub use crate::component::IdentifiedComponentType as _;
    pub use crate::component::IdentifiedType as _;
    pub use crate::{
        app::{App, map::Dependency},
        children::Children,
        component::ComponentId,
        component::bundle::Bundle,
        component::listen::Shouter,
        component::resource::{
            listener::{Listen, Listener, ListenerDerive, OnAddIn, Take},
            watcher::Watcher,
        },
        component::take::Giver,
        input::get::Get,
        input::{Create, CreateDerive, Input},
        macros::{
            CloneIn, GetRef, Input, create, identified, listen_message, listener, pure_resource,
            resource, respond, take_message,
        },
        namespaced_id::{self, NamespacedId, NamespacedIdRef, ident},
        // this is a #[macro_export] macro so it can't be in macros
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
