use std::ops::{Deref, DerefMut};

use quote::ToTokens;
use syn::{
    Ident, Token, bracketed, parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token::{Bracket, Paren},
};

fn parse_ident(input: ParseStream, expected_ident: &str) -> syn::Result<Ident> {
    let ident: Ident = input.parse()?;
    if ident == expected_ident {
        Ok(ident)
    } else {
        Err(syn::Error::new_spanned(
            ident,
            format!("expected '{expected_ident}'"),
        ))
    }
}

pub struct NameValue<T> {
    pub ident: Ident,
    pub eq: syn::token::Eq,
    pub value: T,
}

impl<T> Deref for NameValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> NameValue<T> {
    pub fn to_tokens_with(
        &self,
        tokens: &mut proc_macro2::TokenStream,
        func: impl FnOnce(&mut proc_macro2::TokenStream),
    ) {
        self.ident.to_tokens(tokens);
        self.eq.to_tokens(tokens);
        func(tokens);
    }
}

impl<T: Parse> NameValue<T> {
    pub fn parse(input: ParseStream, expected_ident: &str) -> syn::Result<Self> {
        let ident = parse_ident(input, expected_ident)?;
        Self::parse_after(ident, input)
    }

    pub fn parse_after(ident: Ident, input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            ident,
            eq: input.parse()?,
            value: input.parse()?,
        })
    }
}

impl<T: Parse + ToTokens> ToTokens for NameValue<T> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.to_tokens_with(tokens, |tokens| self.value.to_tokens(tokens));
    }
}

pub struct NameValueArray<T: Parse> {
    pub ident: Ident,
    pub eq: Token![=],
    pub bracket: Bracket,
    pub values: Punctuated<T, Token![,]>,
}

impl<T: Parse> NameValueArray<T> {
    pub fn parse(input: ParseStream, expected_ident: &str) -> syn::Result<Self> {
        let content;
        Ok(Self {
            ident: parse_ident(input, expected_ident)?,
            eq: input.parse()?,
            bracket: bracketed!(content in input),
            values: content.parse_terminated(T::parse, Token![,])?,
        })
    }
}

impl<T: Parse + ToTokens> ToTokens for NameValueArray<T> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.ident.to_tokens(tokens);
        self.eq.to_tokens(tokens);
        self.bracket
            .surround(tokens, |tokens| self.values.to_tokens(tokens));
    }
}

pub struct List<T> {
    pub ident: Ident,
    pub parens: Paren,
    pub values: Punctuated<T, syn::token::Comma>,
}

impl<T> Deref for List<T> {
    type Target = Punctuated<T, syn::token::Comma>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> DerefMut for List<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

impl<T: Parse> List<T> {
    pub fn parse(input: ParseStream, expected_ident: &str) -> syn::Result<Self> {
        let ident = parse_ident(input, expected_ident)?;
        Self::parse_after(ident, input)
    }

    pub fn parse_after(ident: Ident, input: ParseStream) -> syn::Result<Self> {
        Self::parse_after_with(ident, input, T::parse)
    }
}

impl<T> List<T> {
    pub fn parse_after_with(
        ident: Ident,
        input: ParseStream,
        parser: fn(_: ParseStream<'_>) -> syn::Result<T>,
    ) -> syn::Result<Self> {
        let content;
        Ok(Self {
            ident,
            parens: parenthesized!(content in input),
            values: content.parse_terminated(parser, Token![,])?,
        })
    }
}

impl<T: ToTokens> ToTokens for List<T> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.ident.to_tokens(tokens);
        self.parens
            .surround(tokens, |tokens| self.values.to_tokens(tokens));
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type IntoIter = <&'a Punctuated<T, Token![,]> as IntoIterator>::IntoIter;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut List<T> {
    type IntoIter = <&'a mut Punctuated<T, Token![,]> as IntoIterator>::IntoIter;
    type Item = &'a mut T;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

#[derive(Default, Clone)]
pub struct Comma<T> {
    pub value: T,
    pub comma: Option<syn::token::Comma>,
}

impl<T> Comma<T> {
    pub fn parse_with(
        input: ParseStream<'_>,
        parser: fn(_: ParseStream<'_>) -> syn::Result<T>,
    ) -> syn::Result<Self> {
        Ok(Self {
            value: parser(input)?,
            comma: input.peek(Token![,]).then(|| input.parse()).transpose()?,
        })
    }

    pub fn to_tokens_with(
        &self,
        tokens: &mut proc_macro2::TokenStream,
        func: impl FnOnce(&mut proc_macro2::TokenStream),
    ) {
        func(tokens);
        self.comma.to_tokens(tokens);
    }

    pub const fn has_comma(&self) -> bool {
        self.comma.is_some()
    }
}

impl<T: Parse> Parse for Comma<T> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Self::parse_with(input, T::parse)
    }
}

impl<T: ToTokens> ToTokens for Comma<T> {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.to_tokens_with(tokens, |tokens| self.value.to_tokens(tokens));
    }
}
