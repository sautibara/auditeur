use std::collections::HashSet;

use crate::prelude::*;

#[derive(Default, Debug)]
pub struct Constraints {
    constraints: HashSet<Constraint>,
}

impl Constraints {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            constraints: HashSet::new(),
        }
    }

    #[must_use]
    pub fn insert(mut self, constraint: Constraint) -> Self {
        self.constraints.insert(constraint);
        self
    }

    #[must_use]
    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.insert(Constraint::AfterResource { id: id.into() })
    }

    #[must_use]
    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.insert(Constraint::AfterBundle { id: id.into() })
    }

    #[must_use]
    pub fn extend(mut self, other: Self) -> Self {
        self.constraints.extend(other.constraints);
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }
}

impl IntoIterator for Constraints {
    type Item = Constraint;
    type IntoIter = std::collections::hash_set::IntoIter<Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.into_iter()
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
#[non_exhaustive]
pub enum Constraint {
    AfterResource { id: NamespacedId },
    AfterBundle { id: NamespacedId },
}
