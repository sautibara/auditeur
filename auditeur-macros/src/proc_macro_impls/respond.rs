use std::borrow::Cow;

use proc_macro2::Span;
use quote::{format_ident, quote, quote_spanned, ToTokens};
use syn::{
    parse::Parse, parse_quote, punctuated::Punctuated, spanned::Spanned, token, Attribute, FnArg,
    Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Pat, ReturnType, Token, Type, Visibility,
};

use crate::{
    custom_generics::CustomGenerics,
    field::{Comma, List, NameValue},
    input_list::{Combination, InputList, InputReferenceList},
    namespaced_id::NamespacedId,
};

// #[respond(
//     "core:library",
//     query(
//         auto,
//         message(ident = LibraryQuery, id = "core:library_query"),
//         ref(ident = LibraryConnection),
//     ),
//     transaction(
//         auto,
//         message(ident = LibraryTransaction, id = "core:library_transaction"),
//         ref(ident = LibraryRef),
//     ),
// )]

const RESPONSE: &str = "response";

fn snake_to_upper_camel_case(snake: &str) -> String {
    snake.split('_').fold(String::new(), |mut string, slice| {
        let mut chars = slice.chars();
        if let Some(first) = chars.next() {
            string.extend(first.to_uppercase());
        }
        string.extend(chars);
        string
    })
}

fn input_idents() -> impl Iterator<Item = String> {
    let mut index = 0;
    std::iter::from_fn(move || {
        let str = format!("auditeur{index}");
        index += 1;
        Some(str)
    })
}

pub struct Respond {
    pub meta: Meta,
    pub item_impl: ItemImpl,
}

impl Respond {
    pub const fn new(meta: Meta, item_impl: ItemImpl) -> Self {
        Self { meta, item_impl }
    }

    pub fn expand(&mut self) -> syn::Result<proc_macro2::TokenStream> {
        self.check_impl_signature()?;
        self.check_and_make_methods_private()?;

        let ident = self.ident()?;
        let config = self.meta.to_config(ident)?;

        let (methods, inputs) = self.gather_methods_and_inputs()?;

        let macro_def = self.define_macro(ident, &config, &inputs);

        let query_enum = Self::define_query_enum(&config, &methods);
        let take_query_impl = Self::impl_take_query(ident, &config, &methods, &inputs)?;
        let connection_struct = Self::define_connection_struct(ident, &config, &methods);

        let mut message_impls = quote! {
            #query_enum
            #take_query_impl
            #connection_struct
        };

        if let Some(config) = &config.transaction {
            let transaction_struct = Self::define_transaction_struct(config);
            let ref_struct = Self::define_ref_struct(ident, config, &methods, &inputs);
            let take_transaction_impl = Self::impl_take_transaction(ident, config, inputs)?;

            message_impls.extend(quote! {
                #transaction_struct
                #ref_struct
                #take_transaction_impl
            });
        }

        if config.get_ref {
            message_impls.extend(Self::impl_get_ref(ident, &config));
        }

        let item_impl = &self.item_impl;

        Ok(quote! {
            #[allow(clippy::needless_pass_by_value)]
            #item_impl

            #macro_def

            #message_impls
        })
    }

    fn check_impl_signature(&self) -> syn::Result<()> {
        let item_impl = &self.item_impl;

        // TODO: support generics
        // This is difficult, since you'd have to check if the generics are used in the Query enum.
        // But, it isn't possible, so it's up to be added in the future.
        if !item_impl.generics.params.is_empty() || item_impl.generics.where_clause.is_some() {
            Err(syn::Error::new_spanned(
                &item_impl.generics,
                "generics are not supported yet, file an issue if you find a use-case",
            ))
        } else if let Some((_, _, for_kw)) = item_impl.trait_.as_ref() {
            Err(syn::Error::new_spanned(for_kw, "expected no trait here"))
        } else {
            Ok(())
        }
    }

    fn check_and_make_methods_private(&mut self) -> syn::Result<()> {
        for item in &mut self.item_impl.items {
            match item {
                ImplItem::Fn(item) => item.vis = Visibility::Inherited,
                item => {
                    return Err(syn::Error::new_spanned(item, "expected only methods"));
                }
            }
        }

        Ok(())
    }

