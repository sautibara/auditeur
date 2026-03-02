// #[listener(
//     "core:walker",
//     listen(message::Queue, message::Audio),
//     respond(...), // not implemented yet, same as listen though
//     registry_component = true,
//     components(ListenComponent<Self, message::Queue>),
// )]

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{Ident, Token, parse::Parse, punctuated::Punctuated, spanned::Spanned};

use crate::{AUDITEUR, Either, field};

pub fn listener_attr(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let item: syn::Item = syn::parse2(item)?;
    let attr: Attrs = syn::parse2(attr)?;

    let (ident, generics) = match &item {
        syn::Item::Struct(strct) => Ok((&strct.ident, &strct.generics)),
        syn::Item::Enum(enm) => Ok((&enm.ident, &enm.generics)),
        _ => Err(syn::Error::new_spanned(&item, "expected struct or enum")),
    }?;

    let config = Config::new(&attr, ident, generics);
    let listener_impl = config.expand();

    Ok(quote! {
        #item
        #listener_impl
    })
}

pub struct Config<'a> {
    registry_component: bool,
    biased: bool,
    attrs: &'a Attrs,
    ident: &'a Ident,
    generics: &'a syn::Generics,
}

impl<'a> Config<'a> {
    fn new(attrs: &'a Attrs, ident: &'a Ident, generics: &'a syn::Generics) -> Self {
        let registry_component = (attrs.attrs.iter())
            .filter_map(Attr::as_registry_component)
            .last()
            .map(|comp| comp.value.value)
            // `registry_component`'s default is `true`.
            .unwrap_or(true);

        // `biased`'s default is `false`.
        let biased = attrs.attrs.iter().any(Attr::is_biased_flag);

        Self {
            registry_component,
            biased,
            attrs,
            ident,
            generics,
        }
    }

    fn expand(&self) -> proc_macro2::TokenStream {
        let ident = self.ident;
        let (impl_generics, type_generics, where_clause) = self.generics.split_for_impl();

        let bias = self.bias();
        let components = self.components();

        let component_len = self.components().count();
        let combinations = std::iter::repeat(quote! { () }).take(component_len);

        let auditeur = &*AUDITEUR;
        quote! {
            impl #impl_generics #auditeur::resource::listener::Listener for #ident #type_generics
            #where_clause
            {
                type Input = ();
                type Components = (#bias, #(#components),*);
                type ComponentCombinations = ( #(#combinations),* );
            }
        }
    }

    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        self.registry_component()
            .into_iter()
            .chain(self.attrs.components())
    }

    fn registry_component(&self) -> Option<proc_macro2::TokenStream> {
        let auditeur = &*AUDITEUR;
        self.registry_component
            .then(|| quote! { #auditeur::resource::listener::RegistryComponent })
    }

    fn bias(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;
        if self.biased {
            quote! { #auditeur::resource::listener::Biased }
        } else {
            quote! { #auditeur::resource::listener::Unbiased }
        }
    }
}

/// The parsable representation of the arguments to the listener macro.
///
/// Turn this and a [`syn::ItemStruct`] to a [`ConfigStruct`].
struct Attrs {
    attrs: Punctuated<Attr, Token![,]>,
}

impl Attrs {
    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        self.attrs.iter().flat_map(Attr::components)
    }
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
    Listen(field::List<Component>),
    Take(field::List<Component>),
    RegistryComponent(field::NameValue<syn::LitBool>),
    Component(field::List<Component>),
    RespondFlag,
    ListenToRegistryFlag,
    BiasedFlag,
}

impl Attr {
    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        match self {
            Self::RespondFlag => Either::Left(Either::Right(std::iter::once(simple_component(
                "RespondComponent",
            )))),
            Self::ListenToRegistryFlag => Either::Left(Either::Right(std::iter::once(
                simple_component("ListenToRegistryComponent"),
            ))),
            Self::Listen(messages) => Either::Right(Either::Left(
                messages.iter().map(Component::component::<ListenSupplier>),
            )),
            Self::Take(messages) => Either::Right(Either::Right(Either::Left(
                messages.iter().map(Component::component::<TakeSupplier>),
            ))),
            Self::Component(messages) => Either::Right(Either::Right(Either::Right(
                messages
                    .iter()
                    .map(Component::component::<VerbatimSupplier>),
            ))),
            _ => Either::Left(Either::Left(std::iter::empty())),
        }
    }

    fn as_registry_component(&self) -> Option<&field::NameValue<syn::LitBool>> {
        if let Self::RegistryComponent(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns `true` if the attr is [`BiasedFlag`].
    ///
    /// [`BiasedFlag`]: Attr::BiasedFlag
    #[must_use]
    fn is_biased_flag(&self) -> bool {
        matches!(self, Self::BiasedFlag)
    }
}

impl Parse for Attr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "listen" {
            field::List::parse_after(ident, input).map(Self::Listen)
        } else if ident == "take" {
            field::List::parse_after(ident, input).map(Self::Take)
        } else if ident == "respond" {
            Ok(Self::RespondFlag)
        } else if ident == "components" {
            field::List::parse_after(ident, input).map(Self::Component)
        } else if ident == "registry_component" {
            field::NameValue::parse_after(ident, input).map(Self::RegistryComponent)
        } else if ident == "listen_to_registry" {
            Ok(Self::ListenToRegistryFlag)
        } else if ident == "biased" {
            Ok(Self::BiasedFlag)
        } else {
            Err(input.error(format!(
                "expected 'registry_component', 'biased', 'listen', 'take', 'listen_to_registry', or 'components', found '{ident}'"
            )))
        }
    }
}

trait ComponentSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream;
}

struct Component(syn::Type);

impl Component {
    fn component<C: ComponentSupplier>(&self) -> proc_macro2::TokenStream {
        C::supply_component(&self.0)
    }
}

impl Parse for Component {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse().map(Self)
    }
}

struct VerbatimSupplier;
impl ComponentSupplier for VerbatimSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream {
        ty.to_token_stream()
    }
}

struct ListenSupplier;
impl ComponentSupplier for ListenSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream {
        default_component("ListenComponent", ty)
    }
}

struct TakeSupplier;
impl ComponentSupplier for TakeSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream {
        default_component("TakeComponent", ty)
    }
}

fn default_component(component: &str, ty: &syn::Type) -> proc_macro2::TokenStream {
    let component = Ident::new(component, ty.span());

    let auditeur = &*AUDITEUR;
    quote! { #auditeur::resource::listener::#component<Self, #ty> }
}

fn simple_component(component: &str) -> proc_macro2::TokenStream {
    let component = Ident::new(component, Span::call_site());

    let auditeur = &*AUDITEUR;
    quote! { #auditeur::resource::listener::#component<Self> }
}
