use crate::prelude::*;

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
}

pub enum Constraint {
    After { id: NamespacedId },
}
