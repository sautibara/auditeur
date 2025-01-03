use quote::quote;
use syn::ItemStruct;

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId, Auditeur};

pub fn resource(mut item: ItemStruct, id: NamespacedId) -> proc_macro2::TokenStream {
    let ident = item.ident.clone();

    let auditeur = Auditeur::new();
    let id_mod = auditeur.id();
    let resource_mod = auditeur.resource();

    let generics = CustomGenerics::from_attr_or_generics(&item.attrs, &item.generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error(),
    };

    item.attrs
        .retain(|attr| !CustomGenerics::attr_matches(attr));

    #[cfg(feature = "path-any")]
    let path_any_impl = super::path_any(&ident, &generics, &auditeur);
    #[cfg(not(feature = "path-any"))]
    let path_any_impl = quote!();

    #[cfg(feature = "runtime-loading")]
    let maybe_get_ref_impl = super::maybe_get_ref(&ident, &generics, &auditeur);
    #[cfg(not(feature = "runtime-loading"))]
    let maybe_get_ref_impl = quote!();

    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    let NamespacedId { namespace, id } = id;
    quote!(
        impl #impl_generics #resource_mod::Identifiable for #ident #type_generics #where_clause {
            fn ident(&self) -> #id_mod::NamespacedIdRef<'static> {
                #id_mod::NamespacedIdRef {
                    namespace: #namespace,
                    id: #id,
                }
            }

            fn const_ident() -> #id_mod::NamespacedIdRef<'static>
            where
                Self: Sized
            {
                #id_mod::NamespacedIdRef {
                    namespace: #namespace,
                    id: #id,
                }
            }
        }

        #path_any_impl
        #maybe_get_ref_impl

        #item
    )
}
