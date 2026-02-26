use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Ident, ImplGenerics, Token, TypeGenerics, WhereClause, fold::Fold, parse::Parse,
    punctuated::Punctuated,
};

use crate::{
    attr::FoldAttr,
    field::List,
    maybe_error::{Continue, ContinueExt, MaybeErrorExt, drain_continues_into, drain_results_into},
};

pub fn create_attr(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let item: syn::Item = syn::parse2(item)?;
    let attr: Attrs = syn::parse2(attr)?;

    match item {
        syn::Item::Struct(strct) => {
            let (config, error) = ConfigStruct::new(&attr, &strct);
            let trait_impl = config.expand();

            if let Some(error) = error {
                return Err(error);
            }

            let strct = FoldAttr::remove("create").fold_item_struct(strct);
            Ok(quote! {
                #strct
                #trait_impl
            })
        }
        // TODO: support enums
        syn::Item::Enum(_) => Err(syn::Error::new_spanned(
            item,
            "expected struct - enums are not supported yet, but are planned in the future",
        )),
        _ => Err(syn::Error::new_spanned(item, "expected struct")),
    }
}

pub struct ConfigStruct<'a> {
    ident: &'a syn::Ident,
    fields: ConfigFields<'a>,
    shared: Config<'a>,
}

impl<'a> ConfigStruct<'a> {
    fn new(attrs: &'a Attrs, strct: &'a syn::ItemStruct) -> Continue<Self> {
        let mut error = None;
        let this = Self {
            ident: &strct.ident,
            fields: ConfigFields::new(strct).drain_into(&mut error),
            shared: Config::new(attrs, &strct.generics).drain_into(&mut error),
        };
        error.wrap(this)
    }

    fn fields_of_type(&self, is_input: bool) -> impl Iterator<Item = &ConfigField<'a>> + Clone {
        (self.fields.0.iter()).filter(move |field| field.is_input == is_input)
    }

    fn field_idents(&self, is_input: bool) -> impl Iterator<Item = FieldIdent<'a>> + Clone {
        self.fields_of_type(is_input).map(|field| field.ident)
    }

    fn field_types(&self, is_input: bool) -> impl Iterator<Item = &'a syn::Type> + Clone {
        self.fields_of_type(is_input).map(|field| field.ty)
    }

    fn expand(&self) -> TokenStream {
        let ident = self.ident;
        let plain_ident = format_ident!("{ident}Plain");
        let input_ident = format_ident!("{ident}Input");

        let plain_struct_def = self.expand_struct(false, &plain_ident);
        let input_struct_def = self.expand_struct(true, &input_ident);

        let input_impl = self.expand_input_impl(&input_ident);
        let trait_impl = self.expand_trait_impl(&plain_ident, &input_ident);
        let default_impl = match self.shared.auto_impl {
            AutoImpl::None => TokenStream::new(),
            AutoImpl::Default => self.expand_default(&plain_ident, &input_ident),
        };

        quote! {
            #plain_struct_def
            #input_struct_def
            #input_impl
            #trait_impl
            #default_impl
        }
    }

    fn expand_trait_impl(&self, plain_ident: &Ident, input_ident: &Ident) -> TokenStream {
        let auditeur = &*crate::AUDITEUR;
        let ident = self.ident;

        let external_input = &self.shared.external_inputs;

        let fields = self.fields.0.iter().map(|field| {
            let struct_ident = if field.is_input {
                quote! { input }
            } else {
                quote! { plain }
            };

            let combined_field = match field.ident {
                FieldIdent::Ident(ident) => quote! { #ident },
                FieldIdent::Index(index) => quote! { #index },
            };

            let inner_field = field.ident;

            quote! { #combined_field: #struct_ident.#inner_field }
        });

        let (impl_generics, type_generics, where_clause) = self.shared.split_generics();

        quote! {
            impl #impl_generics #auditeur::resource::CreateDeriveTypes for #ident #type_generics
                #where_clause
            {
                type Input = #input_ident;
                type Plain = #plain_ident #type_generics;

                type External = (#(#external_input),*);

                fn combine(input: #input_ident, plain: #plain_ident #type_generics) -> Self {
                    Self { #(#fields),* }
                }
            }
        }
    }

    fn expand_struct(&self, is_input: bool, name: &Ident) -> TokenStream {
        let fields = self.field_idents(is_input);
        let types = self.field_types(is_input);

        let attrs = if self.shared.auto_impl.is_default() && !is_input {
            quote! { #[derive(Default)] }
        } else {
            TokenStream::new()
        };

        // include generics only if this is the basic struct
        let generics = if is_input {
            TokenStream::new()
        } else {
            self.shared.generics.to_token_stream()
        };

        quote! {
            #attrs
            #[doc(hidden)]
            pub struct #name #generics {
                #(#fields: #types,)*
            }
        }
    }

    fn expand_input_impl(&self, input_ident: &Ident) -> TokenStream {
        let idents = self.field_idents(true);
        let types = self.field_types(true);

        let auditeur = &*crate::AUDITEUR;
        let inner = crate::proc_macro_impls::input::expand_named_struct(idents, types);
        quote! {
            impl #auditeur::input::Input for #input_ident { #inner }
        }
    }

    fn expand_default(&self, plain_ident: &Ident, input_ident: &Ident) -> TokenStream {
        let auditeur = &*crate::AUDITEUR;
        let ident = self.ident;
        let (impl_generics, type_generics, where_clause) = self.shared.split_generics();

        // okay this should be () but who knows
        let external_input = &self.shared.external_inputs;

        quote! {
            impl #impl_generics #auditeur::resource::CreateDerive for #ident #type_generics
                #where_clause
            {
                async fn create(_: (#(#external_input),*), _: &#input_ident)
                    -> #auditeur::anyhow::Result<#plain_ident>
                {
                    Ok(::std::default::Default::default())
                }
            }
        }
    }
}

struct ConfigFields<'a>(Vec<ConfigField<'a>>);

impl<'a> ConfigFields<'a> {
    fn new(strct: &'a syn::ItemStruct) -> Continue<Self> {
        let mut error = None;
        let fields = (strct.fields.iter())
            .enumerate()
            .map(|(i, field)| ConfigField::new(field, i))
            .map(drain_continues_into(&mut error))
            .collect();
        error.wrap(Self(fields))
    }
}

struct ConfigField<'a> {
    ident: FieldIdent<'a>,
    ty: &'a syn::Type,
    is_input: bool,
}

impl<'a> ConfigField<'a> {
    fn new(field: &'a syn::Field, index: usize) -> Continue<Self> {
        let mut error = None;

        let mut is_input = false;
        for val in crate::attr::collect_attrs::<FieldAttr, FieldAttrs>("create", &field.attrs)
            .filter_map(drain_results_into(&mut error))
        {
            match val {
                FieldAttr::InputFlag => {
                    is_input = true;
                }
            }
        }

        error.wrap(Self {
            ident: field
                .ident
                .as_ref()
                .map_or(FieldIdent::Index(index), FieldIdent::Ident),
            ty: &field.ty,
            is_input,
        })
    }
}

struct Config<'a> {
    auto_impl: AutoImpl,
    external_inputs: Vec<&'a syn::Type>,
    generics: &'a syn::Generics,
}

impl<'a> Config<'a> {
    fn new(attrs: &'a Attrs, generics: &'a syn::Generics) -> Continue<Self> {
        let mut error = None;

        let mut auto_impl = None;
        let mut external_inputs = Vec::new();
        for attr in &attrs.attrs {
            match attr {
                Attr::AutoImplFlag(ident, ty) => {
                    if let Some((old_ident, _)) = auto_impl.replace((ident, ty)) {
                        error.combine(syn::Error::new_spanned(
                            ident,
                            format!("cannot combine '{ident}' with '{old_ident}'"),
                        ));
                        error.combine(syn::Error::new_spanned(
                            old_ident,
                            format!("'{old_ident}' previously declared here"),
                        ));
                    }
                }
                Attr::ExternalInput(types) => external_inputs.extend(types),
            }
        }

        let auto_impl = auto_impl.map_or_else(AutoImpl::default, |(_, ty)| *ty);

        error.wrap(Self {
            auto_impl,
            external_inputs,
            generics,
        })
    }

    fn split_generics(&self) -> (ImplGenerics<'_>, TypeGenerics<'_>, Option<&'_ WhereClause>) {
        self.generics.split_for_impl()
    }
}

#[derive(Default, Clone, Copy)]
enum AutoImpl {
    #[default]
    None,
    Default,
}

impl AutoImpl {
    /// Returns `true` if the auto impl is [`Default`].
    ///
    /// [`Default`]: AutoImpl::Default
    #[must_use]
    const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }
}

#[derive(Clone, Copy)]
enum FieldIdent<'a> {
    Ident(&'a syn::Ident),
    Index(usize),
}

impl ToTokens for FieldIdent<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Ident(ident) => ident.to_tokens(tokens),
            Self::Index(index) => format_ident!("field_{index}").to_tokens(tokens),
        }
    }
}

