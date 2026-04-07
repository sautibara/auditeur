use quote::{ToTokens, quote};
use syn::Ident;

use crate::namespaced_id::NamespacedId;

pub fn identifiable(
    id: &NamespacedId,
    ident: &Ident,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let auditeur = &*crate::AUDITEUR;
    quote! {
       impl #impl_generics #auditeur::app::Identifiable for #ident #type_generics
       #where_clause
       {
           fn dyn_id(&self) -> &'static #auditeur::namespaced_id::NamespacedIdRef {
               <Self as #auditeur::app::Identifiable>::id()
           }

           fn id() -> &'static #auditeur::namespaced_id::NamespacedIdRef
           where
               Self: Sized
           {
               #id
           }
       }
    }
}

#[derive(Clone, Copy)]
pub enum ComponentType {
    Resource,
    #[allow(dead_code)]
    Bundle,
    Message,
}

impl ToTokens for ComponentType {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            #auditeur::app::ComponentType::
        });
        tokens.extend(match self {
            Self::Resource => quote! { Resource },
            Self::Bundle => quote! { Bundle },
            Self::Message => quote! { Message },
        });
    }
}

pub fn component(
    ident: &Ident,
    generics: &syn::Generics,
    component_type: ComponentType,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics #auditeur::app::Component for #ident #type_generics
        #where_clause
        {
            const TYPE: #auditeur::app::ComponentType = #component_type;
        }
    }
}
