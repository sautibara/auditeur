use crate::{input::CloneAs, prelude::*};

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

/// A value that determines if a [`DependencyLink`] can be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Removable {
    Inherit,
    Set(bool),
}

/// A combination of a [`DependencyLink`] and [`Removable`] flag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Full {
    pub link: Link,
    pub removable: Removable,
}

/// A handle that keeps a [`DependencyLink`] alive.
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