    fn ident(&self) -> syn::Result<&Ident> {
        match &*self.item_impl.self_ty {
            Type::Path(path) => path.path.require_ident(),
            _ => Err(syn::Error::new_spanned(
                &self.item_impl.self_ty,
                "expected an identifier",
            )),
        }
    }

    fn gather_methods_and_inputs(&self) -> syn::Result<(Vec<Method>, InputList)> {
        let mut inputs = InputList::default();
        let methods = self
            .item_impl
            .items
            .iter()
            .map(|item| match item {
                ImplItem::Fn(item) => item,
                _ => unreachable!(),
            })
            .map(|method| Method::new(method, &mut inputs))
            .collect::<syn::Result<_>>()?;
        Ok((methods, inputs))
    }

    fn define_query_enum(config: &Config<'_>, methods: &[Method]) -> proc_macro2::TokenStream {
        let auditeur = &*crate::AUDITEUR;

        let mut variants = proc_macro2::TokenStream::new();
        for method in methods {
            let variant = &method.camel_case_ident;
            let ident = method.args.iter().map(|arg| arg.ident);
            let ty = method.args.iter().map(|arg| arg.ty);
            let return_type = method.return_type_to_token_stream();
            let response = format_ident!("{RESPONSE}");
            let docs = &method.docs;

            variants.extend(quote! {
                #(#docs)*
                #variant {
                    #response: #auditeur::tokio::sync::oneshot::Sender<#return_type>,
                    #(#ident: #ty),*
                },
            });
        }

        let ident = &config.query.message_ident;
        let id = &config.query.message_id;

        let message_impl = super::derive_message(ident, id, &CustomGenerics::default());

        quote! {
            #[non_exhaustive]
            pub enum #ident {
                #variants
            }

