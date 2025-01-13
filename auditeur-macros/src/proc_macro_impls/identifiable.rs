use quote::quote;
use syn::{Ident, Item};

use crate::{custom_generics::CustomGenerics, namespaced_id::NamespacedId, Auditeur};

pub fn identifiable(id: NamespacedId, item: &mut Item) -> syn::Result<proc_macro2::TokenStream> {
    let (ident, generics) = CustomGenerics::from_item(item)?;
    Ok(identifiable_with_generics(id, ident, &generics))
}

pub fn identifiable_with_generics(
    id: NamespacedId,
    ident: &Ident,
    generics: &CustomGenerics,
) -> proc_macro2::TokenStream {
    let auditeur = Auditeur::new();
    let id_mod = auditeur.id();
    let resource_mod = auditeur.resource();

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
    )
}
