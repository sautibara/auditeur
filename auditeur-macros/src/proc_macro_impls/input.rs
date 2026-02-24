use proc_macro2::TokenStream;
use syn::ItemStruct;

use crate::{Auditeur, custom_generics::CustomGenerics};

use quote::{ToTokens, format_ident, quote};

#[allow(clippy::cognitive_complexity)] // sorry...
pub fn derive(strct: &ItemStruct) -> proc_macro2::TokenStream {
    let (ident, generics, attrs) = (&strct.ident, &strct.generics, &strct.attrs);

    let generics = CustomGenerics::from_attr_or_generics(attrs, generics);
    let generics = match generics {
        Ok(generics) => generics,
        Err(error) => return error.into_compile_error(),
    };

    let auditeur = Auditeur::new();
    let input_mod = auditeur.input();
    let auditeur = auditeur.crate_name();

    let CustomGenerics {
        impl_generics,
        type_generics,
        where_clause,
    } = generics;

    let implementations = match &strct.fields {
        syn::Fields::Named(_) => {
            let members: Vec<_> = strct.fields.members().collect();
            let types: Vec<_> = strct.fields.iter().map(|field| &field.ty).collect();
            expand_named_struct(members, types)
        }
        syn::Fields::Unnamed(_) => {
            let members: Vec<_> = strct
                .fields
                .members()
                .map(|member| format_ident!("member{}", member))
                .collect();
            let types: Vec<_> = strct.fields.iter().map(|field| &field.ty).collect();
            quote! {
                async fn create(
                    registry: &#auditeur::app::Registry,
                    dependent: #auditeur::app::ComponentIdRef<'_>,
                ) -> Result<Self, #auditeur::Error> {
                    let ( #(#members,)* ) = #auditeur::try_join!(
                        #( <#types as #input_mod::Input>::create(registry, dependent) ),*
                    )?;
                    Ok(Self( #(#members),* ))
                }

                fn constraints() -> #auditeur::input::Constraints {
                    #auditeur::input::Constraints::empty()
                        #(.extend(<#types as #input_mod::Input>::constraints()))*
                }
            }
        }
        syn::Fields::Unit => quote! {
            async fn create(
                _: &#auditeur::app::Registry,
                _: #auditeur::app::ComponentIdRef<'_>,
            ) -> Result<Self, #auditeur::Error> {
                Self
            }

            fn constraints() -> #auditeur::input::Constraints {
                #auditeur::input::Constraints::empty()
            }
        },
    };

    quote! {
        impl #impl_generics #input_mod::Input for #ident #type_generics #where_clause {
            #implementations
        }
    }
}

pub fn expand_named_struct<'a, I: ToTokens>(
    idents: impl IntoIterator<Item = I, IntoIter: Clone>,
    types: impl IntoIterator<Item = &'a syn::Type, IntoIter: Clone>,
) -> TokenStream {
    let auditeur = &*crate::AUDITEUR;
    let idents_0 = idents.into_iter();
    let idents_1 = idents_0.clone();
    let types_0 = types.into_iter();
    let types_1 = types_0.clone();
    quote! {
        async fn create<'a>(
            registry: &'a #auditeur::app::Registry,
            dependent: #auditeur::app::ComponentIdRef<'_>,
        ) -> Result<Self, #auditeur::Error> {
            let ( #(#idents_0,)* ) = #auditeur::try_join!(
                #( <#types_0 as #auditeur::input::Input>::create(registry, dependent) ),*
            )?;
            Ok(Self { #(#idents_1),* })
        }

        fn constraints() -> #auditeur::input::Constraints {
            #auditeur::input::Constraints::empty()
                #(.extend(<#types_1 as #auditeur::input::Input>::constraints()))*
        }
    }
}
