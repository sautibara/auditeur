use std::sync::LazyLock;

use namespaced_id::NamespacedId;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{ToTokens, quote, quote_spanned};
use syn::{
    Ident, Item, ItemStruct, ItemTrait, Token, TypeParamBound, parse_macro_input, spanned::Spanned,
};

use crate::proc_macro_impls::AllCombinations;

mod attr;
mod field;
mod maybe_error;
mod namespaced_id;
mod overlay;

fn snake_to_upper_camel_case(snake: &str) -> String {
    snake.split('_').fold(String::new(), |mut string, slice| {
        let mut chars = slice.chars();
        if let Some(first) = chars.next() {
            string.extend(first.to_uppercase());
        }
        string.extend(chars);
        string
    })
}

fn extract_item_values(item: &Item) -> syn::Result<(&Ident, &syn::Generics, &[syn::Attribute])> {
    match item {
        Item::Struct(item) => Ok((&item.ident, &item.generics, &item.attrs)),
        Item::Enum(item) => Ok((&item.ident, &item.generics, &item.attrs)),
        _ => Err(syn::Error::new_spanned(item, "expected a struct or enum")),
    }
}

enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<I, L: Iterator<Item = I>, R: Iterator<Item = I>> Iterator for Either<L, R> {
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(left) => left.next(),
            Self::Right(right) => right.next(),
        }
    }
}

mod proc_macro_impls {
    mod resource;
    pub use resource::{get_runner_with, resource};

    pub mod input;
    pub use input::derive as derive_input;

    mod all_combinations;
    pub use all_combinations::{AllCombinations, all_combinations};

    mod identifiable;
    pub use identifiable::{component, identified};

    mod listener;
    pub use listener::listener_attr;

    mod respond;
    pub use respond::respond_attr;

    mod message;
    pub use message::{derive_listen_message, derive_take_message};

    mod maybe_get_ref;
    pub use maybe_get_ref::maybe_get_ref;

    mod clone_as;
    pub use clone_as::clone_as_enum;
    pub use clone_as::clone_as_struct;

    mod path_any;
    pub use path_any::any;

    mod create;
    pub use create::create_attr;
}

static AUDITEUR: LazyLock<Root> = LazyLock::new(Root::new);

struct Root {
    crate_name: proc_macro_crate::FoundCrate,
    crate_: Crate,
}

enum Crate {
    Auditeur,
    AuditeurPlugin,
    AuditeurPluginHost,
}

#[expect(dead_code)]
impl Root {
    fn new() -> Self {
        (proc_macro_crate::crate_name("auditeur").map(|crate_name| Self {
            crate_name,
            crate_: Crate::Auditeur,
        }))
        .or_else(|_| {
            proc_macro_crate::crate_name("auditeur-plugin").map(|crate_name| Self {
                crate_name,
                crate_: Crate::AuditeurPlugin,
            })
        })
        .or_else(|_| {
            proc_macro_crate::crate_name("auditeur-plugin-host").map(|crate_name| Self {
                crate_name,
                crate_: Crate::AuditeurPluginHost,
            })
        })
        .expect("expected auditeur, auditeur-plugin, or auditeur-plugin-host to be in Cargo.toml")
    }

