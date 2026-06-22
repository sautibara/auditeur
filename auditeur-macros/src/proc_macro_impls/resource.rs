use quote::quote;
use syn::{Ident, Item};

use crate::namespaced_id::NamespacedId;

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
    let any_impl = super::any(ident, generics);

    let auditeur = &*crate::AUDITEUR;
    let maybe_get_ref_impl = super::maybe_get_ref(ident, generics);
    let identifiable_impl = super::identified(id, ident, generics);
    let component_impl = super::component(
        ident,
        generics,
        &quote! {
            #auditeur::component::resource::Resources
        },
    );

    quote!(
        #identifiable_impl
        #component_impl
        #any_impl
        #maybe_get_ref_impl
    )
}