            #message_impl
        }
    }

    fn impl_take_query(
        item_ident: &Ident,
        config: &Config<'_>,
        methods: &[Method],
        input_list: &InputList,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let auditeur = &*crate::AUDITEUR;

        let message_ident = &config.query.message_ident;

        let response = format_ident!("{RESPONSE}");

        let mut variants = proc_macro2::TokenStream::new();
        for method in methods {
            let variant = &method.camel_case_ident;
            let method_ident = method.snake_case_ident;
            let arg0 = method.args.iter().map(|arg| arg.ident);
            let arg1 = arg0.clone();
            let inputs = method
                .combination
                .selections
                .iter()
                .zip(input_idents())
                .filter(|(selection, _)| selection.in_set)
                .map(|(selection, ident)| Ident::new(&ident, selection.span));

            let maybe_await = method
                .asyncness
                .map(|token| quote_spanned! { token.span() => .await })
                .unwrap_or_default();

            variants.extend(quote! {
                #message_ident::#variant { #response, #(#arg0),* } => {
                    let result = self.#method_ident(#(#arg1,)* (#(#inputs),*)) #maybe_await;
                    let _ = #response.send(result);
                }
            });
        }

        let input_macro_ident = super::input_reflection::input_macro_ident_of(
            &parse_quote!(Take<#message_ident>),
            &parse_quote!(#item_ident),
        )?;
        let input_macro = super::input_reflection::export(&input_macro_ident, input_list);

        let input_reference_list = input_list.clone().into_reference_list().with_lifetime("'a");

        let inputs = input_idents()
            .take(input_list.len())
            .map(|ident| Ident::new(&ident, Span::call_site()));

        Ok(quote! {
            #input_macro

            impl #auditeur::resource::listener::Take<#message_ident> for #item_ident {
                type OnInput<'a> = #input_reference_list;

                async fn on(&mut self, query: #message_ident, (#(#inputs),*): Self::OnInput<'_>) {
                    match query {
                        #variants
                    }
                }
            }
        })
    }

    fn define_connection_struct(
        item_ident: &Ident,
        config: &Config<'_>,
        methods: &[Method],
    ) -> proc_macro2::TokenStream {
        let auditeur = &*crate::AUDITEUR;

        let message_ident = &config.query.message_ident;

        let mut fields = quote! {
            giver: #auditeur::resource::listener::Giver<#item_ident, #message_ident>,
        };

        let mut create_fields = quote! {
            giver: #auditeur::resource::listener::Giver::new(registry),
        };

        let response = format_ident!("{RESPONSE}");

        let mut methods_out = proc_macro2::TokenStream::new();
        for method in methods {
            let variant = &method.camel_case_ident;
            let method_ident = method.snake_case_ident;
            let arg0 = method.args.iter().map(|arg| arg.ident);
            let arg1 = arg0.clone();
            let ty = method.args.iter().map(|arg| arg.ty);
            let return_type = method.return_type_to_token_stream();
            let docs = &method.docs;

            methods_out.extend(quote! {
                #(#docs)*
                pub async fn #method_ident(&self, #(#arg0: #ty),*) -> #return_type {
                    let (#response, receiver) = #auditeur::tokio::sync::oneshot::channel();
                    let query = #message_ident::#variant { #response, #(#arg1),* };
                    self.giver.give(query).await;
                    receiver.await.expect(stringify!(#item_ident, "::", #method_ident, " panicked"))
                }
            });
        }

        if let Some(transaction) = config.transaction.as_ref() {
            let transaction_message = &transaction.message_ident;
            let transaction_ref = &transaction.ref_ident;

            fields.extend(quote! {
                transactions: #auditeur::resource::listener::Giver<#item_ident, #transaction_message>,
            });

            create_fields.extend(quote! {
                transactions: #auditeur::resource::listener::Giver::new(registry),
            });

            methods_out.extend(quote! {
                pub async fn run_transaction<T: Send + Sync + 'static>(
                    &self,
                    transaction: impl for<'a> FnOnce(#transaction_ref<'a>) -> T + Send + Sync + 'static,
                ) -> T {
                    let (response, receiver) = #auditeur::tokio::sync::oneshot::channel();
                    self.spawn_transaction(move |reference| {
                        let res = transaction(reference);
                        let _ = response.send(res);
                    })
                    .await;
                    receiver.await.expect(stringify!(#item_ident, " transaction panicked"))
                }

                pub async fn spawn_transaction(
                    &self,
                    transaction: impl for<'a> FnOnce(#transaction_ref<'a>) + Send + Sync + 'static,
                ) {
                    self.transactions
                        .give(#transaction_message::from(transaction))
                        .await;
                }
            });
        }

        let ref_ident = &config.query.ref_ident;

        quote! {
            #[derive(Clone)]
            pub struct #ref_ident { #fields }

            impl #auditeur::input::Input for #ref_ident {
                async fn create(registry: &#auditeur::app::Registry, _: &NamespacedIdRef) -> Result<Self, #auditeur::Error> {
                    Ok(Self::new(registry))
                }

                fn constraints() -> #auditeur::app::Constraints {
                    #auditeur::app::Constraints::empty()
                }
            }

            #[allow(clippy::missing_panics_doc)]
            impl #ref_ident {
                fn new(registry: &#auditeur::app::Registry) -> Self {
                    Self { #create_fields }
                }

                #methods_out
            }
        }
    }

    fn define_transaction_struct(config: &TakeConfig) -> proc_macro2::TokenStream {
        let TakeConfig {
            message_ident,
            message_id,
            ref_ident,
        } = config;

        let message_impl =
            super::derive_message(message_ident, message_id, &CustomGenerics::default());

        quote! {
            pub struct #message_ident {
                func: Box<dyn for<'a> FnOnce(#ref_ident<'a>) + Send + Sync>,
            }

            #message_impl

            impl<F: for<'a> FnOnce(#ref_ident<'a>) + Send + Sync + 'static> From<F> for #message_ident {
                fn from(value: F) -> Self {
                    Self {
                        func: Box::new(value),
                    }
                }
            }
        }
    }

    fn impl_take_transaction(
        item_ident: &Ident,
        config: &TakeConfig,
        input_list: InputList,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let TakeConfig {
            message_ident,
            ref_ident,
            ..
        } = config;

        let input_macro_ident = super::input_reflection::input_macro_ident_of(
            &parse_quote!(Take<#message_ident>),
            &parse_quote!(#item_ident),
        )?;
        let input_macro = super::input_reflection::export(&input_macro_ident, &input_list);

        let input_reference_list = input_list.into_reference_list().with_lifetime("'a");

        let auditeur = &*crate::AUDITEUR;

        Ok(quote! {
            impl #auditeur::resource::listener::Take<#message_ident> for #item_ident {
                type OnInput<'a> = #input_reference_list;

                async fn on(&mut self, transaction: #message_ident, input: Self::OnInput<'_>) {
                    let reference = #ref_ident {
                        deref: self,
                        input,
                    };
                    (transaction.func)(reference);
                }
            }

            #input_macro
        })
    }

    fn define_ref_struct(
        item_ident: &Ident,
        config: &TakeConfig,
        methods: &[Method],
        input_list: &InputList,
    ) -> proc_macro2::TokenStream {
        let TakeConfig { ref_ident, .. } = config;

        let mut methods_out = proc_macro2::TokenStream::new();
        for method in methods {
            let method_ident = method.snake_case_ident;
            let arg0 = method.args.iter().map(|arg| arg.ident);
            let arg1 = arg0.clone();
            let ty = method.args.iter().map(|arg| arg.ty);
            let return_type = method.return_type_to_token_stream();
            let mutability = method.mutability;
            let docs = &method.docs;

            let input = method
                .combination
                .selections
                .iter()
                .enumerate()
                .filter(|(_, selection)| selection.in_set)
                .map(|(i, selection)| format_ident!("self.input.{i}", span = selection.span));

            methods_out.extend(quote! {
                #(#docs)*
                #[must_use]
                pub fn #method_ident(& #mutability self, #(#arg0: #ty),*) -> #return_type {
                    self.deref.#method_ident(#(#arg1),*, (#(#input),*))
                }
            });
        }

        let input_list = input_list.clone().into_reference_list().with_lifetime("'a");

        quote! {
            pub struct #ref_ident<'a> {
                deref: &'a mut #item_ident,
                #[allow(dead_code)]
                input: #input_list,
            }

            impl #ref_ident<'_> { #methods_out }
        }
    }

    fn define_macro(
        &self,
        item_ident: &Ident,
        config: &Config,
        input_list: &InputList,
    ) -> proc_macro2::TokenStream {
        let macro_ident = format_ident!("auditeur_respond_impl_of_{item_ident}");

        let meta = &self.meta;

        let mut input = quote! { #input_list };
        if config.transaction.is_some() {
            input_list.to_tokens(&mut input);
        }

        quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, clippy::redundant_pub_crate)]
            #[allow(unused_imports)]
            pub(crate) mod #macro_ident {
                #[macro_export]
                #[doc(hidden)]
                macro_rules! #macro_ident {
                    (($callback:path $(, $($callbacks:tt)*)?) ($($params:tt)*) () $($rest:tt)*) => {
                        $callback!(($($($callbacks)*)?) ($($params)*) (#meta) $($rest)* #input);
                    };
                }
                pub use #macro_ident;
            }
        }
    }

    fn impl_get_ref(item_ident: &Ident, config: &Config) -> proc_macro2::TokenStream {
        let auditeur = &*crate::AUDITEUR;
        let connection = &config.query.ref_ident;

        #[cfg(feature = "path-any")]
        let any_impl = super::path_any(connection, &CustomGenerics::default());
        #[cfg(not(feature = "path-any"))]
        let any_impl = quote! {};

        quote! {
            #any_impl

            impl #auditeur::input::get::GetRef for #item_ident {
                type Ref = #connection;

                fn get_ref(&self, registry: &#auditeur::app::Registry) -> Self::Ref {
                    #connection::new(registry)
                }
            }
        }
    }
}

