use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitBool, Path, Token, Type, TypePath,
    parse::Parse, parse_quote, punctuated::Punctuated, spanned::Spanned,
};

use crate::{
    field::NameValue,
    input_list::{InputList, InputReferenceList},
};

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
            format!(
                "{string_rep} is too complex of a type - only types with characters in the range of ([\\w_<>]|::) are supported."
            ),
        ))
    }
}

pub fn input_macro_ident_of(trait_: &Type, implementer: &Type) -> syn::Result<Ident> {
    let trait_ = type_to_ident(trait_)?;
    let implementer = type_to_ident(implementer)?;
    let full_ident = format!("auditeur_input_of_impl_{trait_}_for_{implementer}");
    Ok(Ident::new(&full_ident, implementer.span()))
}

pub fn input_macro_ident_of_method(
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
        #[doc(hidden)]
        #[allow(non_snake_case, clippy::redundant_pub_crate)]
        #[allow(unused_imports)]
        pub(crate) mod #macro_ident {
            #[macro_export]
            #[doc(hidden)]
            macro_rules! #macro_ident {
                (($callback:path $(, $($callbacks:tt)*)?) ($($params:tt)*) $($rest:tt)*) => {
                    $callback!(($($($callbacks)*)?) ($($params)*) $($rest)* #inputs);
                };
            }
            pub use #macro_ident;
        }
    }
}

#[derive(Default)]
pub struct HasInputArgs {
    args: Punctuated<HasInputArg, Token![,]>,
}

impl HasInputArgs {
    const OWNED_DEFAULT: bool = false;
    pub fn owned(&self) -> bool {
        self.args
            .iter()
            .filter_map(HasInputArg::as_owned)
            .next_back()
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
    Owned(Ident),
    OwnedExplicit(NameValue<LitBool>),
}

impl HasInputArg {
    #[expect(clippy::unnecessary_wraps)]
    const fn as_owned(&self) -> Option<bool> {
        match self {
            Self::Owned(_) => Some(!HasInputArgs::OWNED_DEFAULT),
            Self::OwnedExplicit(name_value) => Some(name_value.value.value),
        }
    }
}

impl ToTokens for HasInputArg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Owned(value) => value.to_tokens(tokens),
            Self::OwnedExplicit(value) => value.to_tokens(tokens),
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
            ("owned", Some(name_value)) => Ok(Self::OwnedExplicit(name_value)),
            ("owned", None) => Ok(Self::Owned(ident)),
            _ => Err(syn::Error::new_spanned(ident, "expected 'owned'")),
        }
    }
}

pub struct HasInput {
    implementation: ItemImpl,
    args: HasInputArgs,
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
        let const_ident = crate::snake_to_upper_camel_case(&const_ident);
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
        let trait_ = Type::Path(TypePath {
            qself: None,
            path: trait_.clone(),
        });
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
                input_macro_ident_of_method(&trait_, implementer, &func.sig.ident.to_string())?
            } else {
                input_macro_ident_of(&trait_, implementer)?
            };
            macros.extend(super::export_input(&macro_ident, &owned_inputs));

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

#[derive(Default, Debug)]
pub struct CallbackList {
    pub list: Punctuated<Path, syn::token::Comma>,
}

impl CallbackList {
    pub fn push_front(&mut self, path: Path) {
        self.list.insert(0, path);
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
