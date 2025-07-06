use proc_macro2::Ident;

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId};
use quote::quote;

pub fn derive_message(
    ident: &Ident,
    id: &NamespacedId,
    generics: &CustomGenerics,
) -> proc_macro2::TokenStream {
    let identifiable_impl = super::identifiable_with_generics(id, ident, generics);

    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(ident, generics);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    let auditeur = &*crate::AUDITEUR;

    quote! {
        impl #auditeur::Message for #ident {}

        #identifiable_impl
        #path_any_impl
    }
}
