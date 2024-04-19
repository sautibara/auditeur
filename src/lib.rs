pub mod app;
pub mod listener;
pub mod resource;

pub mod prelude {
    pub use crate::{
        app::constraints::Constraints,
        listener::Listener,
        resource::{NamespacedId, NamespacedIdRef, Resource},
    };

    pub(crate) use crate::listener::input::Error as InputError;
    pub(crate) use crate::{app::App, listener, Message};

    pub use async_trait::async_trait;
    pub use std::ops::ControlFlow;
    pub use tokio::sync::broadcast::Sender;

    pub use auditeur_macros::{ident, resource};
}

pub trait Message: Clone + Send + 'static {
    fn startup() -> Self;
    fn exit() -> Self;

    /// Returns `true` if `self` matches `Self::exit()`
    fn is_exit(&self) -> bool;
}

#[macro_export]
macro_rules! bake_message {
    ($message:ty) => {
        pub type App = $crate::app::App<$message>;
        pub type Get<T> = $crate::listener::input::Get<$message, T>;
    };
}
