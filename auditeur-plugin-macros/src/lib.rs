use proc_macro::TokenStream;
use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_macro_input, spanned::Spanned, ItemFn, LitStr, Stmt};
use thiserror::Error;

#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    fn export(mut func: ItemFn, id: LitStr) -> Result<proc_macro2::TokenStream, ExportError> {
        let ident = func.sig.ident.to_string();
        if ident != "main" {
            return Err(ExportError {
                reason: ExportErrorReason::NotNamedMain(ident),
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

        let params = &func.sig.inputs;
        let too_many_params = params.len() > 1;
        let first_param_is_typed = params
            .first()
            .is_some_and(|parameter| matches!(parameter, syn::FnArg::Typed(_)));
        let no_return = matches!(func.sig.output, syn::ReturnType::Default);
        if too_many_params || !first_param_is_typed || !no_return {
            return Err(ExportError {
                reason: ExportErrorReason::IncorrectSignature(func.sig.to_token_stream()),
                span: func.sig.span(),
            });
        }

        // ensure that the app parameter actually takes in an app
        let first_parameter_span = params.first().expect("checked above").span();
        let statement: Stmt = syn::parse2(
            quote_spanned! { first_parameter_span => let _: &mut ::auditeur::app::App<_> = app; },
        )
        .expect("statement should parse");
        func.block.stmts.insert(0, statement);

        Ok(quote! {
            #[::auditeur_plugin::stabby_export(canaries)]
            #func

            #[::auditeur_plugin::stabby_export(canaries)]
            pub fn ident() -> ::auditeur_plugin::auditeur::resource::NamespacedIdRef<'static> {
                ::auditeur_plugin::auditeur::ident!(#id)
            }
        })
    }

    let func = parse_macro_input!(item as ItemFn);
    let id = parse_macro_input!(attr as LitStr);

    export(func, id)
        .unwrap_or_else(ExportError::into_compile_error)
        .into()
}

struct ExportError {
    reason: ExportErrorReason,
    span: proc_macro2::Span,
}

#[derive(Debug, Error)]
enum ExportErrorReason {
    #[error("exported function must be named `main`, but it is named {0} instead")]
    NotNamedMain(String),
    #[error("exported function must be public")]
    NotPub,
    #[error("exported function must not have generics")]
    HasGenerics,
    #[error("exported function must not be async")]
    IsAsync,
    #[error(
        "exported function must have the signature of `fn(app: &mut App)`, but it has {0} instead"
    )]
    IncorrectSignature(proc_macro2::TokenStream),
}

impl ExportError {
    pub fn into_compile_error(self) -> proc_macro2::TokenStream {
        let message = self.reason.to_string();
        quote_spanned! { self.span => pub fn error() { ::core::compile_error!(#message) } }
    }
}
