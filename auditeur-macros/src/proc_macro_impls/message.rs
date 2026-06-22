use proc_macro2::Ident;

use crate::namespaced_id::NamespacedId;
use quote::quote;

pub fn derive_listen_message(
    ident: &Ident,
    id: &NamespacedId,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let identifiable_impl = super::identified(id, ident, generics);
    let any_impl = super::any(ident, generics);
    let component_impl = super::component(
        ident,
        generics,
        &quote! {
            #auditeur::component::listen::ListenMessages
        },
    );

    quote! {
        impl #auditeur::component::listen::ListenMessage for #ident {}

        #identifiable_impl
        #component_impl
        #any_impl
    }
}

pub fn derive_take_message(
    ident: &Ident,
    id: &NamespacedId,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let identifiable_impl = super::identified(id, ident, generics);
    let any_impl = super::any(ident, generics);
    let component_impl = super::component(
        ident,
        generics,
        &quote! {
            #auditeur::component::take::TakeMessages
        },
    );

    quote! {
        impl #auditeur::component::take::TakeMessage for #ident {}

        #identifiable_impl
        #component_impl
        #any_impl
    }
}