struct Method<'a> {
    snake_case_ident: &'a Ident,
    camel_case_ident: Ident,
    args: Vec<Arg<'a>>,
    combination: Combination,
    return_type: Option<&'a Type>,
    asyncness: Option<Token![async]>,
    mutability: Option<Token![mut]>,
    docs: Vec<&'a Attribute>,
    // timeout: Option<Expr>,
    // on_error: MethodOnError,
}

struct Arg<'a> {
    ident: &'a Ident,
    ty: &'a Type,
}

// enum MethodOnError {
//     Panic,
//     Func(Expr),
//     Propagate,
// }

impl<'a> Method<'a> {
    pub fn new(item: &'a ImplItemFn, all_inputs: &mut InputList) -> syn::Result<Self> {
        let snake_case_ident = &item.sig.ident;
        let camel_case_ident = Ident::new(
            &snake_to_upper_camel_case(&snake_case_ident.to_string()),
            snake_case_ident.span(),
        );

        let mut typed_args = item.sig.inputs.iter().filter_map(|input| match input {
            FnArg::Typed(typed) => Some(typed),
            FnArg::Receiver(_) => None,
        });

        let Some(input_list) = typed_args.next_back() else {
            return Err(syn::Error::new_spanned(
                &item.sig.inputs,
                "expected at least one typed argument to specify inputs",
            ));
        };

        let input_list =
            syn::parse2::<InputReferenceList>(input_list.ty.to_token_stream())?.into_owned_list();

        let combination = all_inputs.merge(&input_list)?;

        let args: Vec<Arg> = typed_args
            .map(|arg| match &*arg.pat {
                Pat::Ident(pat) if pat.ident != RESPONSE => Ok(Arg {
                    ident: &pat.ident,
                    ty: &arg.ty,
                }),
                _ => Err(syn::Error::new_spanned(
                    &arg.pat,
                    format!("expected any pattern that is not the identifier '{RESPONSE}'"),
                )),
            })
            .collect::<syn::Result<_>>()?;

        let return_type = match &item.sig.output {
            ReturnType::Default => None,
            ReturnType::Type(_, typ) => Some(&**typ),
        };

        let asyncness = item.sig.asyncness;
        let mutability = item
            .sig
            .inputs
            .iter()
            .find_map(|input| match input {
                FnArg::Receiver(arg) => Some(arg.mutability),
                FnArg::Typed(_) => None,
            })
            .flatten();

        let docs = item
            .attrs
            .iter()
            .filter(|item| item.path().is_ident("doc"))
            .collect();

        Ok(Self {
            snake_case_ident,
            camel_case_ident,
            args,
            combination,
            return_type,
            asyncness,
            mutability,
            docs,
            // TODO: read below config from attributes:
            // #[auditeur(timeout = Duration::from_secs(2), on_error = panic)]
            // where on_error can be panic, default, func(|| _), or propagate
            // timeout: None,
            // on_error: MethodOnError::Panic,
        })
    }

