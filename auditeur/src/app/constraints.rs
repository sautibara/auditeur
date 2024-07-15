use std::collections::HashSet;

use crate::prelude::*;

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

    /// Specify that the [`Resource`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.insert(Constraint::AfterResource { id: id.into() })
    }

    /// Specify that the [`Bundle`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.insert(Constraint::AfterBundle { id: id.into() })
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
    /// Only load the resource after the other resource with the id `id` has been loaded
    AfterResource { id: NamespacedId },
    /// Only load the resource after the bundle with the id `id` has been loaded
    AfterBundle { id: NamespacedId },
}
