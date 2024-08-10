//! An id that is prefixed with a namespace
//!
//! Usually, the namespace should be a more general id that represents many lower-level ids. For
//! example, all builtin [`resource`](mod@crate::resource)s in this crate have ids under the
//! `auditeur` namespace. This is a way to lessen the chances of name collisions while still
//! retaining an identifier that can accurately represent the resource through text
//! (`auditeur:resource_map` is a lot more recognizeable than `2dd42d85-d83d-4663-8fd1-e4e8e88faef5`).

use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use thiserror::Error;

/// A simple id that is prefixed with a namespace
///
/// There are two different versions of this type: [`NamespacedId`], which represents an owned id,
/// and [`NamespacedIdRef`] which represents a reference to an id. As such, many things return
/// [`NamespacedIdRef`], which you can then convert to this type through [`From`] or
/// [`NamespacedIdRef::into_owned`].
///
/// To create a [`NamespacedIdRef`] from a static string, use [`ident!`](crate::ident).
#[derive(PartialEq, Eq, Clone, Hash, PartialOrd, Ord)]
pub struct NamespacedId {
    pub namespace: String,
    pub id: String,
}

impl NamespacedId {
    #[must_use]
    pub fn referenced(&self) -> NamespacedIdRef<'_> {
        self.into()
    }
}

impl Debug for NamespacedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r#"NamespacedId "{self}""#)
    }
}

impl Display for NamespacedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.id)
    }
}

impl FromStr for NamespacedId {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let components: Vec<_> = s.split(':').collect();
        let [namespace, id] = &components[..] else {
            return match components.len() {
                0..=1 => Err(ParseError::NotEnoughComponents),
                2.. => Err(ParseError::TooManyComponents),
            };
        };
        Ok(Self {
            namespace: (*namespace).to_string(),
            id: (*id).to_string(),
        })
    }
}

/// A reference to a [`NamespacedId`]
#[allow(clippy::module_name_repetitions)] // Ref is way too general
#[derive(PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct NamespacedIdRef<'a> {
    pub namespace: &'a str,
    pub id: &'a str,
}

impl NamespacedIdRef<'_> {
    #[must_use]
    pub fn into_owned(self) -> NamespacedId {
        self.into()
    }
}

impl Debug for NamespacedIdRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r#"NamespacedIdRef "{self}""#)
    }
}

impl Display for NamespacedIdRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.id)
    }
}

impl<'a> From<&'a NamespacedId> for NamespacedIdRef<'a> {
    fn from(val: &'a NamespacedId) -> NamespacedIdRef<'_> {
        NamespacedIdRef {
            namespace: &val.namespace,
            id: &val.id,
        }
    }
}

impl<'a> From<NamespacedIdRef<'a>> for NamespacedId {
    fn from(value: NamespacedIdRef<'a>) -> Self {
        Self {
            namespace: value.namespace.to_owned(),
            id: value.id.to_owned(),
        }
    }
}

impl PartialEq<NamespacedIdRef<'_>> for NamespacedId {
    fn eq(&self, other: &NamespacedIdRef<'_>) -> bool {
        self.namespace == other.namespace && self.id == other.id
    }
}

impl PartialEq<NamespacedId> for NamespacedIdRef<'_> {
    fn eq(&self, other: &NamespacedId) -> bool {
        self.namespace == other.namespace && self.id == other.id
    }
}

/// An error encountered while parsing a [`NamespacedId`]
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the id should not have more than one colon")]
    TooManyComponents,
    #[error("the id should have a colon in the middle")]
    NotEnoughComponents,
}
