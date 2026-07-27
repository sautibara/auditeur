use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::ops::Deref;

use namespaced_id::ArcNamespacedId;

use crate::prelude::*;

use crate::component::registry::{Registries, Registry};

pub mod bundle;
pub mod listen;
pub mod registry;
pub mod resource;
pub mod take;

/// A type that can be added to a specific [`Registry`] as a component.
///
/// See the documentation of [`Registry`] for more information about components.
///
/// This trait usually has blanket implementations for each registry.
pub trait Component<R: Registry>: Identified {
    type ComponentInput;

    fn register(input: Self::ComponentInput) -> (R::Loader, Children);
}

/// A type that belongs to a single well-defined [`Registry`].
///
/// Types can be added to multiple registries as long as they implement [`Register`] for each of
/// them. This trait, however, is meant to be implemented only for types that only make sense to be
/// added to a single registry.
pub trait RegistryOf: Component<Self::Registry> {
    type Registry: Registry;
}

#[derive(Hash, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComponentId {
    pub registry: &'static NamespacedIdRef,
    pub component: ComponentIdPath,
}

impl ComponentId {
    #[must_use]
    pub fn within<R, C>() -> Self
    where
        R: Registry + IdentifiedType,
        C: Component<R> + IdentifiedType,
    {
        Self {
            registry: R::id(),
            component: ComponentIdPath::Static(C::id()),
        }
    }

    #[must_use]
    pub fn registry_id(&self) -> Self {
        Self {
            registry: Registries::id(),
            component: ComponentIdPath::Static(self.registry),
        }
    }
}

impl Display for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.registry, &*self.component)
    }
}

impl Debug for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ComponentId(\"{self}\")")
    }
}

#[derive(Debug, Clone)]
pub enum ComponentIdPath {
    Shared(ArcNamespacedId),
    Static(&'static NamespacedIdRef),
}

impl From<ArcNamespacedId> for ComponentIdPath {
    fn from(v: ArcNamespacedId) -> Self {
        Self::Shared(v)
    }
}

impl From<&'static NamespacedIdRef> for ComponentIdPath {
    fn from(v: &'static NamespacedIdRef) -> Self {
        Self::Static(v)
    }
}

impl From<NamespacedId> for ComponentIdPath {
    fn from(v: NamespacedId) -> Self {
        Self::Shared(v.into())
    }
}

impl Deref for ComponentIdPath {
    type Target = NamespacedIdRef;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(shared) => shared,
            Self::Static(stat) => stat,
        }
    }
}

impl From<ComponentIdPath> for NamespacedId {
    fn from(value: ComponentIdPath) -> Self {
        value.deref().to_owned()
    }
}

impl Hash for ComponentIdPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state);
    }
}

impl PartialEq for ComponentIdPath {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for ComponentIdPath {}

impl PartialOrd for ComponentIdPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentIdPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(&**other)
    }
}

/// A type whose values have associated [`NamespacedId`]s that uniquely represent them.
pub trait Identified {
    fn id_dyn(&self) -> &NamespacedIdRef;
}

/// A type that has an associated [`NamespacedId`] that uniquely represents it.
///
/// If a type implements this trait, then they must also guarantee that [`Self::id()`] equals
/// [`Self::dyn_id`] for all values.
pub trait IdentifiedType: Identified {
    fn id() -> &'static NamespacedIdRef;
}

/// A type that has a constant [`NamespacedId`] that uniquely represents it.
///
/// This is a optional trait to allow using types' identifiers in const context.
///
/// If a type implements this trait, then they must also guarantee that [`Self::ID`] equals
/// [`Self::id()`] equals [`Self::dyn_id`] for all values.
pub trait IdentifiedConst: IdentifiedType {
    const ID: &'static NamespacedIdRef;
}

/// A type whose values have a well-defined [`ComponentId`].
///
/// These values also must belong to a specific registry by definition.
pub trait IdentifiedComponent {
    fn component_id_dyn(&self) -> ComponentId;
}

/// Types that have a single well-defined [`ComponentId`].
///
/// This is only implemented for components that implement [`RegistryOf`].
pub trait IdentifiedComponentType {
    fn component_id() -> ComponentId;
}

impl<C> IdentifiedComponentType for C
where
    C: RegistryOf + IdentifiedType,
{
    fn component_id() -> ComponentId {
        ComponentId::within::<C::Registry, C>()
    }
}
