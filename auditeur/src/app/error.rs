//! A shared list of errors held in the [`App`].
//!
//! If errors occur that do not directly get returned to the caller, they'll automatically go to
//! the error list. This includes, but is not limited to, errors occurred during
//! [adding](super::AppComponentsMut::add),
//! [lazily loading](crate::component::registry::Loader::on_load),
//! and [unloading](crate::component::registry::Loaded::unload) components, errors that
//! occur in [resources](crate::component::resource), and calls to [`AppErrorsMut::add`].
//!
//! # Examples
//!
//! ```
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), auditeur::Error> {
//! // [`AppComponentsMut::add`] attempts to add as many components as possible,
//! // so it doesn't early-return if an error occurs, and instead sends it to the error list.
//!
//! #[resource("test:resource")]
//! #[pure_resource]
//! #[create(default)]
//! struct Resource;
//!
//! // Consider an app with a single resource.
//! let mut app = App::builder().with::<Resource>(()).spawn();
//!
//! // It shouldn't have any errors yet - this is all normal for now.
//! assert!(app.errors().get().await.is_empty());
//!
//! // But if we try to add the resource again,
//! app.components_mut()
//!     .add(Children::new().with::<Resource>(()))
//!     .await;
//! // an error should occur due to a name collision.
//! let errors = app.errors().get().await;
//! assert!((errors.iter()).any(|error| matches!(error, auditeur::Error::NameCollision { .. })));
//!
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::sync::Arc;

use thiserror::Error;

use crate::{
    app::{
        AppErrors, AppErrorsMut, AppFn, AppFnErrors, AppFnErrorsMut, AppFnOnce, AppFuture,
        AppMutFnOnce, AppMutFuture, HandleData, OneshotSender, Owned, event::Event,
        map::RequestAction,
    },
    prelude::*,
};

/// A static view into a shared list of errors held in the [`App`].
///
/// All errors that occur in the app bubble up into a shared append-only error list. These values
/// are immutable snapshots of that list, containing all errors that occurred before the snapshot
/// was taken. To take a new snapshot, call [`AppErrors::get`].
#[derive(Default, Clone, Debug)]
pub struct Errors {
    errors: imbl::Vector<Arc<Error>>,
}

impl Errors {
    pub fn iter(&self) -> impl Iterator<Item = &Error> {
        self.errors.iter().map(|error| &**error)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Owned {
    /// Add an error to the shared error list and notify listeners.
    pub(super) fn add_error(&mut self, error: Error) {
        self.add_shared_error(Arc::new(error));
    }

    /// Add an arc error to the shared error list and notify listeners.
    pub(super) fn add_shared_error(&mut self, error: Arc<Error>) {
        error!("{error}");
        self.errors.errors.push_back(error.clone());
        let _ = self.event_sender.send(Event::Error(error));
    }

    /// Returns [`Some`] if `result` is [`Ok`], or if there is an error, returns [`None`] and sends
    /// that error to the shared error list.
    pub(super) fn drain_result<T>(&mut self, result: Result<T, Error>) -> Option<T> {
        result.map_err(|err| self.add_error(err)).ok()
    }
}

impl HandleData {
    /// Wraps an [`anyhow::Error`] into a [`crate::Error`] using the current
    /// [dependent](Dependent) as the source component.
    fn wrap_error(&self, error: anyhow::Error) -> Error {
        Error::FromComponent {
            id: self.dependent_id().into_id(),
            error,
        }
    }
}

/// A message from a handle to run a method on the global [`App`].
#[derive(Debug)]
pub(super) enum Method {
    AddShared(Arc<Error>),
    Get(OneshotSender<Errors>),
}

impl Method {
    pub async fn run(self, app: &mut App) {
        match self {
            Self::AddShared(error) => app.errors_mut().add_shared(error).await,
            Self::Get(sender) => {
                let errors = app.errors().get().await;
                let () = sender.send(errors);
            }
        }
    }
}

impl AppFnErrors {
    /// See [`AppErrors::get`].
    pub const fn get(self) -> impl AppFnOnce<Output = Errors> {
        AppFn::oneshot(
            (),
            |(), sender, _| super::Method::Errors(Method::Get(sender)),
            |(), owned, _| owned.errors.clone(),
        )
    }
}

impl<'a> AppErrors<'a> {
    pub(super) const fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Wraps an [`anyhow::Error`] into a [`crate::Error`] using the current
    /// [dependent](super#dependents) as the source component.
    #[must_use]
    pub fn wrap(self, error: impl Into<anyhow::Error>) -> Error {
        self.app.handle.wrap_error(error.into())
    }

    /// Gets an immutable snapshot into the shared error list.
    #[allow(clippy::missing_panics_doc)]
    pub fn get(self) -> impl AppFuture<'a, Output = Errors> {
        AppFn.errors().get().call(self.app)
    }
}

impl AppFnErrorsMut {
    #[expect(clippy::unused_self)]
    const fn add_shared(self, error: Arc<Error>) -> impl AppMutFnOnce<Output = ()> {
        AppFn::spawn_mut(
            error,
            |error, _| super::Method::Errors(Method::AddShared(error)),
            |error, owned, _| owned.add_shared_error(error),
        )
    }

    #[expect(clippy::should_implement_trait)]
    pub fn add(self, error: Error) -> impl AppMutFnOnce<Output = ()> {
        self.add_shared(Arc::new(error))
    }

    pub fn add_wrapped(self, error: impl Into<anyhow::Error>) -> impl AppMutFnOnce<Output = ()> {
        let error = error.into();
        AppFn::using_handle_mut(|handle| {
            let error = handle.wrap_error(error);
            AppFn.errors_mut().add(error)
        })
    }
}

impl<'a> AppErrorsMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Adds a new error to the shared error list.
    ///
    /// This also sends an [`Event::Error`] to all event listeners.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, error: Error) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.errors_mut().add(error).call(self.app)
    }

    /// Adds a new error to the shared error list, wrapping it like [`AppErrors::wrap`].
    pub fn add_wrapped(
        self,
        error: impl Into<anyhow::Error>,
    ) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.errors_mut().add_wrapped(error).call(self.app)
    }

    fn add_shared(self, error: Arc<Error>) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.errors_mut().add_shared(error).call(self.app)
    }

    /// Returns [`Some`] if `result` is [`Ok`], or if there is an error, returns [`None`] and sends
    /// that error to the shared error list.
    pub async fn drain_result<T>(self, result: Result<T, Error>) -> Option<T> {
        match result {
            Ok(val) => Some(val),
            Err(err) => {
                self.add(err).await;
                None
            }
        }
    }

    /// Returns [`Some`] if `result` is [`Ok`], or if there is an error, returns [`None`] and sends
    /// that error to the shared error list, wrapped like [`AppErrors::wrap`].
    pub async fn drain_any_result<T>(self, result: Result<T, anyhow::Error>) -> Option<T> {
        match result {
            Ok(val) => Some(val),
            Err(err) => {
                let err = self.app.errors().wrap(err);
                self.add(err).await;
                None
            }
        }
    }
}

