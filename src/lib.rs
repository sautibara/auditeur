use std::ops::DerefMut;

use app::App;
use resource::NamespacedId;
use thiserror::Error;

pub mod app;
pub mod input;
pub mod listener;
pub mod resource;
pub mod watcher;

pub mod prelude {
    pub use crate::{
        app::constraints::Constraints,
        listener::Listener,
        resource::{NamespacedId, NamespacedIdRef, Resource},
        watcher::Watcher,
        Error as AuditeurError, Exit, Exiter, Plugin,
    };

    pub(crate) use crate::Error;
    pub(crate) use crate::{app::Registry, input::Input, Message};

    pub use async_trait::async_trait;
    pub use std::ops::ControlFlow;
    pub use tokio::sync::broadcast::Sender;

    pub use auditeur_macros::{ident, resource};
}

pub trait Message: Clone + Send + 'static {}

#[macro_export]
macro_rules! bake_message {
    ($message:ty) => {
        pub type App = $crate::app::App<$message>;
        pub type Registry = $crate::app::Registry<$message>;
        pub type Get<T> = $crate::input::Get<$message, T>;
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exit;

#[derive(Clone, Debug)]
pub struct Exiter {
    exiter: tokio::sync::broadcast::Sender<Exit>,
}

impl Exiter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            exiter: tokio::sync::broadcast::Sender::new(1),
        }
    }

    pub fn exit(&self) {
        let _ = self.exiter.send(Exit);
    }

    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Exit> {
        self.exiter.subscribe()
    }
}

impl Default for Exiter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("dependency '{id}' not found, has it been loaded?")]
    NotFound { id: NamespacedId },
}

pub trait Plugin<M: Message> {
    fn apply(&mut self, app: &mut App<M>);
}

impl<M: Message> Plugin<M> for Box<dyn Plugin<M>> {
    fn apply(&mut self, app: &mut App<M>) {
        self.deref_mut().apply(app);
    }
}
