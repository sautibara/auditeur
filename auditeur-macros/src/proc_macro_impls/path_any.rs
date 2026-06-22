use syn::Ident;

use crate::AUDITEUR;

use quote::quote;

#[cfg(feature = "path-any")]
pub fn any(ident: &Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let path_any_mod = AUDITEUR.path_any();

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let mut path_id = quote! {
        #path_any_mod::PathId::new(std::concat!(
            std::module_path!(),
            "::",
            std::stringify!(#ident),
        ))
    };

    for param_bound in generics.type_params() {
        let ty = &param_bound.ident;
        path_id = quote! {
            #path_id.join(&<#ty as #path_any_mod::PathAnyConst>::PATH_ID)
        }
    }

    quote! {
        unsafe impl #impl_generics #path_any_mod::PathAnyConst for #ident #type_generics #where_clause {
            const PATH_ID: #path_any_mod::PathId = #path_id;
        }
    }
}

#[cfg(not(feature = "path-any"))]
pub fn any(_: &Ident, _: &syn::Generics) -> proc_macro2::TokenStream {
    quote! {}
}
