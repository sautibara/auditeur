use quote::{format_ident, quote};
use syn::{Ident, ItemEnum};

pub fn clone_as_struct<'i>(
    ident: &Ident,
    fields: impl Iterator<Item = &'i Ident>,
    generics: &syn::Generics,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics #auditeur::input::CloneIn for #ident #type_generics #where_clause {
            async fn clone_as(&self, app: &mut #auditeur::app::App) -> Self {
                use #auditeur::input::clone_as::Specialization as _;
                Self {
                    #(#fields: (&mut &self.#fields).specialization(app).await),*
                }
            }
        }
    }
}

pub fn clone_as_enum(enm: &ItemEnum) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let (impl_generics, type_generics, where_clause) = enm.generics.split_for_impl();

    let mut variants = quote! {};

    for variant in &enm.variants {
        let variant_ident = &variant.ident;

        match &variant.fields {
            syn::Fields::Named(fields) => {
                let field_ident_1 = fields.named.iter().filter_map(|field| field.ident.as_ref());
                let field_ident_2 = field_ident_1.clone();

                variants.extend(quote! {
                    Self::#variant_ident { #(#field_ident_1),* } => Self::#variant_ident { #(
                        #field_ident_2: (&mut #field_ident_2).specialization(app).await
                    ),* },
                });
            }
            syn::Fields::Unnamed(fields) => {
                let field_ident_1 = fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format_ident!("field_{i}"));
                let field_ident_2 = field_ident_1.clone();

                variants.extend(quote! {
                    Self::#variant_ident ( #(#field_ident_1),* ) => Self::#variant_ident { #(
                        #field_ident_2: (&mut #field_ident_2).specialization(app).await
                    ),* },
                });
            }
            syn::Fields::Unit => {
                variants.extend(quote! {
                    Self::#variant_ident => Self::#variant_ident,
                });
            }
        }
    }

    let ident = &enm.ident;

    quote! {
        impl #impl_generics #auditeur::input::CloneAs for #ident #type_generics #where_clause {
            async fn clone_as(&self, app: &mut #auditeur::app::App) -> Self {
                use #auditeur::input::clone_as::Specialization as _;
                match self {
                    #variants
                }
            }
        }
    }
}
