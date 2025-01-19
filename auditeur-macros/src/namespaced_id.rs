use std::{
    fmt::{self, Display},
    str::FromStr,
};

use syn::{parse::Parse, LitStr};
use thiserror::Error;

pub struct NamespacedId {
    pub namespace: String,
    pub id: String,
}

impl Display for NamespacedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the id should not have more than one colon")]
    TooManyComponents,
    #[error("the id should have a colon in the middle")]
    NotEnoughComponents,
}

impl Parse for NamespacedId {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let string: LitStr = input.parse()?;
        string
            .value()
            .parse()
            .map_err(|err| syn::Error::new_spanned(&string, format!("{err:?}")))
    }
}
