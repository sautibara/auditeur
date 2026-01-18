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
        let sender = self.sender.clone();
        let dep = Full {
            link: Link {
                dependency: self.dep.link.dependency.clone(),
                dependent: dependent.into_owned(),
            },
            removable: self.dep.removable,
        };
        Self::new(sender, dep)
    }
}
