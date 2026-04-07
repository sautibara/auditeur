use proc_macro2::Ident;

use crate::{namespaced_id::NamespacedId, proc_macro_impls::ComponentType};
use quote::quote;

pub fn derive_message(
    ident: &Ident,
    id: &NamespacedId,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let identifiable_impl = super::identifiable(id, ident, generics);
    let component_impl = super::component(ident, generics, ComponentType::Message);

    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(ident, generics);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    let auditeur = &*crate::AUDITEUR;

    quote! {
        impl #auditeur::Message for #ident {}

        #identifiable_impl
        #component_impl
        #path_any_impl
    }
}