    pub fn id(&self) -> proc_macro2::TokenStream {
        quote! { #self::namespaced_id }
    }

    pub fn resource(&self) -> proc_macro2::TokenStream {
        quote! { #self::resource }
    }

    pub fn input(&self) -> proc_macro2::TokenStream {
        quote! { #self::input }
    }

    pub fn anyhow(&self) -> proc_macro2::TokenStream {
        quote! { #self::anyhow }
    }

    #[cfg(feature = "path-any")]
    pub fn path_any(&self) -> proc_macro2::TokenStream {
        quote! { #self::path_any }
    }
}

impl ToTokens for Root {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match &self.crate_name {
            proc_macro_crate::FoundCrate::Itself => quote!(crate).to_tokens(tokens),
            proc_macro_crate::FoundCrate::Name(name) => {
                Ident::new(name, Span::call_site()).to_tokens(tokens);
            }
        }

        match self.crate_ {
            Crate::Auditeur => (),
            Crate::AuditeurPlugin => quote! { ::auditeur }.to_tokens(tokens),
            Crate::AuditeurPluginHost => {
                quote! { ::auditeur_plugin::auditeur }.to_tokens(tokens);
            }
        }
    }
}

#[proc_macro_attribute]
pub fn resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    proc_macro_impls::resource(&item, &id)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn pure_resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    if !attr.is_empty() {
        return syn::Error::new_spanned(
            proc_macro2::TokenStream::from(attr),
            "attribute should be empty",
        )
        .into_compile_error()
        .into();
    }

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    let auditeur = &*crate::AUDITEUR;
    let method = quote! {
        #auditeur::component::resource::pure::PureMethod
    };

    let get_runner_with = proc_macro_impls::get_runner_with(ident, generics, &method);
    quote! {
        #get_runner_with
        #item
    }
    .into()
}

#[proc_macro_attribute]
pub fn identified(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    let identifiable_impl = proc_macro_impls::identified(&id, ident, generics);
    quote! {
        #identifiable_impl
        #item
    }
    .into()
}

#[proc_macro_attribute]
pub fn listen_message(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    let implementation = proc_macro_impls::derive_listen_message(ident, &id, generics);

    quote! {
        #item
        #implementation
    }
    .into()
}

#[proc_macro_attribute]
pub fn take_message(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    let implementation = proc_macro_impls::derive_take_message(ident, &id, generics);

    quote! {
        #item
        #implementation
    }
    .into()
}

#[proc_macro_attribute]
pub fn create(attr: TokenStream, item: TokenStream) -> TokenStream {
    proc_macro_impls::create_attr(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro]
pub fn ident(string: TokenStream) -> TokenStream {
    let id = parse_macro_input!(string as NamespacedId);

    id.to_token_stream().into()
}

#[proc_macro_derive(Input, attributes(custom_generics))]
pub fn derive_input(item: TokenStream) -> TokenStream {
    let strct = parse_macro_input!(item as ItemStruct);
    proc_macro_impls::derive_input(&strct).into()
}

#[proc_macro_derive(GetRef)]
pub fn get_ref(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let (clone_as, ident) = match item {
        Item::Struct(strct) => (
            proc_macro_impls::clone_as_struct(
                &strct.ident,
                strct.fields.iter().filter_map(|field| field.ident.as_ref()),
                &strct.generics,
            ),
            strct.ident,
        ),
        Item::Enum(enm) => (proc_macro_impls::clone_as_enum(&enm), enm.ident),
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let auditeur = &*AUDITEUR;

    quote! {
        #clone_as

        impl #auditeur::input::get::GetRef for #ident {
            type Ref = Self;
            async fn get_ref(
                &self,
                _: &#auditeur::app::Registry,
                dependent: #auditeur::app::ComponentIdRef<'_>,
            ) -> Result<Self, #auditeur::Error> {
                Ok(<Self as #auditeur::input::CloneAs>::clone_as(self, dependent))
            }
        }
    }
    .into()
}

#[proc_macro_derive(MaybeGetRef)]
#[cfg(feature = "runtime-loading")]
pub fn maybe_get_ref(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    proc_macro_impls::maybe_get_ref(ident, generics).into()
}

#[proc_macro_derive(CloneIn)]
pub fn clone_in(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemStruct);

    let fields = item.fields.iter().filter_map(|field| field.ident.as_ref());
    let generics = &item.generics;

    proc_macro_impls::clone_as_struct(&item.ident, fields, generics).into()
}

#[proc_macro_derive(Any)]
pub fn any(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let (ident, generics, _) = match extract_item_values(&item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    proc_macro_impls::any(ident, generics).into()
}

#[cfg(feature = "path-any")]
#[proc_macro]
pub fn implement_path_any_external(ident: TokenStream) -> TokenStream {
    let PathAnyExternal { ident, generics } = parse_macro_input!(ident as PathAnyExternal);

    // TODO: ensure that there are no name collisions
    proc_macro_impls::any(&ident, &generics).into()
}

#[cfg(feature = "path-any")]
struct PathAnyExternal {
    ident: Ident,
    generics: syn::Generics,
}

#[cfg(feature = "path-any")]
impl syn::parse::Parse for PathAnyExternal {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident = input.parse()?;
        let generics = if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            input.parse()?
        } else {
            syn::Generics::default()
        };
        Ok(Self { ident, generics })
    }
}

#[proc_macro_attribute]
pub fn cfg_supertrait(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut item_trait = parse_macro_input!(item as ItemTrait);
    let CfgSupertrait { cfg, typ, default } = parse_macro_input!(attr as CfgSupertrait);

    item_trait.supertraits.push(typ);
    let with_trait = quote! {
        #[cfg(#cfg)]
        #item_trait
    };
    item_trait.supertraits.pop();
    item_trait.supertraits.pop_punct();
    if let Some(typ) = default {
        item_trait.supertraits.push(typ);
    }
    quote! {
        #with_trait
        #[cfg(not(#cfg))]
        #item_trait
    }
    .into()
}

struct CfgSupertrait {
    cfg: syn::Meta,
    typ: TypeParamBound,
    default: Option<TypeParamBound>,
}

impl syn::parse::Parse for CfgSupertrait {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let cfg = input.parse()?;
        let _: Token![,] = input.parse()?;
        let typ = input.parse()?;
        let default = if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self { cfg, typ, default })
    }
}

#[proc_macro]
pub fn all_combinations(params: TokenStream) -> TokenStream {
    let params = parse_macro_input!(params as AllCombinations);
    proc_macro_impls::all_combinations(&params)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn listener(attr: TokenStream, item: TokenStream) -> TokenStream {
    proc_macro_impls::listener_attr(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn watcher(attr: TokenStream, item: TokenStream) -> TokenStream {
    fn watcher_attr(attr: TokenStream, item: TokenStream) -> syn::Result<proc_macro2::TokenStream> {
        let auditeur = &*crate::AUDITEUR;

        if let Some(token) = attr.into_iter().next() {
            return Err(syn::Error::new(
                token.span().into(),
                "expected attribute to be empty",
            ));
        }

        let item: Item = syn::parse(item)?;
        let (ident, generics, _) = extract_item_values(&item)?;

        Ok(proc_macro_impls::get_runner_with(
            ident,
            generics,
            &quote! {
                #auditeur::component::resource::watcher::WatcherMethod
            },
        ))
    }

    watcher_attr(attr, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn respond(attr: TokenStream, item: TokenStream) -> TokenStream {
    proc_macro_impls::respond_attr(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
