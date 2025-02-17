use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse::Parse, punctuated::Punctuated, spanned::Spanned, Ident, Path, Token, Type};

use crate::{field::Parenthesized, input_list::InputList};

use super::listener::Specifier;

pub fn type_to_ident(typ: &Type) -> syn::Result<Ident> {
    let string_rep = typ.to_token_stream().to_string();
    let mut normalized = string_rep
        .replace('_', "__")
        .replace('<', "_lt_")
        .replace('>', "_gt_")
        .replace("::", "_sep_");
    normalized.retain(|character| !character.is_whitespace());

    if normalized
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_')
    {
        Ok(Ident::new(&normalized, typ.span()))
    } else {
        Err(syn::Error::new_spanned(
            typ,
            format!("{string_rep} is too complex of a type - only types with characters in the range of ([\\w_<>]|::) are supported."),
        ))
    }
}

pub fn input_macro_of(trait_: &Type, implementer: &Type) -> syn::Result<Ident> {
    let trait_ = type_to_ident(trait_)?;
    let implementer = type_to_ident(implementer)?;
    let full_ident = format!("auditeur_input_of_impl_{trait_}_for_{implementer}");
    Ok(Ident::new(&full_ident, implementer.span()))
}

pub fn input_macro_of_method(
    trait_: &Type,
    implementer: &Type,
    method: &str,
) -> syn::Result<Ident> {
    let trait_ = type_to_ident(trait_)?;
    let implementer = type_to_ident(implementer)?;
    let full_ident = format!("auditeur_input_of_{method}_in_impl_{trait_}_for_{implementer}");
    Ok(Ident::new(&full_ident, implementer.span()))
}

pub fn export(macro_ident: &Ident, inputs: &InputList) -> TokenStream {
    quote! {
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #macro_ident {
            (($callback:path $(, $($callbacks:tt)*)?) ($($params:tt)*) $($rest:tt)*) => {
                $callback!(($($($callbacks)*)?) ($($params)*) $($rest)* #inputs);
            };
        }
        #[doc(hidden)]
        pub(crate) use #macro_ident;
    }
}

#[derive(Default, Debug)]
pub struct CallbackList {
    pub list: Punctuated<Path, syn::token::Comma>,
}

impl CallbackList {
    pub fn push_ident_front(&mut self, ident: Ident) {
        self.list.insert(0, Path::from(ident));
    }
}

impl Parse for CallbackList {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            list: input.parse_terminated(Path::parse_mod_style, Token![,])?,
        })
    }
}

impl ToTokens for CallbackList {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.list.to_tokens(tokens);
    }
}

pub struct ExpandInputSpecifier {
    callbacks: Parenthesized<CallbackList>,
    params: Parenthesized<ExpandSpecifierParams>,
    implementer: Type,
    rest: proc_macro2::TokenStream,
}

impl ExpandInputSpecifier {
    pub fn expand(mut self) -> syn::Result<proc_macro2::TokenStream> {
        let ExpandSpecifierParams {
            specifier,
            rest: params,
        } = self.params.value;

        match specifier.value {
            Specifier::None => self.rest.extend(quote! { () }),
            Specifier::Input(name_value) => {
                let input_list = name_value.value;
                self.rest.extend(quote! { #input_list });
            }
            // we'll have to fill in the specifier, so add it as a callback
            Specifier::Implementation(implementation) => {
                let macro_name = super::input_reflection::input_macro_of(
                    &implementation.value,
                    &self.implementer,
                )?;
                self.callbacks.value.push_ident_front(macro_name);
            }
        }

        let Self {
            callbacks,
            implementer,
            rest,
            ..
        } = self;

        let mut other_callbacks = callbacks.value.list.into_pairs();
        let callback = other_callbacks
            .next()
            .ok_or_else(|| {
                syn::Error::new(
                    callbacks.paren.span.span(),
                    "expected at least one callback",
                )
            })?
            .into_value();

        Ok(quote! { #callback!((#(#other_callbacks)*) (#params) #implementer #rest ); })
    }
}

impl Parse for ExpandInputSpecifier {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            callbacks: input.parse()?,
            params: input.parse()?,
            implementer: input.parse()?,
            rest: input.parse()?,
        })
    }
}

struct ExpandSpecifierParams {
    specifier: Parenthesized<Specifier>,
    rest: proc_macro2::TokenStream,
}

impl Parse for ExpandSpecifierParams {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            specifier: input.parse()?,
            rest: input.parse()?,
        })
    }
}

impl ToTokens for ExpandSpecifierParams {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.specifier.to_tokens(tokens);
        self.rest.to_tokens(tokens);
    }
}
