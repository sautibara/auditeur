use syn::GenericParam;
use syn::Ident;

use crate::AUDITEUR;
use crate::custom_generics::CustomGenerics;

use quote::quote;

pub fn path_any(ident: &Ident, generics: &CustomGenerics) -> proc_macro2::TokenStream {
    let path_any_mod = AUDITEUR.path_any();

    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    let mut path_id = quote! {
        #path_any_mod::PathId::new(std::concat!(
            std::module_path!(),
            "::",
            std::stringify!(#ident),
        ))
    };

    for param_bound in generics.type_generics.iter().filter_map(|param| {
        if let GenericParam::Type(param) = param {
            Some(param)
        } else {
            None
        }
    }) {
        path_id = quote! {
            #path_id.join(&<#param_bound as #path_any_mod::PathAnyConst>::PATH_ID)
        }
    }

    quote! {
        unsafe impl #impl_generics #path_any_mod::PathAnyConst for #ident #type_generics #where_clause {
            const PATH_ID: #path_any_mod::PathId = #path_id;
        }
    }
}