/// An error that occurred in the [`App`].
#[derive(Error, Debug)]
pub enum Error {
    /// An [`anyhow::Error`] was produced in the component with the id `id`.
    ///
    /// This primarily comes from calls to [`AppErrors::wrap`].
    #[error("component '{id}' had an error: {error:?}")]
    FromComponent {
        id: ComponentId,
        #[source]
        error: anyhow::Error,
    },
    /// A component with the id `id` was attempted to be added to the app, but a component with
    /// that id already existed.
    #[error("attempted to add '{id}' to the app, but that component already exists")]
    NameCollision { id: ComponentId },
    // TODO: actually produce this error.
    // /// A component with the id `id` was attempted to be loaded, but one of its dependencies
    // /// depends on it.
    // #[error("attempted to load '{id}' to the app, but one of its dependencies depends on it")]
    // DependencyLoop { id: ComponentId },
    /// A component with the id `id` attempted to downcast an [`Any`](crate::any::Any) value that
    /// should never fail, but it did fail.
    #[error("attempted to get '{id}' from the app, but the downcast failed")]
    DowncastFailed { id: ComponentId },
    /// A component with the id of `child` was attempted to be added to the app, but its parent
    /// component with the id `parent` was not found.
    #[error("attempted to add '{child}' to the app, but its parent '{parent}' was not found")]
    ParentNotFound {
        child: ComponentId,
        parent: ComponentId,
    },
    /// A component with the id `dependent` attempted to load the component with the id
    /// `dependency`, but that component was not found.
    #[error(
        "attempted to get '{dependency}' with dependent '{dependent}', but that dependency was not found"
    )]
    DependencyNotFound {
        dependency: ComponentId,
        dependent: ComponentId,
    },
    /// A component with the id `dependent` attempted to load the component with the id
    /// `dependency`, but the dependent component was not found.
    #[error(
        "attempted to get '{dependency}' with dependent '{dependent}', but that dependent was not found"
    )]
    DependentNotFound {
        dependency: ComponentId,
        dependent: ComponentId,
    },
    /// A component with the id `component` was attempted to be added to the app, but its
    /// registry was not found.
    #[error(
        "attempted to add '{component}' to the app, but its registry '{}' was not found",
        component.registry
    )]
    RegistryNotFound { component: ComponentId },
    /// The request `request` was made in the app, but that request didn't make sense.
    #[error("attemped to request the app to {request}, but the request was invalid")]
    InvalidRequest { request: RequestAction },
}
