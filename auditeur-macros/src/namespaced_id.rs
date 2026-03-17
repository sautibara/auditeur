use proc_macro2::Span;
use quote::{ToTokens, quote_spanned};
use syn::{LitStr, parse::Parse};

#[derive(Debug, Clone)]
pub struct NamespacedId(LitStr);

impl NamespacedId {
    pub fn new(value: &str, span: Span) -> Self {
        Self(LitStr::new(value, span))
    }

    pub fn concat(self, value: &str) -> Self {
        let mut string = self.0.value();
        string += value;
        Self::new(&string, self.0.span())
    }
}

impl Parse for NamespacedId {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse().map(Self)
    }
}

impl ToTokens for NamespacedId {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let string = &self.0;
        let span = self.0.span();

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote_spanned! {
            span => #auditeur::namespaced_id::ident!(#string)
        });
    }
}
