use namespaced_id::NamespacedId;
use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro_crate::crate_name;
use quote::{quote, quote_spanned, ToTokens};
use syn::{Ident, Item, spanned::Spanned, parenthesized, Token, GenericParam, WhereClause, TypeParamBound, parse_macro_input, ItemStruct, LitStr, Attribute, Expr, ExprLit, Lit, punctuated::Punctuated, ItemTrait, Generics};

mod namespaced_id;

fn get_crate(name: &str) -> Option<proc_macro2::TokenStream> {
    let core = crate_name(name).ok()?;

    let stream = match core {
        proc_macro_crate::FoundCrate::Itself => quote!(crate),
        proc_macro_crate::FoundCrate::Name(name) => {
            let name = Ident::new(&name, Span::call_site());
            quote!( #name )
        }
    };

    Some(stream)
}

struct Auditeur {
    root: proc_macro2::TokenStream
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

    pub fn path_any(&self) -> proc_macro2::TokenStream {
        let root = &self.root;
        quote! { #root::path_any }
    }
}

#[proc_macro_attribute]
pub fn resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(item as ItemStruct);
    let ident = item.ident.clone();

    let string = parse_macro_input!(attr as LitStr);
    let id = string.value().parse();

    let auditeur = Auditeur::new();
    let id_mod = auditeur.id();
    let resource_mod = auditeur.resource();

    let generics = CustomGenerics::from_attr_or_generics(&item.attrs, &item.generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error().into(),
    };

    item.attrs.retain(|attr| !CustomGenerics::attr_matches(attr));

    #[cfg(feature = "path-any")]
    let path_any_impl = implement_path_any(&ident, &generics, auditeur);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    let CustomGenerics { impl_generics, type_generics, where_clause } = generics;

    match id {
        Ok(NamespacedId { namespace, id }) => quote!(
            impl #impl_generics #resource_mod::Resource for #ident #type_generics #where_clause {
                fn ident(&self) -> #id_mod::NamespacedIdRef<'static> {
                    #id_mod::NamespacedIdRef {
                        namespace: #namespace,
                        id: #id,
                    }
                }

                fn const_ident() -> #id_mod::NamespacedIdRef<'static>
                where
                    Self: Sized
                {
                    #id_mod::NamespacedIdRef {
                        namespace: #namespace,
                        id: #id,
                    }
                }
            }

            #path_any_impl

            #item
        ),
        Err(err) => {
            let err = err.to_string();
            quote_spanned!(string.span() => 
                compile_error!(concat!("failed to parse ident: ", #err));)
        }
    }
    .into()
}

#[proc_macro]
pub fn ident(string: TokenStream) -> TokenStream {
    let string = parse_macro_input!(string as LitStr);
    let id = string.value().parse();

    let auditeur = Auditeur::new();
    let id_mod = auditeur.id();

    match id {
        Ok(NamespacedId { namespace, id }) => quote!(
            #id_mod::NamespacedIdRef {
                namespace: #namespace,
                id: #id,
            }
        ),
        Err(err) => {
            let err = err.to_string();
            quote_spanned!(string.span() => 
                compile_error!(concat!("failed to parse ident: ", #err));)
        }
    }
    .into()
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
            }.into();
        },
    };

    let auditeur = Auditeur::new();
    let input_mod = auditeur.input();

    quote! {
        impl #input_mod::GetRef for #ident {
            type Ref = Self;
            fn get_ref(&self) -> Self {
                self.clone()
            }
        }
    }.into()
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
            }.into();
        },
    };

    let generics = CustomGenerics::from_attr_or_generics(&attrs, &generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error().into(),
    };

    let auditeur = Auditeur::new();
    implement_path_any(&ident, &generics, auditeur).into()
}

