use quote::quote;
use syn::{Ident, Item};

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId};

pub fn identifiable(id: &NamespacedId, item: &mut Item) -> syn::Result<proc_macro2::TokenStream> {
    let (ident, generics) = CustomGenerics::from_item(item)?;
    Ok(identifiable_with_generics(id, ident, &generics))
}

pub fn identifiable_with_generics(
    id: &NamespacedId,
    ident: &Ident,
    generics: &CustomGenerics,
) -> proc_macro2::TokenStream {
    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    let auditeur = &*crate::AUDITEUR;
    quote! {
       impl #impl_generics #auditeur::resource::Identifiable for #ident #type_generics
       #where_clause
       {
           fn ident(&self) -> &'static #auditeur::namespaced_id::NamespacedIdRef {
               Self::const_ident()
           }

           fn const_ident() -> &'static #auditeur::namespaced_id::NamespacedIdRef
           where
               Self: Sized
           {
               #id
           }
       }
    }
}
