use std::sync::LazyLock;

use custom_generics::CustomGenerics;
use namespaced_id::NamespacedId;
use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro_impls::{ComponentCallback, ExpandInputSpecifier, HasInput, IntermediateListener};
use quote::{quote, quote_spanned, ToTokens};
use syn::{
    parse_macro_input, punctuated::Punctuated, spanned::Spanned, Attribute, GenericParam, Ident,
    Item, ItemImpl, ItemStruct, ItemTrait, Token, TypeParamBound,
};

use crate::proc_macro_impls::AllCombinations;

mod custom_generics;
mod field;
mod input_list;
mod namespaced_id;

mod proc_macro_impls {
    mod resource;
    pub use resource::{resource, resource_impls_with_generics};

    mod create;
    pub use create::create;

    mod input;
    pub use input::derive as derive_input;

    mod all_combinations;
    pub use all_combinations::{all_combinations, AllCombinations};

    mod identifiable;
    pub use identifiable::{identifiable, identifiable_with_generics};

    pub mod input_reflection;
    pub use input_reflection::export as export_input;
    pub use input_reflection::ExpandInputSpecifier;
    pub use input_reflection::HasInput;

    mod listener;
    pub use listener::Intermediate as IntermediateListener;
    pub use listener::{runnable_derive, ComponentCallback, Listener};

    #[cfg(feature = "path-any")]
    mod maybe_get_ref;
    #[cfg(feature = "path-any")]
    pub use maybe_get_ref::maybe_get_ref;

    #[cfg(feature = "runtime-loading")]
    mod path_any;
    #[cfg(feature = "runtime-loading")]
    pub use path_any::path_any;
}

fn type_from_ident(ident: syn::Ident) -> syn::Type {
    syn::Type::Path(syn::TypePath {
        qself: None,
        path: syn::Path::from(ident),
    })
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
            proc_macro_crate::FoundCrate::Name(ref name) => {
                Ident::new(name, Span::call_site()).to_tokens(tokens);
            }
        };

        match self.crate_ {
            Crate::Auditeur => (),
            Crate::AuditeurPlugin => quote! { ::auditeur }.to_tokens(tokens),
            Crate::AuditeurPluginHost => {
                quote! { ::auditeur_plugin::auditeur_plugin_host }.to_tokens(tokens);
            }
        }
    }
}

