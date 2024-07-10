use std::ops::Deref;

use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_macro_input, spanned::Spanned, FnArg, ItemFn};
use thiserror::Error;

#[proc_macro_attribute]
pub fn export(_: TokenStream, item: TokenStream) -> TokenStream {
    // Most of this function deals with ensuring that the function actually has the correct
    // signature. If the function doesn't have the correct signature, it could lead loading the
    // plugin to error (in strange ways)
    fn export(mut func: ItemFn) -> Result<proc_macro2::TokenStream, ExportError> {
        let ident = func.sig.ident.to_string();
        if ident != "init" {
            return Err(ExportError {
                reason: ExportErrorReason::NotNamedInit(ident),
                span: func.sig.ident.span(),
            });
        }

        if !matches!(func.vis, syn::Visibility::Public(_)) {
            return Err(ExportError {
                reason: ExportErrorReason::NotPub,
                span: func.vis.span(),
            });
        }

        if func.sig.generics.lt_token.is_some() || func.sig.generics.where_clause.is_some() {
            return Err(ExportError {
                reason: ExportErrorReason::HasGenerics,
                span: func.sig.generics.span(),
            });
        }

        if func.sig.asyncness.is_some() {
            return Err(ExportError {
                reason: ExportErrorReason::IsAsync,
                span: func.sig.asyncness.span(),
            });
        }

        let param_count = func.sig.inputs.len();
        // this will return None if any parameters are self
        let typed_parameters: Option<Vec<_>> = (func.sig.inputs)
            .iter_mut()
            .map(|parameter| match parameter {
                FnArg::Typed(parameter) => Some(parameter),
                _ => None,
            })
            .collect();
        let has_return = matches!(func.sig.output, syn::ReturnType::Type(_, _));
        // this is a weird way of doing it, but it's necessary due to how let else works
        // this packs all the filters together so that filtered_params is None whenever any filters fail
        // this allows later code to know that all parameters are Typed (through typed_parameters)
        // so that they can access the parameters' patterns
        let filtered_params = (param_count == 1 && has_return)
            .then_some(())
            .and(typed_parameters);

        let Some(params) = filtered_params else {
            return Err(ExportError {
                reason: ExportErrorReason::IncorrectSignature(func.sig.to_token_stream()),
                span: func.sig.span(),
            });
        };

        // ensure that each parameter is the correct type
        let parameter_types = [quote! { ::auditeur_plugin::PluginContext }];
        params
            .into_iter()
            .zip(parameter_types)
            .enumerate()
            .map(|(index, (parameter, typ))| {
                // if the parameter is a wildcard, then change it to a temporary identifier
                // we still have to check the type later, so it must have a name
                if matches!(parameter.pat.deref(), &syn::Pat::Wild(_)) {
                    let span = parameter.pat.span();
                    let ident = Ident::new(&format!("unk_{index}"), span);
                    let pat = syn::PatIdent {
                        ident,
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        subpat: None,
                    };
                    parameter.pat = Box::new(syn::Pat::Ident(pat));
                }
                (parameter, typ)
            })
            .map(|(parameter, typ)| {
                let ident = &parameter.pat;
                let span = parameter.span();
                // create a dummy let statement that will error when the parameter doesn't match
                // the given type above
                // this gives a much better error message than anything else we could do
                syn::parse2(quote_spanned! { span => let _: #typ = #ident; })
                    .expect("statement should parse")
            })
            .for_each(|statement| func.block.stmts.insert(0, statement));

        let span = func.sig.output.span();
        let return_type_assert = quote_spanned! { span =>
            #[allow(unreachable_code)]
            #[allow(clippy::diverging_sub_expression)]
            if false {
                // we fake-call the function in order to find the return type
                // this is allowed since it is in an if false block, so it never gets run but still
                // gets compiled and outputs errors
                // since it never gets run, we can just use unimplemented! for the argument
                let _: ::auditeur_plugin::auditeur::app::bundle::Bundle<_> = init(unimplemented!());
            }
        };
        let return_type_assert = syn::parse2(return_type_assert).expect("statement should parse");
        func.block.stmts.insert(0, return_type_assert);

        Ok(quote! {
            #[::auditeur_plugin::stabby_export(canaries)]
            #func
        })
    }

    let func = parse_macro_input!(item as ItemFn);

    export(func)
        .unwrap_or_else(ExportError::into_compile_error)
        .into()
}

struct ExportError {
    reason: ExportErrorReason,
    span: proc_macro2::Span,
}

#[derive(Debug, Error)]
enum ExportErrorReason {
    #[error("exported function must be named `init`, but it is named {0} instead")]
    NotNamedInit(String),
    #[error("exported function must be public")]
    NotPub,
    #[error("exported function must not have generics")]
    HasGenerics,
    #[error("exported function must not be async")]
    IsAsync,
    #[error(
        "exported function must have the signature of `fn(_: PluginContext) -> Bundle`, but it has {0} instead"
    )]
    IncorrectSignature(proc_macro2::TokenStream),
}

impl ExportError {
    pub fn into_compile_error(self) -> proc_macro2::TokenStream {
        let message = self.reason.to_string();
        quote_spanned! { self.span => pub fn error() { ::core::compile_error!(#message) } }
    }
}