    fn return_type_to_token_stream(&self) -> proc_macro2::TokenStream {
        self.return_type
            .map_or_else(|| quote! { () }, ToTokens::to_token_stream)
    }
}

pub struct Config<'a> {
    pub query: TakeConfig<'a>,
    pub transaction: Option<TakeConfig<'a>>,
    pub get_ref: bool,
}

pub struct TakeConfig<'a> {
    pub message_ident: Cow<'a, Ident>,
    pub message_id: Cow<'a, NamespacedId>,
    pub ref_ident: Cow<'a, Ident>,
}

#[derive(Clone, Copy)]
struct TakeDefaultSuffixes {
    message_ident: &'static str,
    message_id: &'static str,
    ref_ident: &'static str,
}

impl TakeDefaultSuffixes {
    const QUERY: Self = Self {
        message_ident: "Query",
        message_id: "query",
        ref_ident: "Connection",
    };

    const TRANSACTION: Self = Self {
        message_ident: "Transaction",
        message_id: "transaction",
        ref_ident: "Ref",
    };
}

pub struct Meta {
    id: Option<Comma<NamespacedId>>,
    params: Punctuated<MetaParam, Token![,]>,
}

impl Meta {
    pub fn to_config(&self, item_ident: &Ident) -> syn::Result<Config> {
        let query = self.to_take_config(
            self.params.iter().filter_map(MetaParam::as_query).flatten(),
            item_ident,
            TakeDefaultSuffixes::QUERY,
            "query",
        )?;

        let transaction = self
            .params
            .iter()
            .any(MetaParam::is_transaction)
            .then(|| {
                self.to_take_config(
                    self.params
                        .iter()
                        .filter_map(MetaParam::as_transaction)
                        .flatten(),
                    item_ident,
                    TakeDefaultSuffixes::TRANSACTION,
                    "transaction",
                )
            })
            .transpose()?;

        let get_ref = self.params.iter().any(MetaParam::is_get_ref);

        Ok(Config {
            query,
            transaction,
            get_ref,
        })
    }

