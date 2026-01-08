use quote::ToTokens;
use syn::{
    Attribute, Expr, ExprLit, GenericParam, Ident, Item, Lit, Token, WhereClause, parenthesized,
    spanned::Spanned,
};

use crate::GenericArgList;

use quote::quote;

#[derive(Default)]
pub struct CustomGenerics {
    pub impl_generics: proc_macro2::TokenStream,
    pub type_generics: GenericArgList,
    pub where_clause: proc_macro2::TokenStream,
}

impl CustomGenerics {
    pub fn from_item(item: &mut Item) -> syn::Result<(&Ident, Self)> {
        let (ident, attrs, generics) = match item {
            Item::Struct(strct) => (&strct.ident, &mut strct.attrs, &strct.generics),
            Item::Enum(strct) => (&strct.ident, &mut strct.attrs, &strct.generics),
            _ => return Err(syn::Error::new_spanned(&item, "expected a struct or enum")),
        };

        let generics = Self::from_attr_or_generics(attrs, generics)?;
        attrs.retain(|attr| !Self::attr_matches(attr));

        Ok((ident, generics))
    }

    pub fn from_generics(generics: &syn::Generics) -> Self {
        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
        Self {
            impl_generics: impl_generics.into_token_stream(),
            type_generics: syn::parse2(type_generics.into_token_stream())
                .expect("parsing should never fail"),
            where_clause: where_clause.into_token_stream(),
        }
    }

    pub fn from_attr_or_generics(
        attrs: &[Attribute],
        generics: &syn::Generics,
    ) -> syn::Result<Self> {
        for attr in attrs {
            if let syn::Meta::NameValue(meta) = &attr.meta
                && meta
                    .path
                    .get_ident()
                    .is_some_and(|ident| ident == "resource_custom_generics")
            {
                let Expr::Lit(ExprLit {
                    lit: Lit::Str(string),
                    ..
                }) = &meta.value
                else {
                    return Err(syn::Error::new(
                        meta.value.span(),
                        "expected string literal",
                    ));
                };
                return syn::parse_str(&string.value());
            }
        }
        Ok(Self::from_generics(generics))
    }

    pub fn attr_matches(attr: &Attribute) -> bool {
        if let syn::Meta::NameValue(meta) = &attr.meta {
            meta.path
                .get_ident()
                .is_some_and(|ident| ident == "resource_custom_generics")
        } else {
            false
        }
    }
}

impl syn::parse::Parse for CustomGenerics {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        parenthesized!(content in input);
        let impl_generics = content.parse_terminated(GenericParam::parse, Token![,])?;
        let _: Token![-] = input.parse()?;
        let content;
        parenthesized!(content in input);
        let type_generics = content.parse_terminated(GenericParam::parse, Token![,])?;
        let where_clause: Option<WhereClause> = input.parse()?;
        Ok(Self {
            impl_generics: quote! { <#impl_generics> },
            type_generics: GenericArgList {
                params: Some(type_generics),
            },
            where_clause: where_clause.into_token_stream(),
        })
    }
}