#[cfg(feature = "path-any")]
#[proc_macro]
pub fn implement_path_any_external(ident: TokenStream) -> TokenStream {
    let PathAnyExternal { ident, generics } = parse_macro_input!(ident as PathAnyExternal);

    let auditeur = Auditeur::new();
    // TODO: ensure that there are no name collisions
    implement_path_any(&ident, &CustomGenerics::from_generics(&generics), auditeur).into()
}

#[cfg(feature = "path-any")]
struct PathAnyExternal {
    ident: Ident,
    generics: Generics,
}

impl syn::parse::Parse for PathAnyExternal {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident = input.parse()?;
        let generics = if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            input.parse()?
        } else {
            Generics::default()
        };
        Ok(Self { ident, generics })
    }
}

#[cfg(feature = "path-any")]
fn implement_path_any(ident: &Ident, generics: &CustomGenerics, auditeur: Auditeur) -> proc_macro2::TokenStream {
    let path_any_mod = auditeur.path_any();

    let CustomGenerics { impl_generics, type_generics, where_clause } = generics;

    let mut path_id = quote! {
        #path_any_mod::PathId::new(std::concat!(
            std::module_path!(),
            "::",
            std::stringify!(#ident),
        ))
    };

    for param_bound in generics
        .type_generics
        .iter()
        .filter_map(|param| if let GenericParam::Type(param) = param {
            Some(param)
        } else {
            None
        }) 
    {
        path_id = quote! {
            #path_id.join(&<#param_bound as #path_any_mod::ConstPathAny>::PATH_ID)
        }
    }

    quote! {
        unsafe impl #impl_generics #path_any_mod::ConstPathAny for #ident #type_generics #where_clause {
            const PATH_ID: #path_any_mod::PathId = #path_id;
        }
    }
}

#[derive(Default)]
struct CustomGenerics {
    pub impl_generics: proc_macro2::TokenStream,
    pub type_generics: GenericArgList,
    pub where_clause: proc_macro2::TokenStream,
}

impl CustomGenerics {
    pub fn from_generics(generics: &syn::Generics) -> Self {
        let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
        Self {
            impl_generics: impl_generics.into_token_stream(),
            type_generics: syn::parse2(type_generics.into_token_stream()).expect("parsing should never fail"),
            where_clause: where_clause.into_token_stream(),
        }
    }

    pub fn from_attr_or_generics(attrs: &[Attribute], generics: &syn::Generics) -> syn::Result<Self> {
        for attr in attrs {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if meta.path.get_ident().is_some_and(|ident| ident == "resource_custom_generics") {
                    let Expr::Lit(ExprLit { lit: Lit::Str(string), .. }) = &meta.value else {
                        return Err(syn::Error::new(meta.value.span(), "expected string literal"));
                    };
                    return syn::parse_str(&string.value());
                }
            }
        }
        Ok(Self::from_generics(generics))
    }

    pub fn attr_matches(attr: &Attribute) -> bool {
        if let syn::Meta::NameValue(meta) = &attr.meta {
            meta.path.get_ident().is_some_and(|ident| ident == "resource_custom_generics")
        } else {
            false
        }
    }
}

impl syn::parse::Parse for CustomGenerics {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        parenthesized!(content in input);
        let impl_generics = content.parse_terminated(GenericParam::parse, Token![,])?;
        let _: Token![-] = input.parse()?;
        let content;
        parenthesized!(content in input);
        let type_generics = content.parse_terminated(GenericParam::parse, Token![,])?;
        let where_clause: Option<WhereClause> = input.parse()?;
        Ok(Self {
            impl_generics: quote! { <#impl_generics> },
            type_generics: GenericArgList { params: Some(type_generics) },
            where_clause: where_clause.into_token_stream(),
        })
    }
}

#[derive(Default)]
struct GenericArgList {
    params: Option<Punctuated<GenericParam, Token![,]>>,
}

impl GenericArgList {
    pub fn iter(&self) -> impl Iterator<Item = &GenericParam> {
        self.params.as_ref().map(Punctuated::iter).into_iter().flatten()
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
        Ok(Self { params: Some(params) })
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
    }.into()
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
