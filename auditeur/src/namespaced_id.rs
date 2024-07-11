use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use thiserror::Error;

#[derive(PartialEq, Eq, Clone, Hash)]
pub struct NamespacedId {
    pub namespace: String,
    pub id: String,
}

impl NamespacedId {
    #[must_use]
    pub fn into_ref(&self) -> NamespacedIdRef<'_> {
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
        let mut components = s.split(':');
        let (namespace, id) = (components.next())
            .zip(components.next())
            .ok_or(ParseError::NotEnoughComponents)?;
        if components.next().is_some() {
            return Err(ParseError::TooManyComponents);
        }
        Ok(Self {
            namespace: namespace.to_string(),
            id: id.to_string(),
        })
    }
}

#[allow(clippy::module_name_repetitions)] // Ref is way too general
#[derive(PartialEq, Eq, Clone, Copy)]
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

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the id should not have more than one colon")]
    TooManyComponents,
    #[error("the id should have a colon in the middle")]
    NotEnoughComponents,
}
