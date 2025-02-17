use syn::{
    parse::Parse, parse_quote, spanned::Spanned, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl,
    ItemStruct, Type,
};

use crate::{custom_generics::CustomGenerics, input_list::InputReferenceList, Auditeur};

use quote::{format_ident, quote, ToTokens};

use super::input_reflection::{input_macro_of, input_macro_of_method};

#[allow(clippy::cognitive_complexity)] // sorry...
pub fn derive(strct: ItemStruct) -> proc_macro2::TokenStream {
    let (ident, generics, attrs) = (strct.ident, strct.generics, strct.attrs);

    let generics = CustomGenerics::from_attr_or_generics(&attrs, &generics);
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
                async fn create<'a>(registry: &'a #auditeur::app::Registry, as_resource: &NamespacedIdRef) -> Result<Self, #auditeur::Error> {
                    let ( #(#members),* ) = #auditeur::try_join!(
                        #( <#types as #input_mod::Input>::create(registry, as_resource) ),*
                    )?;
                    Ok(Self { #(#members),* })
                }

                fn constraints() -> #auditeur::app::Constraints {
                    #auditeur::app::Constraints::empty()
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
                async fn create(registry: &#auditeur::app::Registry, as_resource: &NamespacedIdRef) -> Result<Self, #auditeur::Error> {
                    let ( #(#members),* ) = #auditeur::try_join!(
                        #( <#types as #input_mod::Input>::create(registry, as_resource) ),*
                    )?;
                    Ok(Self( #(#members),* ))
                }

                fn constraints() -> #auditeur::app::Constraints {
                    #auditeur::app::Constraints::empty()
                        #(.extend(<#types as #input_mod::Input>::constraints()))*
                }
            }
        }
        syn::Fields::Unit => quote! {
            async fn create(_: &#auditeur::app::Registry, _: &NamespacedIdRef) -> Result<Self, #auditeur::Error> {
                Self
            }

            fn constraints() -> #auditeur::app::Constraints {
                #auditeur::app::Constraints::empty()
            }
        },
    };

    quote! {
        impl #impl_generics #input_mod::Input for #ident #type_generics #where_clause {
            #implementations
        }
    }
}

fn uppercase_first_letter(string: &str) -> String {
    let mut chars = string.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

pub struct HasInput {
    implementation: ItemImpl,
}

impl HasInput {
    fn funcs_mut(items: &mut [ImplItem]) -> impl Iterator<Item = &mut ImplItemFn> {
        // TODO: no_input attribute
        items.iter_mut().filter_map(|item| {
            if let ImplItem::Fn(item) = item {
                Some(item)
            } else {
                None
            }
        })
    }

    fn type_ident_of(method_ident: &Ident) -> Ident {
        let const_ident = method_ident.to_string();
        let const_ident = uppercase_first_letter(&const_ident);
        let const_ident = const_ident + "Input";
        syn::Ident::new(&const_ident, method_ident.span())
    }

    pub fn expand(mut self) -> syn::Result<proc_macro2::TokenStream> {
        let Some((None, trait_, _)) = &self.implementation.trait_ else {
            return Err(syn::Error::new_spanned(
                self.implementation,
                "expected this to be a trait implementation",
            ));
        };
        let (_, trait_generics, _) = self.implementation.generics.split_for_impl();
        let trait_: Type = parse_quote!( #trait_ #trait_generics );

        let implementer = &self.implementation.self_ty;

        let mut types = Vec::new();
        let mut macros = proc_macro2::TokenStream::new();
        let func_count = Self::funcs_mut(&mut self.implementation.items).count();
        for func in Self::funcs_mut(&mut self.implementation.items) {
            let last_arg_span = func.sig.inputs.last().span();
            let last_arg = func.sig.inputs.last_mut();
            let last_arg = last_arg
                .and_then(|input| {
                    if let FnArg::Typed(input) = input {
                        Some(input)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    syn::Error::new(last_arg_span, "expected at least one typed argument")
                })?;

            let const_ident = Self::type_ident_of(&func.sig.ident);
            let referenced_inputs: InputReferenceList = syn::parse2(last_arg.ty.to_token_stream())?;
            let referenced_inputs = referenced_inputs.with_lifetime("'a");
            types.push(parse_quote!( type #const_ident<'a> = #referenced_inputs; ));

            let owned_inputs = referenced_inputs.into_owned_list();
            let macro_ident = if func_count > 1 {
                input_macro_of_method(&trait_, implementer, &func.sig.ident.to_string())?
            } else {
                input_macro_of(&trait_, implementer)?
            };
            macros.extend(super::export_input(&macro_ident, &owned_inputs));

            last_arg.ty = parse_quote!( <Self as #trait_>::#const_ident<'_> );
        }

        self.implementation.items.extend(types);
        let implementation = self.implementation;
        Ok(quote! {
            #macros
            #implementation
        })
    }
}

impl Parse for HasInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            implementation: input.parse()?,
        })
    }
}
