use std::collections::HashSet;

use crate::{
    app::event::{ComponentId, ComponentIdRef, DependencyLink},
    prelude::*,
};

/// A set of [constraints](Constraint) that must be satisfied before a [resource](Resource) or
/// [bundle](Bundle) can be loaded in an [app](App)
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Constraints {
    constraints: HashSet<Constraint>,
}

impl Constraints {
    /// Create a new, empty set of [constraints](Constraint)
    #[must_use]
    pub fn empty() -> Self {
        Self {
            constraints: HashSet::new(),
        }
    }

    /// Insert a [`Constraint`] into the set
    #[must_use]
    pub fn insert(mut self, constraint: Constraint) -> Self {
        self.constraints.insert(constraint);
        self
    }

    /// Specify that the component with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_component(self, id: impl Into<ComponentId>) -> Self {
        self.insert(Constraint::After(id.into()))
    }

    /// Specify that the [`Resource`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Resource(id.into()))
    }

    /// Specify that the [`Bundle`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Bundle(id.into()))
    }

    /// Specify that the [`Message`](crate::Message) with id `id` must be loaded before loading this
    /// resource or bundle
    #[must_use]
    pub fn after_message(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Message(id.into()))
    }

    /// Combine the constraints in both this set and the `other` set, removing any duplicates
    #[must_use]
    pub fn extend(mut self, other: Self) -> Self {
        self.constraints.extend(other.constraints);
        self
    }

    /// Returns `true` if there are no [constraints](Constraint) within this set
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    /// Obtain a borrowing iterator over each [`Constraint`] in the set
    pub fn iter(&self) -> impl Iterator<Item = &Constraint> {
        self.constraints.iter()
    }

    /// Creates a set of dependency links that models this constraint.
    #[must_use]
    pub fn into_dependencies(self, dependent: ComponentIdRef<'_>) -> HashSet<DependencyLink> {
        self.constraints
            .into_iter()
            .map(|constraint| match constraint {
                Constraint::After(component) => DependencyLink {
                    dependency: component,
                    dependent: dependent.into_owned(),
                },
            })
            .collect()
    }
}

impl IntoIterator for Constraints {
    type Item = Constraint;
    type IntoIter = std::collections::hash_set::IntoIter<Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.into_iter()
    }
}

impl<'a> IntoIterator for &'a Constraints {
    type Item = &'a Constraint;
    type IntoIter = std::collections::hash_set::Iter<'a, Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.iter()
    }
}

/// A constraint that must be satisfied before a [resource](Resource) or [bundle](Bundle) can be
/// loaded in an [app](App)
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Constraint {
    /// Only load the resource after the other component with the id `id` has been loaded
    After(ComponentId),
}
