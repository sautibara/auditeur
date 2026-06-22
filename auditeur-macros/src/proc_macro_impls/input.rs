use proc_macro2::TokenStream;
use syn::ItemStruct;

use quote::{ToTokens, quote};

#[allow(clippy::cognitive_complexity)] // sorry...
pub fn derive(strct: &ItemStruct) -> proc_macro2::TokenStream {
    let (ident, generics) = (&strct.ident, &strct.generics);

    let auditeur = &*crate::AUDITEUR;

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    let implementations = match &strct.fields {
        syn::Fields::Named(_) => {
            let members = strct.fields.members();
            let types = strct.fields.iter().map(|field| &field.ty);
            expand_named_struct(members, types)
        }
        syn::Fields::Unnamed(_) => {
            let types = strct.fields.iter().map(|field| &field.ty);
            quote! {
                async fn create(
                    app: &mut #auditeur::app::App,
                ) -> Result<Self, #auditeur::Error> {
                    Ok(Self(
                        #( <#types as #auditeur::input::Input>::create(app).await?, )*
                    ))
                }
            }
        }
        syn::Fields::Unit => quote! {
            async fn create(
                _: &mut #auditeur::app::App,
            ) -> Result<Self, #auditeur::Error> {
                Ok(Self)
            }
        },
    };

    quote! {
        impl #impl_generics #auditeur::input::Input for #ident #type_generics #where_clause {
            #implementations
        }
    }
}

pub fn expand_named_struct<I: ToTokens, T: ToTokens>(
    idents: impl IntoIterator<Item = I>,
    types: impl IntoIterator<Item = T>,
) -> TokenStream {
    let auditeur = &*crate::AUDITEUR;
    let idents = idents.into_iter();
    let types = types.into_iter();
    quote! {
        async fn create(
            app: &mut #auditeur::app::App,
        ) -> Result<Self, #auditeur::Error> {
            Ok(Self { #(
                #idents: <#types as #auditeur::input::Input>::create(app).await?,
            )* })
        }
    }
}