fn get_crate(name: &str) -> Option<proc_macro2::TokenStream> {
    let core = proc_macro_crate::crate_name(name).ok()?;

    let stream = match core {
        proc_macro_crate::FoundCrate::Itself => quote!(crate),
        proc_macro_crate::FoundCrate::Name(name) => {
            let name = Ident::new(&name, Span::call_site());
            quote!( #name )
        }
    };

    Some(stream)
}

// TODO: replace with Root
struct Auditeur {
    root: proc_macro2::TokenStream,
}

impl Auditeur {
    pub fn new() -> Self {
        let root = get_crate("auditeur")
            // fallback to auditeur-plugin, if it exists
            .or_else(|| get_crate("auditeur-plugin").map(|auditeur_plugin| quote! { #auditeur_plugin::auditeur }))
            .or_else(|| get_crate("auditeur-plugin-host").map(|auditeur_plugin_host| quote! { #auditeur_plugin_host::auditeur_plugin::auditeur }))
            .expect("auditeur or auditeur_plugin should be in Cargo.toml");

        Self { root }
    }

    pub const fn crate_name(&self) -> &proc_macro2::TokenStream {
        &self.root
    }

    pub fn id(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::namespaced_id }
    }

    pub fn resource(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::resource }
    }

    pub fn input(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::input }
    }

    pub fn anyhow(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::anyhow }
    }

    #[cfg(feature = "path-any")]
    pub fn path_any(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::path_any }
    }
}

fn extract_attr(vec: &mut Vec<Attribute>, ident: &str) -> impl Iterator<Item = Attribute> {
    let (matches, others) = std::mem::take(vec)
        .into_iter()
        .partition(|attr| attr.path().is_ident(ident));
    *vec = others;
    matches.into_iter()
}

#[proc_macro_attribute]
pub fn resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    proc_macro_impls::resource(item, &id)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn identifiable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    let identifiable_impl = match proc_macro_impls::identifiable(&id, &mut item) {
        Ok(identifiable_impl) => identifiable_impl,
        Err(err) => return err.into_compile_error().into(),
    };

    quote! {
        #identifiable_impl
        #item
    }
    .into()
}

#[proc_macro_attribute]
pub fn message(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(item as Item);
    let id = parse_macro_input!(attr as NamespacedId);

    let (ident, generics) = match CustomGenerics::from_item(&mut item) {
        Ok(tuple) => tuple,
        Err(err) => return err.into_compile_error().into(),
    };

    let auditeur = Auditeur::new();

    let identifiable_impl = proc_macro_impls::identifiable_with_generics(&id, ident, &generics);

    #[cfg(feature = "path-any")]
    let path_any_impl = proc_macro_impls::path_any(ident, &generics, &auditeur);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    let crate_name = auditeur.crate_name();

    quote! {
        impl #crate_name::Message for #ident {}

        #item
        #identifiable_impl
        #path_any_impl
    }
    .into()
}

#[proc_macro]
pub fn ident(string: TokenStream) -> TokenStream {
    let id = parse_macro_input!(string as NamespacedId);

    id.to_token_stream().into()
}

#[proc_macro_attribute]
pub fn create(_: TokenStream, item: TokenStream) -> TokenStream {
    let implementation = parse_macro_input!(item as ItemImpl);
    proc_macro_impls::create(implementation).into()
}

#[proc_macro_derive(Create)]
pub fn create_derive(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let ident = match item {
        Item::Struct(strct) => strct.ident,
        Item::Enum(enm) => enm.ident,
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let auditeur = Auditeur::new();
    let resource_mod = auditeur.resource();
    let anyhow = auditeur.anyhow();

    quote! {
        // we can be generic over message because there are no inputs to depend on it
        impl #resource_mod::Create for #ident {
            type RegistryInput = ();
            type ExternalInput = ();
            async fn create((): (), (): ()) -> #anyhow::Result<Self> {
                Ok(Self::default())
            }
        }
    }
    .into()
}

#[proc_macro_derive(Input, attributes(custom_generics))]
pub fn derive_input(item: TokenStream) -> TokenStream {
    let strct = parse_macro_input!(item as ItemStruct);
    proc_macro_impls::derive_input(strct).into()
}

#[proc_macro_derive(GetRef)]
pub fn get_ref(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let ident = match item {
        Item::Struct(strct) => strct.ident,
        Item::Enum(enm) => enm.ident,
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let auditeur = Auditeur::new();
    let input_mod = auditeur.input();

    quote! {
        impl #input_mod::get::GetRef for #ident {
            type Ref = Self;
            fn get_ref(&self) -> Self {
                self.clone()
            }
        }
    }
    .into()
}

