use quote::quote;
use syn::Ident;

use crate::namespaced_id::NamespacedId;

pub fn identified(
    id: &NamespacedId,
    ident: &Ident,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let auditeur = &*crate::AUDITEUR;
    quote! {
       impl #impl_generics #auditeur::component::Identified for #ident #type_generics
       #where_clause
       {
           fn id_dyn(&self) -> &#auditeur::namespaced_id::NamespacedIdRef {
               <Self as #auditeur::component::IdentifiedType>::id()
           }
       }

       impl #impl_generics #auditeur::component::IdentifiedType for #ident #type_generics
       #where_clause
       {
           fn id() -> &'static #auditeur::namespaced_id::NamespacedIdRef
           where
               Self: Sized
           {
               <Self as #auditeur::component::IdentifiedConst>::ID
           }
       }

       impl #impl_generics #auditeur::component::IdentifiedConst for #ident #type_generics
       #where_clause
       {
           const ID: &'static #auditeur::namespaced_id::NamespacedIdRef = #id;
       }
    }
}

pub fn component(
    ident: &Ident,
    generics: &syn::Generics,
    registry: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics #auditeur::component::RegistryOf for #ident #type_generics
        #where_clause
        {
            type Registry = #registry;
        }
    }
}
