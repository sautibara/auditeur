use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse::Parse, parse_quote, punctuated::Punctuated, spanned::Spanned, FnArg, Ident, ImplItem,
    ImplItemFn, ItemImpl, LitBool, Path, Token, Type,
};

use crate::{
    field::{NameValue, Parenthesized},
    input_list::{InputList, InputReferenceList},
};

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

#[derive(Default)]
pub struct HasInputArgs {
    args: Punctuated<HasInputArg, Token![,]>,
}

impl HasInputArgs {
    const CRATE_ROOT_DEFAULT: bool = false;
    pub fn top_level(&self) -> bool {
        self.args
            .iter()
            .filter_map(HasInputArg::as_crate_root)
            .last()
            .unwrap_or(Self::CRATE_ROOT_DEFAULT)
    }

    const OWNED_DEFAULT: bool = false;
    pub fn owned(&self) -> bool {
        self.args
            .iter()
            .filter_map(HasInputArg::as_owned)
            .last()
            .unwrap_or(Self::OWNED_DEFAULT)
    }
}

impl Parse for HasInputArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(HasInputArg::parse, Token![,])
            .map(|args| Self { args })
    }
}

impl ToTokens for HasInputArgs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.args.to_tokens(tokens);
    }
}

enum HasInputArg {
    CrateRoot(Ident),
    CrateRootExplicit(NameValue<LitBool>),
    Owned(Ident),
    OwnedExplicit(NameValue<LitBool>),
}

impl HasInputArg {
    const fn as_crate_root(&self) -> Option<bool> {
        match self {
            Self::CrateRoot(_) => Some(!HasInputArgs::CRATE_ROOT_DEFAULT),
            Self::CrateRootExplicit(name_value) => Some(name_value.value.value),
            _ => None,
        }
    }

    const fn as_owned(&self) -> Option<bool> {
        match self {
            Self::Owned(_) => Some(!HasInputArgs::OWNED_DEFAULT),
            Self::OwnedExplicit(name_value) => Some(name_value.value.value),
            _ => None,
        }
    }
}

impl ToTokens for HasInputArg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::CrateRoot(value) | Self::Owned(value) => value.to_tokens(tokens),
            Self::CrateRootExplicit(value) | Self::OwnedExplicit(value) => value.to_tokens(tokens),
        }
    }
}

impl Parse for HasInputArg {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        let explicit = input
            .peek(Token![=])
            .then(|| NameValue::parse_after(ident.clone(), input))
            .transpose()?;

        match (&*ident.to_string(), explicit) {
            ("crate_root", Some(name_value)) => Ok(Self::CrateRootExplicit(name_value)),
            ("crate_root", None) => Ok(Self::CrateRoot(ident)),
            ("owned", Some(name_value)) => Ok(Self::OwnedExplicit(name_value)),
            ("owned", None) => Ok(Self::Owned(ident)),
            _ => Err(syn::Error::new_spanned(
                ident,
                "expected one of 'crate_root' or 'owned'",
            )),
        }
    }
}

pub struct HasInput {
    implementation: ItemImpl,
    args: HasInputArgs,
}

fn uppercase_first_letter(string: &str) -> String {
    let mut chars = string.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl HasInput {
    pub fn parse_entire(
        item: proc_macro::TokenStream,
        attr: proc_macro::TokenStream,
    ) -> syn::Result<Self> {
        Ok(Self {
            implementation: syn::parse(item)?,
            args: (!attr.is_empty())
                .then(|| syn::parse(attr))
                .transpose()?
                .unwrap_or_default(),
        })
    }

    fn funcs_mut(items: &mut [ImplItem]) -> impl Iterator<Item = &mut ImplItemFn> {
        // TODO: no_input attribute
        items.iter_mut().filter_map(|item| {
            if let ImplItem::Fn(item) = item {
                Some(item)
            } else {
                None
            }
        })
    }

    fn type_ident_of(method_ident: &Ident) -> Ident {
        let const_ident = method_ident.to_string();
        let const_ident = uppercase_first_letter(&const_ident);
        let const_ident = const_ident + "Input";
        syn::Ident::new(&const_ident, method_ident.span())
    }

    pub fn expand(mut self) -> syn::Result<proc_macro2::TokenStream> {
        if self.args.owned() {
            return Err(syn::Error::new_spanned(
                self.args,
                "'owned' is not implemented yet",
            ));
        }

        let Some((None, trait_, _)) = &self.implementation.trait_ else {
            return Err(syn::Error::new_spanned(
                self.implementation,
                "expected this to be a trait implementation",
            ));
        };
        let (_, trait_generics, _) = self.implementation.generics.split_for_impl();
        let trait_: Type = parse_quote!( #trait_ #trait_generics );

        let implementer = &self.implementation.self_ty;

        let mut types = Vec::new();
        let mut macros = proc_macro2::TokenStream::new();
        let func_count = Self::funcs_mut(&mut self.implementation.items).count();
        for func in Self::funcs_mut(&mut self.implementation.items) {
            let last_arg_span = func.sig.inputs.last().span();
            let last_arg = func.sig.inputs.last_mut();
            let last_arg = last_arg
                .and_then(|input| {
                    if let FnArg::Typed(input) = input {
                        Some(input)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    syn::Error::new(last_arg_span, "expected at least one typed argument")
                })?;

            let const_ident = Self::type_ident_of(&func.sig.ident);
            let referenced_inputs: InputReferenceList = syn::parse2(last_arg.ty.to_token_stream())?;
            let referenced_inputs = referenced_inputs.with_lifetime("'a");
            types.push(parse_quote!( type #const_ident<'a> = #referenced_inputs; ));

            let owned_inputs = referenced_inputs.into_owned_list();
            let macro_ident = if func_count > 1 {
                input_macro_of_method(&trait_, implementer, &func.sig.ident.to_string())?
            } else {
                input_macro_of(&trait_, implementer)?
            };
            macros.extend(super::export_input(&macro_ident, &owned_inputs, &self.args));

            last_arg.ty = parse_quote!( <Self as #trait_>::#const_ident<'_> );
        }

        self.implementation.items.extend(types);
        let implementation = self.implementation;
        Ok(quote! {
            #macros
            #implementation
        })
    }
}

pub fn export(macro_ident: &Ident, inputs: &InputList, args: &HasInputArgs) -> TokenStream {
    let mut stream = quote! {
        #[allow(non_local_definitions)]
        #[macro_export]
        #[doc(hidden)]
        macro_rules! #macro_ident {
            (($callback:path $(, $($callbacks:tt)*)?) ($($params:tt)*) $($rest:tt)*) => {
                $callback!(($($($callbacks)*)?) ($($params)*) $($rest)* #inputs);
            };
        }
    };
    if !args.top_level() {
        stream.extend(quote! {
            #[allow(unused_imports)]
            #[doc(hidden)]
            pub(crate) use #macro_ident;
        });
    }
    stream
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
