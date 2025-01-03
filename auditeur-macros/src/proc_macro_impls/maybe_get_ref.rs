use syn::Ident;

use crate::{custom_generics::CustomGenerics, Auditeur};

use quote::quote;

pub fn maybe_get_ref(
    ident: &Ident,
    generics: &CustomGenerics,
    auditeur: &Auditeur,
) -> proc_macro2::TokenStream {
    let input_mod = auditeur.input();

    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    quote! {
        impl #impl_generics #input_mod::MaybeGetRef for #ident #type_generics #where_clause {
            fn maybe_get_ref(&self) -> Option<Box<dyn #input_mod::CloneAny>> {
                use #input_mod::maybe_get_ref::Specialization as _;
                (&mut &*self).specialization()
            }
        }
    }
}