    fn id(&self) -> Option<&NamespacedId> {
        self.id.as_ref().map(|id| &id.value)
    }

    fn to_take_config<'a>(
        &self,
        params: impl Iterator<Item = &'a TakeTypesParam>,
        item_ident: &Ident,
        defaults: TakeDefaultSuffixes,
        param_name: &'static str,
    ) -> syn::Result<TakeConfig<'a>> {
        let (message_ident, message_id, ref_ident, ref_id) = params.fold(
            (None, None, None, None),
            |(message_ident, message_id, ref_ident, ref_id), param| {
                (
                    param.message_ident().or(message_ident),
                    param.message_id().or(message_id),
                    param.ref_ident().or(ref_ident),
                    param.ref_id().or(ref_id),
                )
            },
        );

        let message_ident = message_ident.map_or_else(
            || Cow::Owned(format_ident!("{item_ident}{}", defaults.message_ident)),
            Cow::Borrowed,
        );

        let message_id = message_id
            .map(Cow::Borrowed)
            .or_else(|| {
                self.id().map(|id| {
                    Cow::Owned(NamespacedId {
                        namespace: id.namespace.clone(),
                        id: format!("{}_{}", id.id, defaults.message_id),
                    })
                })
            })
            .ok_or_else(|| {
                syn::Error::new(
                    self.span(),
                    format!(
                        "expected a namespaced id for the {param_name}'s message. \
                    Try specifying '{param_name}(message_id = ...)' \
                    or use the default by passing in the resource's namespaced id \
                    as the first argument (ex: #[respond(\"namespace:id\")])"
                    ),
                )
            })?;

        let ref_ident = ref_ident.map_or_else(
            || Cow::Owned(format_ident!("{item_ident}{}", defaults.ref_ident)),
            Cow::Borrowed,
        );

        if ref_id.is_some() {
            return Err(syn::Error::new(
                self.span(),
                "specifying an id for the reference does nothing",
            ));
        }

        Ok(TakeConfig {
            message_ident,
            message_id,
            ref_ident,
        })
    }
}

impl Parse for Meta {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let id = input
            .peek(LitStr)
            .then(|| Comma::parse(input))
            .transpose()?;
        let params = id
            .as_ref()
            .is_none_or(Comma::has_comma)
            .then(|| input.parse_terminated(MetaParam::parse, Token![,]))
            .transpose()?
            .unwrap_or_default();
        Ok(Self { id, params })
    }
}

impl ToTokens for Meta {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.id.to_tokens(tokens);
        self.params.to_tokens(tokens);
    }
}

enum MetaParam {
    Query(List<TakeTypesParam>),
    Transaction(List<TakeTypesParam>),
    TransactionAutoFlag(Ident),
    GetRefFlag(Ident),
}

