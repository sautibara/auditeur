use std::{
    fmt::{self, Display},
    str::FromStr,
};

use proc_macro2::Span;
use quote::quote;
use quote::ToTokens;
use syn::{parse::Parse, LitStr};
use thiserror::Error;

use crate::Auditeur;

#[derive(Debug, Clone)]
pub struct NamespacedId {
    pub namespace: String,
    pub id: String,
}

impl NamespacedId {
    pub fn to_lit_str(&self) -> LitStr {
        let combined = format!("{}:{}", self.namespace, self.id);
        LitStr::new(&combined, Span::call_site())
    }
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

impl ToTokens for NamespacedId {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let namespaced_id = Auditeur::new().id();

        let namespace_size = self.namespace.len();
        let string = self.to_string();
        let bytes = string.bytes();

        tokens.extend(quote! {
            {
                const COERCE: &#namespaced_id::NamespacedIdRef = unsafe {
                    const COERCE: &#namespaced_id::Coerce<[u8]> = &#namespaced_id::Coerce {
                        namespace_size: #namespace_size,
                        data: [#(#bytes),*],
                    };
                    // SAFETY: Coerce<[u8]>, Coerce<str>, and NamespacedIdRef all have the same layout
                    // and this is valid utf-8.
                    std::mem::transmute(COERCE)
                };
                COERCE
            }
        });
    }
}
