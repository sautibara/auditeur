//! Identifiers returned from the [`App`] with special guarantees.
//!
//! When methods from this crate return identifiers from within this module, they guarantee that
//! the identifiers' guarantees are valid right when the identifier is returned. For example, if
//! an [`AddedComponentId`] is returned from a [`Map`](super::map::Map), the consumer can know
//! that the component was added to the app when the map was obtained. Note that this guarantee
//! doesn't extend beyond the time in which the identifier was obtained; keeping an
//! [`AddedComponentId`] alive will not keep the component added.
//!
//! Identifiers related to components include:
//! - [`AddedComponentId`],
//! - [`LoadedComponentId`],
//!     - [`LoadIndex`],
//!
//! Identifiers related to dependencies include:
//! - [`DependencyId`],
//! - [`AddedDependencyId`],
//! - [`LoadedDependencyId`],
//!     - [`DependencyLoads`],
//! - [`DependencyPart`],
//!
//! Identifiers related to registries include:
//! - [`LoadedRegistryId`],

use std::{
    borrow::Borrow,
    fmt::{Debug, Display},
    hash::Hash,
    ops::Deref,
};

use crate::{
    component::{ComponentIdPath, registry::Registries},
    prelude::*,
};

/// An specifically indexed load, mostly used when relating to components.
///
/// If obtained from the app, it is guaranteed that this load is active.
///
/// See [`LoadedComponentId`] or [`LoadedDependencyId`] for more information.
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LoadIndex(pub(crate) usize);

impl Display for LoadIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A [`Component`] that is currently added.
///
/// See [`LoadedComponentId`] for a component that is also loaded.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AddedComponentId(pub ComponentId);

impl AddedComponentId {
    pub(crate) const fn new_inner(id: ComponentId) -> Self {
        Self(id)
    }

    pub(crate) fn app() -> Self {
        Self(App::component_id())
    }

    pub(crate) const fn loaded_unchecked(self, index: LoadIndex) -> LoadedComponentId {
        LoadedComponentId {
            component: self,
            index,
        }
    }

    #[must_use]
    pub fn clone_id(&self) -> ComponentId {
        self.deref().clone()
    }

    #[must_use]
    pub fn into_id(self) -> ComponentId {
        self.0
    }
}

impl Deref for AddedComponentId {
    type Target = ComponentId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<ComponentId> for AddedComponentId {
    fn borrow(&self) -> &ComponentId {
        &self.0
    }
}

impl Display for AddedComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ComponentId> for AddedComponentId {
    fn from(value: ComponentId) -> Self {
        Self(value)
    }
}

pub(crate) type RegistryId = &'static NamespacedIdRef;

/// A [`Registry`](crate::component::registry::Registry) that is currently loaded.
///
/// This means that the registry has components added to it.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedRegistryId(pub RegistryId);

impl LoadedRegistryId {
    pub(crate) fn component_id(&self) -> AddedComponentId {
        AddedComponentId(ComponentId {
            registry: Registries::id(),
            component: ComponentIdPath::Static(self.0),
        })
    }
}

impl Deref for LoadedRegistryId {
    type Target = RegistryId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A [`Component`] that is currently loaded, with a specific index.
///
/// If a component is gets unloaded and then loaded again immediately afterwards, there's a chance
/// that it doesn't completely unload before the load begins. This means that there could be
/// multiple loads of a component active at a single time. Load indicies are used to separate the
/// different loads of a component.
///
/// See [`AddedComponentId`] for a component that is added but may not be loaded.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedComponentId {
    pub component: AddedComponentId,
    pub index: LoadIndex,
}

impl LoadedComponentId {
    #[must_use]
    pub fn into_id(self) -> ComponentId {
        self.component.into_id()
    }
}

impl Display for LoadedComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' ({})", self.component, self.index)
    }
}

impl Deref for LoadedComponentId {
    type Target = AddedComponentId;
    fn deref(&self) -> &Self::Target {
        &self.component
    }
}

/// A [`Dependency`] that may or may not be added.
///
/// This doesn't come with any guarantees.
///
/// See [`AddedDependencyId`] for a dependency that was added to the app.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct DependencyId {
    pub dependent: ComponentId,
    pub dependency: ComponentId,
}

impl Display for DependencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' -> '{}'", self.dependent, self.dependency)
    }
}

/// A [`Dependency`] that is currently added.
///
/// See [`DependencyId`] for a dependency that may or may not be added, or [`LoadedDependencyId`]
/// for a dependency that is loaded.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AddedDependencyId {
    pub dependent: AddedComponentId,
    pub dependency: AddedComponentId,
}

impl AddedDependencyId {
    pub(crate) fn new_unchecked(id: DependencyId) -> Self {
        Self {
            dependent: AddedComponentId(id.dependent),
            dependency: AddedComponentId(id.dependency),
        }
    }

    pub(crate) fn into_id(self) -> DependencyId {
        DependencyId {
            dependent: self.dependent.into_id(),
            dependency: self.dependency.into_id(),
        }
    }
}

/// [Load indicies](LoadIndex) for the dependency and dependent of a [`Dependency`].
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct DependencyLoads {
    pub dependent: LoadIndex,
    pub dependency: LoadIndex,
}

/// A [`Dependency`] that is currently loaded.
///
/// See [`AddedDependencyId`] for a dependency that is added but may not be loaded.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedDependencyId {
    pub id: AddedDependencyId,
    pub loads: DependencyLoads,
}

impl LoadedDependencyId {
    #[must_use]
    pub fn new(dependent: LoadedComponentId, dependency: LoadedComponentId) -> Self {
        Self {
            id: AddedDependencyId {
                dependent: dependent.component,
                dependency: dependency.component,
            },
            loads: DependencyLoads {
                dependent: dependent.index,
                dependency: dependency.index,
            },
        }
    }

    #[must_use]
    pub fn into_id(self) -> DependencyId {
        self.id.into_id()
    }

    #[must_use]
    pub fn dependent(&self) -> LoadedComponentId {
        (self.id.dependent.clone()).loaded_unchecked(self.loads.dependent)
    }

    #[must_use]
    pub fn dependency(&self) -> LoadedComponentId {
        (self.id.dependency.clone()).loaded_unchecked(self.loads.dependency)
    }
}

impl Display for LoadedDependencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {}", self.dependent(), self.dependency())
    }
}

/// A [`Dependency`] that is loaded, plus a single [`DependencyType`].
///
/// A single dependency may be made up of multiple parts - multiple [`Dependency`] handles -
/// that all have separate [types](DependencyType). The dependency is only considered removed
/// if all parts are dropped.
///
/// See [`LoadedDependencyId`] for a dependency that is loaded with no extra type.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct DependencyPart {
    pub id: LoadedDependencyId,
    pub ty: DependencyType,
}
