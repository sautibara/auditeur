use std::{ops::Deref, sync::Arc};


use crate::{
    app::{
        AppFn, AppFnOnce, AppFuture, AppMutFnOnce, AppMutFuture, HandleData, OneshotSender, Owned,
        event::Event,
    },
    prelude::*,
};

/// A static view into a shared list of errors held in the [`App`].
///
/// All errors that occur in the app bubble up into a shared append-only error list. These values
/// are immutable snapshots of that list, containing all errors that occurred before the snapshot
/// was taken. To take a new snapshot, call [`AppErrors::get`].
#[derive(Default, Clone)]
pub struct Errors {
    errors: im::Vector<Arc<Error>>,
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
            id: self.dependent_added_id().deref().clone(),
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

/// Return type of [`AppFn::errors`], see [`AppErrors`].
#[must_use]
pub struct AppFnErrors;
impl AppFnErrors {
    /// See [`AppErrors::get`].
    pub const fn get(self) -> impl AppFnOnce<Output = Errors> {
        AppFn::oneshot(
            (),
            |(), sender, _| super::Method::Errors(Method::Get(sender)),
            async |(), owned, _| owned.errors.clone(),
        )
    }
}

/// Return type of [`App::errors()`].
pub struct AppErrors<'a> {
    app: &'a App,
}

impl<'a> AppErrors<'a> {
    pub(super) const fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Wraps an [`anyhow::Error`] into a [`crate::Error`] using the current
    /// [dependent](super::Dependent) as the source component.
    #[must_use]
    pub fn wrap(self, error: anyhow::Error) -> Error {
        self.app.handle.wrap_error(error)
    }

    /// Gets an immutable snapshot into the shared error list.
    #[allow(clippy::missing_panics_doc)]
    pub fn get(self) -> impl AppFuture<'a, Output = Errors> {
        AppFn.errors().get().call(self.app)
    }
}

/// Return type of [`AppFn::errors_mut`], see [`AppErrorsMut`].
#[must_use]
pub struct AppFnErrorsMut;
impl AppFnErrorsMut {
    #[expect(clippy::unused_self)]
    const fn add_shared(self, error: Arc<Error>) -> impl AppMutFnOnce<Output = ()> {
        AppFn::spawn_mut(
            error,
            |error, _| super::Method::Errors(Method::AddShared(error)),
            async |error, owned, _| owned.add_shared_error(error),
        )
    }

    #[expect(clippy::should_implement_trait)]
    pub fn add(self, error: Error) -> impl AppMutFnOnce<Output = ()> {
        self.add_shared(Arc::new(error))
    }

    pub fn add_wrapped(self, error: anyhow::Error) -> impl AppMutFnOnce<Output = ()> {
        AppFn::using_handle_mut(|handle| {
            let error = handle.wrap_error(error);
            AppFn.errors_mut().add(error)
        })
    }
}

/// Return type of [`App::errors_mut()`].
pub struct AppErrorsMut<'a> {
    app: &'a mut App,
}

impl<'a> AppErrorsMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Adds a new error to the shared error list.
    ///
    /// This also sends a [`RegistryEvent::Error`] to all event listeners.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, error: Error) -> impl AppMutFuture<'a, Output = ()> {
        AppFn.errors_mut().add(error).call(self.app)
    }

    /// Adds a new error to the shared error list, wrapping it like [`AppErrors::wrap`].
    pub fn add_wrapped(self, error: anyhow::Error) -> impl AppMutFuture<'a, Output = ()> {
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
