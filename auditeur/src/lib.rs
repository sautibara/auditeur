#![allow(dead_code)]
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
            listener::{Listen, Listener, ListenerDerive, Take},
            watcher::Watcher,
        },
        input::get::Get,
        input::{Create, CreateDerive, Input},
        macros::{
            GetRef, Input, create, identified, listen_message, listener, pure_resource, resource,
            respond, take_message,
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
