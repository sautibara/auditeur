use namespaced_id::NamespacedId;
use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro_crate::crate_name;
use quote::{quote, quote_spanned};
use syn::{parse_macro_input, Ident, ItemStruct, LitStr};

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
            .expect("auditeur should be in Cargo.toml");

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
}

#[proc_macro_attribute]
pub fn resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemStruct);
    let ident = item.ident.clone();

    let string = parse_macro_input!(attr as LitStr);
    let id = string.value().parse();

    let auditeur = Auditeur::new();
    let id_mod = auditeur.id();
    let resource_mod = auditeur.resource();

    match id {
        Ok(NamespacedId { namespace, id }) => quote!(
            impl #resource_mod::Resource for #ident {
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