/// The parsable representation of the arguments to the create macro.
///
/// Turn this an a [`syn::ItemStruct`] to a [`ConfigStruct`].
struct Attrs {
    attrs: Punctuated<Attr, Token![,]>,
}

impl Parse for Attrs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(Attr::parse, Token![,])
            .map(|attrs| Self { attrs })
    }
}

impl IntoIterator for Attrs {
    type IntoIter = syn::punctuated::IntoIter<Attr>;
    type Item = Attr;

    fn into_iter(self) -> Self::IntoIter {
        self.attrs.into_iter()
    }
}

enum Attr {
    AutoImplFlag(Ident, AutoImpl),
    ExternalInput(List<syn::Type>),
}

impl Parse for Attr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "default" {
            Ok(Self::AutoImplFlag(ident, AutoImpl::Default))
        } else if ident == "external_input" {
            List::parse_after(ident, input).map(Self::ExternalInput)
        } else {
            Err(input.error(format!(
                "expected 'default' or 'external_input', found {ident}"
            )))
        }
    }
}

struct FieldAttrs {
    attrs: Punctuated<FieldAttr, Token![,]>,
}

impl Parse for FieldAttrs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(FieldAttr::parse, Token![,])
            .map(|attrs| Self { attrs })
    }
}

impl IntoIterator for FieldAttrs {
    type IntoIter = syn::punctuated::IntoIter<FieldAttr>;
    type Item = FieldAttr;

    fn into_iter(self) -> Self::IntoIter {
        self.attrs.into_iter()
    }
}

enum FieldAttr {
    InputFlag,
}

impl Parse for FieldAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "input" {
            Ok(Self::InputFlag)
        } else {
            Err(input.error(format!("expected 'input', found {ident}")))
        }
    }
}
