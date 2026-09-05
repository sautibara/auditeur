use syn::Ident;

use quote::quote;

#[cfg(feature = "path-any")]
pub fn any(ident: &Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    use quote::ToTokens;

    let path_any_mod = crate::AUDITEUR.path_any();

    let (_, type_generics, where_clause) = generics.split_for_impl();

    let mut path_id = quote! {
        #path_any_mod::PathId::new(std::concat!(
            std::module_path!(),
            "::",
            std::stringify!(#ident),
        ))
    };

    let mut impl_generics = quote! {};
    if !generics.params.is_empty() {
        impl_generics.extend(quote! { < });

        for pair in generics.params.pairs() {
            use syn::GenericParam;

            match pair.value() {
                GenericParam::Lifetime(param) => param.to_tokens(&mut impl_generics),
                GenericParam::Const(param) => param.to_tokens(&mut impl_generics),
                GenericParam::Type(param) => {
                    param.ident.to_tokens(&mut impl_generics);
                    param
                        .colon_token
                        .unwrap_or_default()
                        .to_tokens(&mut impl_generics);
                    param.bounds.to_tokens(&mut impl_generics);
                    if !param.bounds.empty_or_trailing() {
                        impl_generics.extend(quote! { + });
                    }
                    impl_generics.extend(quote! { #path_any_mod::PathAnyConst });
                }
            }

            pair.punct().to_tokens(&mut impl_generics);
        }

        impl_generics.extend(quote! { > });
    }

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
