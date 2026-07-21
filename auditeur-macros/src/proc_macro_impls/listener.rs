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
    stop_component: bool,
    biased: bool,
    attrs: &'a Attrs,
    ident: &'a Ident,
    generics: &'a syn::Generics,
}

impl<'a> Config<'a> {
    fn new(attrs: &'a Attrs, ident: &'a Ident, generics: &'a syn::Generics) -> Self {
        let stop_component = (attrs.attrs.iter())
            .filter_map(Attr::as_stop_component)
            .next_back()
            // `registry_component`'s default is `true`.
            .is_none_or(|comp| comp.value.value);

        // `biased`'s default is `false`.
        let biased = attrs.attrs.iter().any(Attr::is_biased_flag);

        Self {
            stop_component,
            biased,
            attrs,
            ident,
            generics,
        }
    }

    fn expand(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;

        let ident = self.ident;
        let (impl_generics, type_generics, where_clause) = self.generics.split_for_impl();

        let bias = self.bias();
        let components = self.components();

        let get_runner_with = super::get_runner_with(
            ident,
            self.generics,
            &quote! {
                #auditeur::component::resource::listener::ListenerMethod
            },
        );

        quote! {
            impl #impl_generics #auditeur::component::resource::listener::Listener
                for #ident #type_generics
            #where_clause
            {
                type Components = (#bias, #(#components),*);

                async fn init(&mut self) -> #auditeur::anyhow::Result<()> {
                    #auditeur::init_specialization!(self).await
                }

                async fn exit(&mut self) -> #auditeur::anyhow::Result<()> {
                    #auditeur::exit_specialization!(self).await
                }
            }

            #get_runner_with
        }
    }

    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        self.stop_component()
            .into_iter()
            .chain(self.attrs.components())
    }

    fn stop_component(&self) -> Option<proc_macro2::TokenStream> {
        let auditeur = &*AUDITEUR;
        self.stop_component
            .then(|| quote! { #auditeur::component::resource::listener::StopComponent })
    }

    fn bias(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;
        if self.biased {
            quote! { #auditeur::component::resource::listener::Biased }
        } else {
            quote! { #auditeur::component::resource::listener::Unbiased }
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
    StopComponent(field::NameValue<syn::LitBool>),
    Component(field::List<Component>),
    RespondFlag,
    StopComponentFlag,
    BiasedFlag,
}

impl Attr {
    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        match self {
            Self::RespondFlag => Either::Left(Either::Right(std::iter::once(simple_component(
                "RespondComponent",
            )))),
            Self::StopComponentFlag => Either::Left(Either::Right(std::iter::once(
                simple_component("StopComponent"),
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

    const fn as_stop_component(&self) -> Option<&field::NameValue<syn::LitBool>> {
        if let Self::StopComponent(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns `true` if the attr is [`BiasedFlag`].
    ///
    /// [`BiasedFlag`]: Attr::BiasedFlag
    #[must_use]
    const fn is_biased_flag(&self) -> bool {
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
            field::NameValue::parse_after(ident, input).map(Self::StopComponent)
        } else if ident == "stop_component" {
            Ok(Self::StopComponentFlag)
        } else if ident == "biased" {
            Ok(Self::BiasedFlag)
        } else {
            Err(input.error(format!(
                "expected 'registry_component', 'biased', 'listen', 'take', 'stop_component', or 'components', found '{ident}'"
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
    quote! { #auditeur::component::resource::listener::#component<Self, #ty> }
}

fn simple_component(component: &str) -> proc_macro2::TokenStream {
    let component = Ident::new(component, Span::call_site());

    let auditeur = &*AUDITEUR;
    quote! { #auditeur::component::resource::listener::#component<Self> }
}
