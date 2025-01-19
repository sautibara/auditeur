use quote::quote;
use syn::Item;

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId, Auditeur};

pub fn resource(mut item: Item, id: &NamespacedId) -> syn::Result<proc_macro2::TokenStream> {
    let (ident, generics) = CustomGenerics::from_item(&mut item)?;
    let auditeur = Auditeur::new();

    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(ident, &generics, &auditeur);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    #[cfg(feature = "runtime-loading")]
    let maybe_get_ref_impl = super::maybe_get_ref(ident, &generics, &auditeur);
    #[cfg(not(feature = "runtime-loading"))]
    let maybe_get_ref_impl = quote!();

    let identifiable_impl = super::identifiable_with_generics(id, ident, &generics);

    Ok(quote!(
        #item
        #identifiable_impl
        #path_any_impl
        #maybe_get_ref_impl
    ))
}
