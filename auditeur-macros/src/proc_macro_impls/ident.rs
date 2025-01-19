use quote::quote;

use crate::{namespaced_id::NamespacedId, Auditeur};

pub fn ident(ident: &NamespacedId, auditeur: &Auditeur) -> proc_macro2::TokenStream {
    let namespaced_id = auditeur.id();

    let namespace_size = ident.namespace.len();
    let string = ident.to_string();
    let bytes = string.bytes();

    quote! {
        {
            const COERCE: &#namespaced_id::NamespacedIdRef = unsafe {
                const COERCE: &#namespaced_id::Coerce<[u8]> = &#namespaced_id::Coerce {
                    namespace_size: #namespace_size,
                    data: [#(#bytes),*],
                };
                // SAFETY: Coerce<[u8]>, Coerce<str>, and NamespacedIdRef all have the same layout
                // and this is valid utf-8.
                std::mem::transmute(COERCE)
            };
            COERCE
        }
    }
}
