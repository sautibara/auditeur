use syn::ItemStruct;

use crate::{Auditeur, custom_generics::CustomGenerics};

use quote::{format_ident, quote};

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
            quote! {
                async fn create<'a>(
                    registry: &'a #auditeur::app::Registry,
                    dependent: #auditeur::app::ComponentIdRef<'_>,
                ) -> Result<Self, #auditeur::Error> {
                    let ( #(#members,)* ) = #auditeur::try_join!(
                        #( <#types as #input_mod::Input>::create(registry, dependent) ),*
                    )?;
                    Ok(Self { #(#members),* })
                }

                fn constraints() -> #auditeur::input::Constraints {
                    #auditeur::input::Constraints::empty()
                        #(.extend(<#types as #input_mod::Input>::constraints()))*
                }
            }
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
