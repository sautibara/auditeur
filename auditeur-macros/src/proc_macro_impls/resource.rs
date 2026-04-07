use quote::quote;
use syn::{Ident, Item};

use crate::{namespaced_id::NamespacedId, proc_macro_impls::ComponentType};

pub fn resource(item: &Item, id: &NamespacedId) -> syn::Result<proc_macro2::TokenStream> {
    let (ident, generics, _) = crate::extract_item_values(item)?;
    let impls = resource_impls_with_generics(ident, id, generics);

    Ok(quote! {
        #item
        #impls
    })
}

pub fn resource_impls_with_generics(
    ident: &Ident,
    id: &NamespacedId,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(ident, generics);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    #[cfg(feature = "runtime-loading")]
    let maybe_get_ref_impl = super::maybe_get_ref(ident, generics);
    #[cfg(not(feature = "runtime-loading"))]
    let maybe_get_ref_impl = quote!();

    let identifiable_impl = super::identifiable(id, ident, generics);
    let component_impl = super::component(ident, generics, ComponentType::Resource);

    quote!(
        #identifiable_impl
        #component_impl
        #path_any_impl
        #maybe_get_ref_impl
    )
}
