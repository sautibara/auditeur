//! Events that occur within an [`App`].
//!
//! # Examples
//!
//! ```
//! use auditeur::app::event::ComponentLifecycle;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), auditeur::Error> {
//! #[resource("test:resource")]
//! #[pure_resource]
//! #[create(default)]
//! struct Resource;
//!
//! let mut app = App::builder().spawn();
//!
//! // With an event listener, we could listen to events that happen in the app.
//! let mut events = app.get_event_listener().await?;
//!
//! // For example, if we add a component to the app,
//! app.components_mut()
//!     .add(Children::new().with::<Resource>(()))
//!     .await;
//!
//! // we should be able to see that in the event list.
//! loop {
//!     if let auditeur::app::Event::Component { id, lifecycle } = events.listen().await
//!         && id == Resource::component_id()
//!         && lifecycle == ComponentLifecycle::Add
//!     {
//!         break;
//!     }
//! }
//!
//! // Note that [`Event`]s are backed by [`listen`]. This means that event listeners can only
//! // see events that happen after they're obtained.
//!
//! drop(events);
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::{fmt::Debug, sync::Arc};

use tokio::sync::broadcast;

use crate::{
    app::map::{ListenerId, Request},
    component::HeapComponentId,
    dependency::DependencyId,
    prelude::*,
};

/// An event that occurs within the [`App`].
///
/// This is added as a [`ListenMessage`](crate::component::listen::ListenMessage) to the app, so a
/// listener can be obtained through
/// [`AppListenMut::listener`](crate::component::listen::AppListenMut::listener).
/// Additionally, if an [`AppFuture`](super::AppFuture) is needed, it can be obtained through
/// [`App::get_event_listener`].
#[derive(Clone, Debug)]
#[listen_message("auditeur:registry_event")]
#[non_exhaustive]
pub enum Event {
    /// An error occurred in the app.
    ///
    /// This error will also be added to the [shared error list](super::error::Errors).
    Error(Arc<Error>),
    /// A component was added, loaded, unloaded, or removed.
    Component {
        id: ComponentId,
        lifecycle: ComponentLifecycle,
    },
    /// A dependency was added or removed.
    Dependency {
        id: DependencyId,
        lifecycle: DependencyLifecycle,
    },
    /// The app is requesting for a [removable](crate::dependency::DependencyType::Removable)
    /// dependency to be removed.
    RequestDependencyRemoval { id: DependencyId },
    /// A new listener is blocking a request.
    ///
    /// This event will be sent for every descendant blocking listener of a request. For example,
    /// if the removal of a loaded component is requested, an event will be sent each for its
    /// removal, unloading, and deactivation (and anything blocking any of those).
    Requested {
        listener: ListenerId,
        request: Request,
    },
    /// A request has been fulfilled.
    RequestSuccess { request: Request },
    /// The app has exited.
    Exit,
    /// The app is waiting for all event listeners to finish using an id.
    #[doc(hidden)]
    Flush(FlushRequest),
    /// The app has fully finished using the id `id`, and it can be unloaded.
    Flushed { id: HeapComponentId },
}

/// The state of a component in the [`App`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentLifecycle {
    /// The component was added to the app, but it hasn't been loaded yet.
    Add,
    /// The component was loaded in the app, but it hasn't been loaded in its registry yet.
    Load,
    /// The component was loaded in its registry;
    /// [`Loader::load`](crate::component::registry::Loader::load) was called.
    Active,
    /// The component was unloaded in its registry, but it hasn't released its dependencies yet.
    Unloading,
    /// The component was unloaded in the app, due to all of its dependents and dependencies
    /// unloading.
    Unload,
    /// The component was removed from the app, forever unable to be loaded again.
    Remove,
}

/// The state of a dependency in the [`App`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyLifecycle {
    Add,
    Remove,
}

#[doc(hidden)]
#[derive(Clone)]
#[expect(dead_code)]
pub struct FlushRequest(Arc<FlushRequestInner>);

impl Debug for FlushRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlushRequest").finish()
    }
}

impl FlushRequest {
    pub(crate) fn new(id: HeapComponentId, event_sender: broadcast::Sender<Event>) -> Self {
        Self(Arc::new(FlushRequestInner { id, event_sender }))
    }
}

struct FlushRequestInner {
    id: HeapComponentId,
    event_sender: broadcast::Sender<Event>,
}

impl Drop for FlushRequestInner {
    fn drop(&mut self) {
        let _ = self.event_sender.send(Event::Flushed {
            id: self.id.clone(),
        });
    }
}
