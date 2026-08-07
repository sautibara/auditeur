use std::{
    borrow::Borrow,
    fmt::{Debug, Display},
    hash::Hash,
    ops::{AddAssign, Deref, SubAssign},
};

use crate::{
    component::{ComponentIdPath, registry::Registries},
    prelude::*,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LoadIndex(pub(crate) usize);

impl Display for LoadIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An id of a component that is guaranteed to be currently added in the map when returned by it.
///
/// Note that this guarantee is only valid at the exact moment that the id is obtained from the
/// map - keeping this id alive doesn't keep the component added.
// WARN: Users of the library should not be able to give these to prove that a component is added
// - it should only be used as a guarantee within the library.
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

/// An id of a specifically numbered load of a component.
///
/// This has the same guarantees as [`AddedComponentId`].
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

/// An id of a dependency that may or may not be added to the map yet.
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

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct DependencyLoads {
    pub dependent: LoadIndex,
    pub dependency: LoadIndex,
}

/// An id of a loaded dependency in the map.
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

/// An id of a part of a loaded dependency in the map.
///
/// A single dependency may be made up of multiple parts - multiple [`Dependency`] handles -
/// that all have separate [types](DependencyType). The dependency is only considered removed
/// if all parts are dropped.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct DependencyPart {
    pub id: LoadedDependencyId,
    pub ty: DependencyType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DependencyTypeCounts {
    pub unremovable: usize,
    pub removable: usize,
    pub transient: usize,
    pub weak: usize,
}

impl DependencyTypeCounts {
    pub(crate) const fn is_zero(&self) -> bool {
        self.unremovable == 0 && self.removable == 0 && self.transient == 0 && self.weak == 0
    }

    pub(crate) const fn effective_type(&self) -> DependencyType {
        if self.unremovable > 0 {
            DependencyType::Unremovable
        } else if self.removable > 0 {
            DependencyType::Removable
        } else if self.transient > 0 {
            DependencyType::Transient
        } else if self.weak > 0 {
            DependencyType::Weak
        } else {
            DependencyType::Transient
        }
    }
}

impl AddAssign<DependencyType> for DependencyTypeCounts {
    fn add_assign(&mut self, ty: DependencyType) {
        match ty {
            DependencyType::Unremovable => self.unremovable += 1,
            DependencyType::Removable => self.removable += 1,
            DependencyType::Transient => self.transient += 1,
            DependencyType::Weak => self.weak += 1,
        }
    }
}

impl SubAssign<DependencyType> for DependencyTypeCounts {
    fn sub_assign(&mut self, ty: DependencyType) {
        match ty {
            DependencyType::Unremovable => self.unremovable = self.unremovable.saturating_sub(1),
            DependencyType::Removable => self.removable = self.removable.saturating_sub(1),
            DependencyType::Transient => self.transient = self.transient.saturating_sub(1),
            DependencyType::Weak => self.weak = self.weak.saturating_sub(1),
        }
    }
}
