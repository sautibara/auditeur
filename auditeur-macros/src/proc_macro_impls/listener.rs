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

#[expect(clippy::struct_excessive_bools)]
pub struct Config<'a> {
    stop_component: bool,
    biased: bool,
    weak: bool,
    options: bool,
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

        // `weak`'s default is `false`.
        let weak = attrs.attrs.iter().any(Attr::is_weak_flag);

        // `options`'s default is `false`.
        let options = attrs.attrs.iter().any(Attr::is_options_flag);

        Self {
            stop_component,
            biased,
            weak,
            options,
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
        let options = self.options();

        let get_runner_with = super::get_runner_with(
            ident,
            self.generics,
            &quote! {
                #auditeur::component::resource::listener::ListenerMethod
            },
        );

        let listener_derives = self.options.then(|| {
            let this = self.as_listener_derive();
            quote! {
                async fn init(&mut self) -> #auditeur::anyhow::Result<()> {
                    #this::init(self).await
                }

                async fn exit(&mut self) -> #auditeur::anyhow::Result<()> {
                    #this::exit(self).await
                }

                fn children(children: #auditeur::children::Children)
                    -> #auditeur::children::Children
                {
                    #this::children(children)
                }
            }
        });

        quote! {
            impl #impl_generics #auditeur::component::resource::listener::Listener
                for #ident #type_generics
            #where_clause
            {
                type Streams = (#bias, #(#components),*);
                #listener_derives
                const OPTIONS: #auditeur::component::resource::Options = #options;
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
            .then(|| quote! { #auditeur::component::resource::listener::StopStream })
    }

    fn bias(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;
        if self.biased {
            quote! { #auditeur::component::resource::listener::Biased }
        } else {
            quote! { #auditeur::component::resource::listener::Unbiased }
        }
    }

    fn as_listener_derive(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;
        let ident = self.ident;
        let (_, ty_generics, _) = self.generics.split_for_impl();
        quote! {
            <#ident #ty_generics as #auditeur::component::resource::listener::ListenerDerive>
        }
    }

    fn options(&self) -> proc_macro2::TokenStream {
        let auditeur = &*AUDITEUR;
        let mut options = quote! { #auditeur::component::resource::Options::new() };

        if self.weak {
            options = quote! { #options.weak(true) };
        }

        if self.options {
            let this = self.as_listener_derive();
            options = quote! {  #this::OPTIONS.overlay_on(#options) };
        }

        options
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
    OnAddIn(field::List<Component>),
    StopComponent(field::NameValue<syn::LitBool>),
    Component(field::List<Component>),
    WeakFlag,
    RespondFlag,
    BiasedFlag,
    OptionsFlag,
}

impl Attr {
    fn components(&self) -> impl Iterator<Item = proc_macro2::TokenStream> {
        match self {
            Self::RespondFlag => Either::Left(Either::Right(std::iter::once(simple_component(
                "RespondStream",
            )))),
            Self::Listen(messages) => Either::Right(Either::Left(Either::Left(
                messages.iter().map(Component::component::<ListenSupplier>),
            ))),
            Self::Take(messages) => Either::Right(Either::Left(Either::Right(
                messages.iter().map(Component::component::<TakeSupplier>),
            ))),
            Self::OnAddIn(messages) => Either::Right(Either::Right(Either::Left(
                messages.iter().map(Component::component::<OnAddInSupplier>),
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

    /// Returns `true` if the attr is [`WeakFlag`].
    ///
    /// [`WeakFlag`]: Attr::WeakFlag
    #[must_use]
    const fn is_weak_flag(&self) -> bool {
        matches!(self, Self::WeakFlag)
    }

    /// Returns `true` if the attr is [`OptionsFlag`].
    ///
    /// [`OptionsFlag`]: Attr::OptionsFlag
    #[must_use]
    const fn is_options_flag(&self) -> bool {
        matches!(self, Self::OptionsFlag)
    }
}

impl Parse for Attr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "listen" {
            field::List::parse_after(ident, input).map(Self::Listen)
        } else if ident == "take" {
            field::List::parse_after(ident, input).map(Self::Take)
        } else if ident == "on_add_in" {
            field::List::parse_after(ident, input).map(Self::OnAddIn)
        } else if ident == "respond" {
            Ok(Self::RespondFlag)
        } else if ident == "streams" {
            field::List::parse_after(ident, input).map(Self::Component)
        } else if ident == "stop_stream" {
            field::NameValue::parse_after(ident, input).map(Self::StopComponent)
        } else if ident == "biased" {
            Ok(Self::BiasedFlag)
        } else if ident == "weak" {
            Ok(Self::WeakFlag)
        } else if ident == "options" {
            Ok(Self::OptionsFlag)
        } else {
            Err(input.error(format!(
                "expected 'biased', 'weak', 'options', 'stop_stream', 'listen', 'take', 'on_add_in', or 'streams', found '{ident}'"
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
        default_component("ListenStream", ty)
    }
}

struct TakeSupplier;
impl ComponentSupplier for TakeSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream {
        default_component("TakeStream", ty)
    }
}

struct OnAddInSupplier;
impl ComponentSupplier for OnAddInSupplier {
    fn supply_component(ty: &syn::Type) -> proc_macro2::TokenStream {
        default_component("OnAddInStream", ty)
    }
}

fn default_component(component: &str, ty: &syn::Type) -> proc_macro2::TokenStream {
    let component = Ident::new(component, ty.span());

    let auditeur = &*AUDITEUR;
    quote! { #auditeur::component::resource::listener::#component<#ty> }
}

fn simple_component(component: &str) -> proc_macro2::TokenStream {
    let component = Ident::new(component, Span::call_site());

    let auditeur = &*AUDITEUR;
    quote! { #auditeur::component::resource::listener::#component<Self> }
}