impl MetaParam {
    const fn as_query(&self) -> Option<&List<TakeTypesParam>> {
        if let Self::Query(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_transaction(&self) -> Option<&List<TakeTypesParam>> {
        if let Self::Transaction(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn is_transaction(&self) -> bool {
        matches!(self, Self::Transaction(_) | Self::TransactionAutoFlag(_))
    }

    /// Returns `true` if the meta param is [`GetRefFlag`].
    ///
    /// [`GetRefFlag`]: MetaParam::GetRefFlag
    #[must_use]
    const fn is_get_ref(&self) -> bool {
        matches!(self, Self::GetRefFlag(..))
    }
}

impl Parse for MetaParam {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "query" => List::parse_after(ident, input).map(Self::Query),
            "transaction" => {
                if input.peek(token::Paren) {
                    List::parse_after(ident, input).map(Self::Transaction)
                } else {
                    Ok(Self::TransactionAutoFlag(ident))
                }
            }
            "get_ref" => Ok(Self::GetRefFlag(ident)),
            other => Err(input.error(format!("expected 'query' or 'transaction', found {other}"))),
        }
    }
}

impl ToTokens for MetaParam {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Query(list) | Self::Transaction(list) => list.to_tokens(tokens),
            Self::TransactionAutoFlag(ident) | Self::GetRefFlag(ident) => ident.to_tokens(tokens),
        }
    }
}

enum TakeTypesParam {
    MessageParams(List<ItemParam>),
    MessageIdent(NameValue<Ident>),
    RefParams(List<ItemParam>),
    RefIdent(NameValue<Ident>),
}

impl TakeTypesParam {
    fn message_ident(&self) -> Option<&Ident> {
        match self {
            Self::MessageParams(list) => list.iter().filter_map(ItemParam::ident).next_back(),
            Self::MessageIdent(name_value) => Some(name_value),
            _ => None,
        }
    }

    fn message_id(&self) -> Option<&NamespacedId> {
        match self {
            Self::MessageParams(list) => list.iter().filter_map(ItemParam::id).next_back(),
            _ => None,
        }
    }

    fn ref_ident(&self) -> Option<&Ident> {
        match self {
            Self::RefParams(list) => list.iter().filter_map(ItemParam::ident).next_back(),
            Self::RefIdent(name_value) => Some(name_value),
            _ => None,
        }
    }

    fn ref_id(&self) -> Option<&NamespacedId> {
        match self {
            Self::RefParams(list) => list.iter().filter_map(ItemParam::id).next_back(),
            _ => None,
        }
    }
}

impl Parse for TakeTypesParam {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "message" => {
                let lookahead = input.lookahead1();
                if lookahead.peek(token::Paren) {
                    List::parse_after(ident, input).map(Self::MessageParams)
                } else if lookahead.peek(Token![=]) {
                    NameValue::parse_after(ident, input).map(Self::MessageIdent)
                } else {
                    Err(lookahead.error())
                }
            }
            "ref" => {
                let lookahead = input.lookahead1();
                if lookahead.peek(token::Paren) {
                    List::parse_after(ident, input).map(Self::RefParams)
                } else if lookahead.peek(Token![=]) {
                    NameValue::parse_after(ident, input).map(Self::RefIdent)
                } else {
                    Err(lookahead.error())
                }
            }
            other => Err(input.error(format!(
                "expected 'auto', 'message', or 'ref', found {other}"
            ))),
        }
    }
}

impl ToTokens for TakeTypesParam {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::MessageParams(list) | Self::RefParams(list) => list.to_tokens(tokens),
            Self::MessageIdent(name_value) | Self::RefIdent(name_value) => {
                name_value.to_tokens(tokens);
            }
        }
    }
}

enum ItemParam {
    Ident(NameValue<Ident>),
    Id(NameValue<NamespacedId>),
}

impl ItemParam {
    fn ident(&self) -> Option<&Ident> {
        match self {
            Self::Ident(name_value) => Some(name_value),
            Self::Id(_) => None,
        }
    }

    fn id(&self) -> Option<&NamespacedId> {
        match self {
            Self::Id(name_value) => Some(name_value),
            Self::Ident(_) => None,
        }
    }
}

impl Parse for ItemParam {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "ident" => NameValue::parse_after(ident, input).map(Self::Ident),
            "id" => NameValue::parse_after(ident, input).map(Self::Id),
            other => Err(input.error(format!("expected 'ident' or 'id', found {other}"))),
        }
    }
}

impl ToTokens for ItemParam {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Ident(name_value) => name_value.to_tokens(tokens),
            Self::Id(name_value) => name_value.to_tokens(tokens),
        }
    }
}
