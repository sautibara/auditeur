use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, quote_spanned, ToTokens};
use syn::{parse_macro_input, spanned::Spanned, Abi, FnArg, ItemFn, LitStr, Signature, Stmt};
use thiserror::Error;

// Most of this function deals with ensuring that the function actually has the correct
// signature. If the function doesn't have the correct signature, it could lead loading the
// plugin to error (in strange ways)
fn export_inner(mut func: ItemFn) -> Result<proc_macro2::TokenStream, ExportError> {
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

    let statements = export_check_signature(&mut func.sig)?
        .chain(export_check_return_value(func.sig.output.span()));
    for stmt in statements {
        // the statements have to be inserted at the beginning in case of an implicit return at the
        // end of the block
        func.block.stmts.insert(0, stmt);
    }

    match func.sig.abi {
        Some(ref abi) if abi.name.as_ref().is_some_and(|name| name.value() == "Rust") => (),
        Some(abi) => {
            return Err(ExportError {
                reason: ExportErrorReason::IncorrectABI(abi.name.as_ref().map(LitStr::value)),
                span: abi.span(),
            })
        }
        None => {
            let abi = Abi {
                extern_token: syn::token::Extern {
                    span: func.sig.span(),
                },
                name: Some(LitStr::new("Rust", func.sig.span())),
            };
            func.sig.abi = Some(abi);
        }
    }

    if let Some(FnArg::Typed(context_arg)) = func.sig.inputs.get(0) {
        // Using the argument's span in case of a weird pattern.
        let span = context_arg.span();
        let ident = &context_arg.pat;
        let stmt =
            syn::parse2(quote_spanned! { span => #ident.init(); }).expect("statement should parse");
        func.block.stmts.insert(0, stmt);
    }

    Ok(quote! {
        #[::auditeur_plugin::stabby_export(canaries)]
        // a few pedantic lints to disable
        #[allow(clippy::no_mangle_with_rust_abi)]
        #[allow(clippy::needless_pass_by_value)]
        #func
    })
}

fn export_check_signature(sig: &mut Signature) -> Result<impl Iterator<Item = Stmt>, ExportError> {
    let param_count = sig.inputs.len();
    // this will return None if any parameters are self
    let typed_parameters: Option<Vec<_>> = (sig.inputs)
        .iter_mut()
        .map(|parameter| match parameter {
            FnArg::Typed(parameter) => Some(parameter),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let has_return = matches!(sig.output, syn::ReturnType::Type(_, _));
    // this is a weird way of doing it, but it's necessary due to how let else works
    // this packs all the filters together so that filtered_params is None whenever any filters fail
    // this allows later code to know that all parameters are Typed (through typed_parameters)
    // so that they can access the parameters' patterns
    let filtered_params = (param_count == 1 && has_return)
        .then_some(())
        .and(typed_parameters);

    let Some(params) = filtered_params else {
        return Err(ExportError {
            reason: ExportErrorReason::IncorrectSignature(sig.to_token_stream()),
            span: sig.span(),
        });
    };

    // ensure that each parameter is the correct type
    let parameter_types = [quote! { ::auditeur_plugin::PluginContext }];
    let statements = params
        .into_iter()
        .zip(parameter_types)
        .enumerate()
        .map(|(index, (parameter, typ))| {
            // if the parameter is a wildcard, then change it to a temporary identifier
            // we still have to check the type later, so it must have a name
            if matches!(*parameter.pat, syn::Pat::Wild(_)) {
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
        });
    // nll problem case 3 requires collecting here to avoid the mutable reference captured in the
    // iterator interfering with the immutable borrow required to return an error
    Ok(statements.collect::<Vec<_>>().into_iter())
}

fn export_check_return_value(span: Span) -> impl Iterator<Item = Stmt> {
    let return_type_assert = quote_spanned! { span =>
        #[allow(unreachable_code)]
        #[allow(clippy::diverging_sub_expression)]
        if false {
            // we fake-call the function in order to find the return type
            // this is allowed since it is in an if false block, so it never gets run but still
            // gets compiled and outputs errors
            // since it never gets run, we can just use unimplemented! for the argument
            let _: ::auditeur_plugin::auditeur::bundle::Bundle = init(unimplemented!());
        }
    };
    let return_type_assert = syn::parse2(return_type_assert).expect("statement should parse");
    std::iter::once(return_type_assert)
}

#[proc_macro_attribute]
pub fn export(_: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);

    export_inner(func)
        .unwrap_or_else(ExportError::into_compile_error)
        .into()
}

struct ExportError {
    reason: ExportErrorReason,
    span: proc_macro2::Span,
}

fn display_optional_string(string: Option<&str>) -> String {
    string.map_or_else(|| "None".to_string(), |string| format!(r#""{string}""#))
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
    #[error(r#"incorrect ABI specified: got {}, expected "Rust""#, display_optional_string(.0.as_deref()))]
    IncorrectABI(Option<String>),
}

impl ExportError {
    pub fn into_compile_error(self) -> proc_macro2::TokenStream {
        let message = self.reason.to_string();
        quote_spanned! { self.span => pub fn error() { ::core::compile_error!(#message) } }
    }
}
