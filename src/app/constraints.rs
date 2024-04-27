use crate::prelude::*;

#[derive(Default, Debug)]
pub struct Constraints {
    constraints: Vec<Constraint>,
}

impl Constraints {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            constraints: Vec::new(),
        }
    }

    #[must_use]
    pub fn run_after(mut self, id: impl Into<NamespacedId>) -> Self {
        self.constraints.push(Constraint::After { id: id.into() });
        self
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
    type IntoIter = std::vec::IntoIter<Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.into_iter()
    }
}

#[derive(Debug)]
pub enum Constraint {
    After { id: NamespacedId },
}
