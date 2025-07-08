use quote::quote;
use syn::{Ident, Item};

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId};

pub fn resource(mut item: Item, id: &NamespacedId) -> syn::Result<proc_macro2::TokenStream> {
    let (ident, generics) = CustomGenerics::from_item(&mut item)?;
    let impls = resource_impls_with_generics(ident, id, &generics);

    Ok(quote! {
        #item
        #impls
    })
}

pub fn resource_impls_with_generics(
    ident: &Ident,
    id: &NamespacedId,
    generics: &CustomGenerics,
) -> proc_macro2::TokenStream {
    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(ident, generics);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    #[cfg(feature = "runtime-loading")]
    let maybe_get_ref_impl = super::maybe_get_ref(ident, generics);
    #[cfg(not(feature = "runtime-loading"))]
    let maybe_get_ref_impl = quote!();

    let identifiable_impl = super::identifiable_with_generics(id, ident, generics);

    quote!(
        #identifiable_impl
        #path_any_impl
        #maybe_get_ref_impl
    )
}
