#![allow(dead_code)]
#![feature(unboxed_closures, async_fn_traits)]
#![recursion_limit = "512"]

use thiserror::Error;

pub mod app;
pub mod children;
pub mod component;
pub mod input;
#[cfg(feature = "path-any")]
pub mod path_any;

#[cfg(test)]
mod tests;

pub use ::namespaced_id;

mod macros;
pub use macros::*;

#[cfg(feature = "path-any")]
pub use path_any::PathAny as Any;
#[cfg(not(feature = "path-any"))]
pub use std::any::Any;

#[cfg(feature = "path-any")]
pub(crate) use path_any::PathAnyConst as AnyConst;
#[cfg(not(feature = "path-any"))]
pub(crate) use std::any::Any as AnyConst;

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
pub use ::anyhow;
#[doc(hidden)]
pub use async_trait;
#[doc(hidden)]
pub use futures::try_join;
#[doc(hidden)]
pub use tokio;

pub mod prelude {
    pub use crate::component::IdentifiedType as _;
    pub use crate::{
        app::App,
        children::Children,
        component::ComponentId,
        component::bundle::Bundle,
        component::resource::{
            Runnable,
            // listener::{Listen, ListenToRegistry, Listener, OnLoad, Take},
            // watcher::Watcher,
        },
        input::{Create, CreateDerive, Input},
        // input::get::Get,
        macros::{
            GetRef, Input, Runnable, create, identified, listen_message, listener, resource,
            respond,
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

    // TODO: remove from prelude once listener and watcher are refactored
    pub use async_trait::async_trait;
}

use prelude::*;

use crate::app::map::Request;

#[derive(Error, Debug)]
pub enum Error {
    #[error("component '{id}' had an error: {error:?}")]
    FromComponent {
        id: ComponentId,
        #[source]
        error: anyhow::Error,
    },
    #[error("attempted to add '{id}' to the app, but that component already exists")]
    NameCollision { id: ComponentId },
    #[error("attempted to load '{id}' to the app, but one of its dependencies depends on it")]
    DependencyLoop { id: ComponentId },
    #[error("attempted to Get resource '{id}', but it doesn't implement GetRef")]
    ResourceNoGetRef { id: NamespacedId },
    #[error("attempted to get '{id}' from the app, but the downcast failed")]
    DowncastFailed { id: ComponentId },
    #[error("attempted to get '{id}' as a dependent, but it doesn't exist in the app")]
    AppDependentNotFound { id: ComponentId },
    #[error("attempted to add '{child}' to the app, but its parent '{parent}' was not found")]
    ParentNotFound {
        child: ComponentId,
        parent: ComponentId,
    },
    #[error(
        "attempted to get '{dependency}' with dependent '{dependent}', but that dependency was not found"
    )]
    DependencyNotFound {
        dependency: ComponentId,
        dependent: ComponentId,
    },
    #[error(
        "attempted to get '{dependency}' with dependent '{dependent}', but that dependent was not found"
    )]
    DependentNotFound {
        dependency: ComponentId,
        dependent: ComponentId,
    },
    #[error(
        "attempted to add '{component}' to the app, but its registry '{}' was not found",
        component.registry
    )]
    RegistryNotFound { component: ComponentId },
    #[error("attemped to request the app to {request}, but the request was invalid")]
    InvalidRequest { request: Request },
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
