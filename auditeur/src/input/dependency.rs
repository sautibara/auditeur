use std::borrow::Cow;

use crate::{input::CloneAs, prelude::*};

#[cfg(doc)]
use crate::input::dependency;

/// A link that `dependent` depends on `dependency` in some way.
///
/// Both `dependent` and `dependency` can be any type of component in an [`App`]. For example, a
/// resource may depend on a message, or a resource could depend on a bundle. This is used to
/// determine when a component loads or unloads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub dependency: ComponentId,
    pub dependent: ComponentId,
}

/// A value that determines if a [`Link`] can be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Removable {
    Inherit,
    Set(bool),
}

/// A combination of a [`Link`] and [`Removable`] flag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Full {
    pub link: Link,
    pub removable: Removable,
}

/// A handle that keeps a [`Link`] alive.
///
/// Specifically, this sends [`RegistryEvent::IncrementDependency`] when it is created and
/// [`RegistryEvent::DecrementDependency`] when it is dropped.
#[derive(Debug)]
pub struct Handle {
    sender: EventSender,
    dep: Full,
}

impl Handle {
    /// Create a new handle, listing that `dep` should be maintained as long as this handle is
    /// alive. The necessary events are sent through `sender`.
    #[must_use]
    pub fn new(sender: EventSender, dep: Full) -> Self {
        sender.send(RegistryEvent::IncrementDependency(dep.clone()));
        Self { sender, dep }
    }

    /// Returns a new [`HandleBuilder`], see [`HandleBuilder::new`].
    #[must_use]
    pub fn builder(
        event_sender: EventSender,
        dependency: impl Into<ComponentId>,
        dependent: impl Into<ComponentId>,
    ) -> HandleBuilder<'static> {
        HandleBuilder::new(event_sender, dependency.into(), dependent.into())
    }

    /// Starts building a new [`Handle`] using this handle's information.
    #[must_use]
    pub fn clone_builder(&self) -> HandleBuilder<'_> {
        From::from(self)
    }

    /// Returns a reference to the dependency that this handle keeps alive.
    #[must_use]
    pub const fn dep(&self) -> &Full {
        &self.dep
    }

    /// Create a new handle to an uncreated resource, listing that `dep` should be maintained as
    /// long as this handle is alive. The necessary events are sent through `sender`.
    ///
    /// This "creates" the resource in the [`ResourceMap`](crate::app::ResourceMap) so that
    /// dependencies can be added to it. This can be used to group together multiple removable
    /// dependencies under a common name. To do this, create a removable dependency to a faux
    /// resource and then many unremovable dependencies from that faux resource to the grouped
    /// dependencies that you want to be able to remove.
    ///
    /// When the `ResourceMap` creates an unload plan, it will attempt to unload the faux resource
    /// and remove your dependency to it using [`RegistryEvent::RemoveDependency`]. That is, only
    /// this specific dependency will be included in the `RemoveDependency` event.
    ///
    /// # Panics
    ///
    /// - If `dep.link.dependency` is not a resource.
    #[must_use]
    pub fn new_to_faux_resource(sender: EventSender, dep: Full, bundle: NamespacedId) -> Self {
        let ComponentId::Resource(dependency) = dep.link.dependency.clone() else {
            panic!("expected the dependency to be a resource");
        };

        // create the faux resource
        sender.send(RegistryEvent::Adopt {
            component: ComponentId::Resource(dependency.clone()),
            bundle: bundle.clone(),
        });
        sender.send(RegistryEvent::Resolving {
            id: dependency.clone(),
            bundle: bundle.clone(),
        });
        sender.send(RegistryEvent::Loading {
            id: dependency,
            bundle,
        });

        // add the dependency
        Self::new(sender, dep)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.sender
            .send(RegistryEvent::DecrementDependency(self.dep.clone()));
    }
}

impl CloneAs for Handle {
    fn clone_as(&self, dependent: ComponentIdRef<'_>) -> Self
    where
        Self: Sized,
    {
        self.clone_builder().dependent(dependent).build()
    }
}

pub struct HandleBuilder<'a> {
    event_sender: EventSender,
    dependency: Cow<'a, ComponentId>,
    dependent: Cow<'a, ComponentId>,
    removable: Removable,
}

impl HandleBuilder<'static> {
    /// Returns a new [`HandleBuilder`] with the specified information
    /// and `removable` set to [`Removable::Inherit`].
    #[must_use]
    pub const fn new(
        event_sender: EventSender,
        dependency: ComponentId,
        dependent: ComponentId,
    ) -> Self {
        Self {
            event_sender,
            dependency: Cow::Owned(dependency),
            dependent: Cow::Owned(dependent),
            removable: Removable::Inherit,
        }
    }
}

impl<'a> From<&'a Handle> for HandleBuilder<'a> {
    fn from(value: &'a Handle) -> Self {
        Self {
            event_sender: value.sender.clone(),
            dependency: Cow::Borrowed(&value.dep.link.dependency),
            dependent: Cow::Borrowed(&value.dep.link.dependent),
            removable: value.dep.removable,
        }
    }
}

impl HandleBuilder<'_> {
    fn into_parts(self) -> (EventSender, Full) {
        let Self {
            event_sender,
            dependency,
            dependent,
            removable,
        } = self;

        (
            event_sender,
            Full {
                link: Link {
                    dependency: dependency.into_owned(),
                    dependent: dependent.into_owned(),
                },
                removable,
            },
        )
    }

    /// See [`Handle::new`].
    #[must_use]
    pub fn build(self) -> Handle {
        let (event_sender, full) = self.into_parts();
        Handle::new(event_sender, full)
    }

    /// See [`Handle::new_to_faux_resource`].
    #[must_use]
    pub fn build_to_faux_resource(self, bundle: NamespacedId) -> Handle {
        let (event_sender, full) = self.into_parts();
        Handle::new_to_faux_resource(event_sender, full, bundle)
    }
}

impl HandleBuilder<'_> {
    #[must_use]
    pub fn removable(self, new_value: bool) -> Self {
        Self {
            removable: Removable::Set(new_value),
            ..self
        }
    }

    #[must_use]
    pub fn inherit_removable(self) -> Self {
        Self {
            removable: Removable::Inherit,
            ..self
        }
    }

    #[must_use]
    pub fn dependent(self, dependent: impl Into<ComponentId>) -> Self {
        Self {
            dependent: Cow::Owned(dependent.into()),
            ..self
        }
    }
}
