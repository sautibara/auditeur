use std::sync::Arc;

use crate::{
    app::{
        error::Errors,
        map::{DependencyId, Map},
    },
    prelude::*,
};

#[derive(Clone, Debug)]
#[listen_message("auditeur:registry_event")]
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
    /// The app is requesting for a [removable](super::map::DependencyType::Removable) dependency
    /// to be removed.
    RequestDependencyRemoval { id: DependencyId },
    /// The app has exited.
    Exit,
}

/// The state of a component in the [`App`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentLifecycle {
    /// The component was added to the app, but it hasn't been loaded yet.
    Add,
    /// The component was loaded in the app, but it hasn't been loaded in its registry yet.
    Load,
    /// The component was loaded in its registry; [`Loader::load`] was called.
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
