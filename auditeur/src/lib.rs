use app::App;
use resource::NamespacedId;
use thiserror::Error;

pub mod app;
pub mod input;
pub mod resource;

pub use auditeur_macros::{ident, resource};

pub mod prelude {
    pub use crate::{
        app::{constraints::Constraints, Exiter, Stopper},
        ident, resource,
        resource::{Listener, NamespacedId, NamespacedIdRef, Resource, Watcher},
        Error as AuditeurError, Plugin,
    };

    pub(crate) use crate::Error;
    pub(crate) use crate::{app::Registry, input::Input, Message};

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
    };
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("dependency '{id}' not found, has it been loaded?")]
    NotFound { id: NamespacedId },
}

pub trait Plugin<M: Message> {
    fn apply(&self, app: &mut App<M>);
}
