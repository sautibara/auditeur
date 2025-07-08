use syn::Ident;

use crate::custom_generics::CustomGenerics;

use quote::quote;

pub fn maybe_get_ref(ident: &Ident, generics: &CustomGenerics) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    quote! {
        impl #impl_generics #auditeur::input::get::MaybeGetRef for #ident #type_generics #where_clause {
            fn maybe_get_ref(&self, registry: &#auditeur::app::Registry) -> Option<Box<dyn #auditeur::input::get::CloneAny>> {
                use #auditeur::input::get::maybe_get_ref::Specialization as _;
                (&mut &*self).specialization(registry)
            }
        }
    }
}