#[proc_macro_derive(MaybeGetRef)]
#[cfg(feature = "runtime-loading")]
pub fn maybe_get_ref(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let (ident, generics, attrs) = match item {
        Item::Struct(strct) => (strct.ident, strct.generics, strct.attrs),
        Item::Enum(enm) => (enm.ident, enm.generics, enm.attrs),
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let generics = CustomGenerics::from_attr_or_generics(&attrs, &generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error().into(),
    };

    proc_macro_impls::maybe_get_ref(&ident, &generics, &Auditeur::new()).into()
}

#[cfg(feature = "path-any")]
#[proc_macro_derive(PathAny, attributes(custom_generics))]
pub fn path_any(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);

    let (ident, generics, attrs) = match item {
        Item::Struct(strct) => (strct.ident, strct.generics, strct.attrs),
        Item::Enum(enm) => (enm.ident, enm.generics, enm.attrs),
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let generics = CustomGenerics::from_attr_or_generics(&attrs, &generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error().into(),
    };

    let auditeur = Auditeur::new();
    proc_macro_impls::path_any(&ident, &generics, &auditeur).into()
}

#[cfg(feature = "path-any")]
#[proc_macro]
pub fn implement_path_any_external(ident: TokenStream) -> TokenStream {
    let PathAnyExternal { ident, generics } = parse_macro_input!(ident as PathAnyExternal);

    let auditeur = Auditeur::new();
    // TODO: ensure that there are no name collisions
    proc_macro_impls::path_any(&ident, &CustomGenerics::from_generics(&generics), &auditeur).into()
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

#[derive(Default)]
struct GenericArgList {
    params: Option<Punctuated<GenericParam, Token![,]>>,
}

impl GenericArgList {
    #[cfg(feature = "path-any")]
    pub fn iter(&self) -> impl Iterator<Item = &GenericParam> {
        self.params
            .as_ref()
            .map(Punctuated::iter)
            .into_iter()
            .flatten()
    }
}

impl ToTokens for GenericArgList {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        if let Some(params) = &self.params {
            tokens.extend(std::iter::once(quote!(<)));
            params.to_tokens(tokens);
            tokens.extend(std::iter::once(quote!(>)));
        }
    }
}

impl syn::parse::Parse for GenericArgList {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { params: None });
        }

        let _: Token![<] = input.parse()?;
        let mut params = Punctuated::new();
        while !input.peek(Token![>]) {
            params.push(input.parse()?);
            if input.peek(Token![>]) {
                break;
            }
            params.push_punct(input.parse()?);
        }
        let _: Token![>] = input.parse()?;
        Ok(Self {
            params: Some(params),
        })
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

#[proc_macro]
pub fn expand_input_specifier(tokens: TokenStream) -> TokenStream {
    let config = parse_macro_input!(tokens as ExpandInputSpecifier);
    config
        .expand()
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn listener(attr: TokenStream, item: TokenStream) -> TokenStream {
    let listener = parse_macro_input!(attr as proc_macro_impls::Listener);
    let item = parse_macro_input!(item as Item);

    let ident = match &item {
        Item::Struct(strct) => &strct.ident,
        Item::Enum(enm) => &enm.ident,
        _ => {
            return syn::Error::new_spanned(item, "expected a struct or enum")
                .into_compile_error()
                .into()
        }
    };

    let impls = listener
        .expand(ident.clone())
        .unwrap_or_else(syn::Error::into_compile_error);

    quote! {
        #item
        #impls
    }
    .into()
}

#[proc_macro]
pub fn expand_listener(value: TokenStream) -> TokenStream {
    let intermediate = parse_macro_input!(value as IntermediateListener);
    intermediate
        .expand()
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn has_input(attr: TokenStream, item: TokenStream) -> TokenStream {
    HasInput::parse_entire(item, attr)
        .and_then(HasInput::expand)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn listener_component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let component_map = parse_macro_input!(attr as ComponentCallback);
    let item = parse_macro_input!(item as Item);

    let ident = match &item {
        Item::Struct(strct) => &strct.ident,
        Item::Enum(enm) => &enm.ident,
        _ => {
            return syn::Error::new_spanned(item, "expected a struct or enum")
                .into_compile_error()
                .into()
        }
    };

    let macro_impl = component_map.expand(ident);

    quote! {
        #item
        #macro_impl
    }
    .into()
}

#[proc_macro_derive(Runnable, attributes(auditeur_runnable))]
pub fn runnable_derive(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as Item);
    let (ident, attrs) = match item {
        Item::Struct(strct) => (strct.ident, strct.attrs),
        Item::Enum(enm) => (enm.ident, enm.attrs),
        _ => {
            return quote_spanned! {
                item.span() => compile_error!("item must be a struct or enum"),
            }
            .into();
        }
    };

    let args = attrs
        .into_iter()
        .map(|attr| attr.meta)
        .filter(|meta| meta.path().is_ident("auditeur_runnable"))
        .last()
        .map_or_else(
            || Ok(proc_macro_impls::input_reflection::HasInputArgs::default()),
            |meta| {
                if let syn::Meta::List(list) = meta {
                    syn::parse2(list.tokens)
                } else {
                    Err(syn::Error::new_spanned(meta, "expected parens"))
                }
            },
        );

    args.and_then(|args| proc_macro_impls::runnable_derive(&crate::type_from_ident(ident), &args))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
