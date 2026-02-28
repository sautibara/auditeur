use std::{borrow::Cow, collections::HashMap};

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Expr, FnArg, GenericParam, Generics, Ident, ImplItem, ImplItemFn, ItemImpl,
    Lifetime, LitStr, Pat, ReturnType, Token, Type, Visibility, braced,
    parse::Parse,
    parse_quote,
    punctuated::Punctuated,
    spanned::Spanned,
    token::{self, Brace},
    visit::Visit,
};

use crate::{
    custom_generics::CustomGenerics,
    field::{Comma, List, NameValue, Parenthesized},
    input_list::{Combination, InputList, InputReferenceList},
    namespaced_id::NamespacedId,
};

pub fn macro_ident(item_ident: &Ident) -> Ident {
    format_ident!("auditeur_respond_impl_of_{item_ident}")
}

// #[respond(
//     "core:library",
//     query(
//         message_ident = LibraryQuery,
//         message_id = "core:library_query",
//         ref_ident = LibraryConnection,
//     ),
//     transaction(
//         message_ident = LibraryTransaction,
//         message_id = "core:library_transaction",
//         ref_ident = LibraryRef,
//     ),
//     fields = |registry| {
//         field_name: field_type = field_constructor,
//     }
// )]

// #[respond(
//     connection_fn,
//     ref_fn,
//     timeout = Duration::from_secs(2),
//     on_error = panic,
// )]

const RESPONSE: &str = "response";

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

        let (visibilities, raw_fns) = self.mutate_methods()?;
        let (methods, inputs) = self.gather_methods_and_inputs(visibilities)?;

        let generics = &self.item_impl.generics;
        let ident = self.ident()?;
        let id_expr = self.id_expr(ident)?;
        let config = self.meta.to_config(ident)?;

        let hidden = Self::hidden(&methods, &raw_fns);

        let macro_def = Self::define_macro(ident, &config, &inputs);
        let respond_impl = Self::impl_respond(ident, &config, &generics);

        let query_enum = Self::define_query_enum(&config, &methods, hidden.as_ref(), generics)?;
        let take_query_impl = Self::impl_take_query(ident, &config, &methods, &inputs, generics)?;
        let connection_struct = Self::define_connection_struct(
            ident,
            &config,
            &methods,
            &raw_fns,
            hidden.as_ref(),
            &id_expr,
        );

        let mut message_impls = quote! {
            #query_enum
            #take_query_impl
            #connection_struct
        };

        if let Some(config) = &config.transaction {
            if !generics.params.is_empty() || generics.where_clause.is_some() {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "generics are not supported with transactions yet - please file an issue if you\
 find a usecase",
                ));
            }

            let transaction_struct = Self::define_transaction_struct(config, hidden.as_ref());
            let ref_struct = Self::define_ref_struct(
                ident,
                config,
                &methods,
                &inputs,
                &raw_fns,
                hidden.as_ref(),
            );
            let take_transaction_impl = Self::impl_take_transaction(ident, config, inputs)?;

            message_impls.extend(quote! {
                #transaction_struct
                #ref_struct
                #take_transaction_impl
            });
        }

        if config.get_ref {
            message_impls.extend(Self::impl_get_ref(ident, &config, generics));
        }

        let item_impl = &self.item_impl;

        Ok(quote! {
            #[allow(clippy::needless_pass_by_value)]
            #item_impl

            #macro_def
            #respond_impl

            #message_impls
        })
    }

    fn check_impl_signature(&self) -> syn::Result<()> {
        let item_impl = &self.item_impl;

        if let Some((_, _, for_kw)) = item_impl.trait_.as_ref() {
            Err(syn::Error::new_spanned(for_kw, "expected no trait here"))
        } else {
            Ok(())
        }
    }

    fn ident(&self) -> syn::Result<&Ident> {
        match &*self.item_impl.self_ty {
            Type::Path(path) => Ok(&path
                .path
                .segments
                .first()
                .ok_or_else(|| syn::Error::new_spanned(path, "expected at least one ident"))?
                .ident),
            _ => Err(syn::Error::new_spanned(
                &self.item_impl.self_ty,
                "expected an identifier",
            )),
        }
    }

    fn id_expr(&self, item_ident: &Ident) -> syn::Result<TokenStream> {
        let auditeur = &*crate::AUDITEUR;
        if self.item_impl.generics.params.is_empty()
            && self.item_impl.generics.where_clause.is_none()
        {
            Ok(quote! { <#item_ident as #auditeur::resource::Identifiable>::const_ident() })
        } else if let Some(id) = self.meta.id() {
            Ok(quote! { #id })
        } else {
            Err(syn::Error::new(
                Span::call_site(),
                "expected id to be specified with generics",
            ))
        }
    }

    fn mutate_methods(&mut self) -> syn::Result<(Vec<Visibility>, RawFns)> {
        let mut raw_fns = Vec::new();
        let iter = self
            .item_impl
            .items
            .drain(..)
            .filter_map(|item| match item {
                ImplItem::Fn(method) => Self::mutate_method(method, &mut raw_fns).transpose(),
                _ => Some(Err(syn::Error::new_spanned(item, "expected a function"))),
            });
        let (items, visibilities) = iter.collect::<syn::Result<_>>()?;
        self.item_impl.items = items;
        Ok((visibilities, RawFns { raw_fns }))
    }

    fn mutate_method(
        mut method: ImplItemFn,
        raw_fns: &mut Vec<(ImplItemFn, RawFnType)>,
    ) -> syn::Result<Option<(ImplItem, Visibility)>> {
        let (attrs, config) = method.attrs.drain(..).try_fold(
            (Vec::new(), MethodAttributes::default()),
            |(mut attrs, mut config), attr| {
                if attr.path().is_ident("respond") {
                    let args: MethodAttributes = attr.parse_args()?;
                    config.attributes.extend(args.attributes.into_pairs());
                } else {
                    attrs.push(attr);
                }
                syn::Result::Ok((attrs, config))
            },
        )?;
        method.attrs = attrs;
        match config.into_config()? {
            MethodAttributeConfig::RawFn(raw_fn) => {
                raw_fns.push((method, raw_fn));
                Ok(None)
            }
            MethodAttributeConfig::Method { timeout, on_error } => {
                if timeout.is_some() || !on_error.is_default() {
                    todo!("timeout and on_error support");
                }
                let vis = std::mem::replace(&mut method.vis, Visibility::Inherited);
                method.attrs.push(parse_quote!(
                    #[allow(clippy::needless_pass_by_ref_mut)]
                ));
                Ok(Some((ImplItem::Fn(method), vis)))
            }
        }
    }

    fn gather_methods_and_inputs(
        &self,
        visibilities: Vec<Visibility>,
    ) -> syn::Result<(Vec<Method<'_>>, InputList)> {
        let mut inputs = InputList::default();
        let methods = (self.item_impl.items.iter())
            .zip(visibilities)
            .map(|(item, visibility)| match item {
                ImplItem::Fn(method) => Method::new(method, visibility, &mut inputs),
                _ => unreachable!(),
            })
            .collect::<syn::Result<_>>()?;
        Ok((methods, inputs))
    }

    fn private_vis(private: bool) -> Option<proc_macro2::TokenStream> {
        if private {
            Some(quote! {
                #[doc(hidden)]
                #[allow(private_interfaces)]
            })
        } else {
            None
        }
    }

    fn hidden(methods: &[Method], raw_fns: &RawFns) -> Option<proc_macro2::TokenStream> {
        let all_private = (methods.iter().map(|method| &method.vis))
            .chain(raw_fns.iter().map(|method| &method.vis))
            .all(|vis| matches!(vis, Visibility::Inherited | Visibility::Restricted(_)));
        Self::private_vis(all_private)
    }

    fn define_query_enum(
        config: &Config<'_>,
        methods: &[Method],
        hidden: Option<&proc_macro2::TokenStream>,
        generics: &Generics,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let auditeur = &*crate::AUDITEUR;

        let mut search_types = SearchTypes::from_generics(generics);

        let mut variants = proc_macro2::TokenStream::new();
        for method in methods {
            let variant = &method.camel_case_ident;
            let ident = method.args.iter().map(|arg| arg.ident);
            let ty = method.args.iter().map(|arg| arg.ty);
            let ty2 = ty.clone();
            let return_type = method.return_type_to_token_stream();
            let response = format_ident!("{RESPONSE}");
            let docs = &method.docs;

            let vis = Self::private_vis(matches!(
                method.vis,
                Visibility::Inherited | Visibility::Restricted(_)
            ));

            for ty in ty2 {
                search_types.visit_type(ty);
            }

            if let Some(return_type) = method.return_type {
                search_types.visit_type(return_type);
            }

            variants.extend(quote! {
                #(#docs)*
                #[allow(rustdoc::broken_intra_doc_links)]
                #vis #variant {
                    #response: #auditeur::tokio::sync::oneshot::Sender<#return_type>,
                    #(#ident: #ty),*
                },
            });
        }

        if search_types.any() {
            return Err(syn::Error::new(
                Span::call_site(),
                "generics are not supported in method arguments or return types with #[respond]. \
This error may be relaxed in the future - please file an issue if you find a usecase",
            ));
        }

        let ident = &config.query.message_ident;
        let id = &config.query.message_id;

        let message_impl = super::derive_message(ident, id, &CustomGenerics::default());

        Ok(quote! {
            #[non_exhaustive]
            #hidden
            pub enum #ident {
                #variants
            }

            #message_impl
        })
    }

    fn impl_take_query(
        item_ident: &Ident,
        config: &Config<'_>,
        methods: &[Method],
        input_list: &InputList,
        generics: &Generics,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let auditeur = &*crate::AUDITEUR;
        let anyhow = auditeur.anyhow();

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

            let mut call = quote! { self.#method_ident(#(#arg1,)* (#(#inputs),*)) };

            if let Some(async_kw) = method.asyncness {
                let await_kw = Token![await](async_kw.span);
                call = quote! { #call . #await_kw };
            }

            if let Some(unsafe_kw) = method.unsafety {
                call = quote! { #unsafe_kw { #call } };
            }

            variants.extend(quote! {
                #message_ident::#variant { #response, #(#arg0),* } => {
                    let result = #call;
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

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        Ok(quote! {
            #input_macro

            impl #impl_generics #auditeur::resource::listener::Take<#message_ident> for #item_ident #ty_generics #where_clause {
                type OnInput<'a> = #input_reference_list;

                async fn on(&mut self, query: #message_ident, (#(#inputs),*): Self::OnInput<'_>) -> #anyhow::Result<()> {
                    match query {
                        #variants
                    }
                    Ok(())
                }
            }
        })
    }

    // TODO: please for the love of god break this into multiple functions
    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
    fn define_connection_struct(
        item_ident: &Ident,
        config: &Config<'_>,
        methods: &[Method],
        raw_fns: &RawFns,
        hidden: Option<&proc_macro2::TokenStream>,
        id_expr: &TokenStream,
    ) -> proc_macro2::TokenStream {
        let auditeur = &*crate::AUDITEUR;

        let message_ident = &config.query.message_ident;

        let mut field_idents = vec![Ident::new("giver", Span::call_site())];
        let mut field_types = vec![parse_quote!(#auditeur::app::channels::Giver<#message_ident>)];

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
            let vis = &method.vis;
            let unsafety = method.unsafety;

            // FIXME: self.giver.clone_erased used, this might be dangerous
            methods_out.extend(quote! {
                #(#docs)*
                #vis #unsafety fn #method_ident(&self, #(#arg0: #ty),*) -> impl std::future::Future<Output = #return_type> + 'static {
                    let giver = self.giver.clone_erased();
                    async move {
                        let (#response, auditeur_receiver) = #auditeur::tokio::sync::oneshot::channel();
                        let query = #message_ident::#variant { #response, #(#arg1),* };
                        giver.give(query).await;
                        auditeur_receiver.await.expect(concat!("respond: ", stringify!(#item_ident), "::", stringify!(#method_ident), " panicked"))
                    }
                }
            });
        }

        for method in raw_fns.connection_fns() {
            method.to_tokens(&mut methods_out);
        }

        if let Some(transaction) = config.transaction.as_ref() {
            let transaction_message = &transaction.message_ident;
            let transaction_ref = &transaction.ref_ident;

            field_idents.push(Ident::new("transactions", Span::call_site()));
            field_types.push(parse_quote!(#auditeur::app::channels::Giver<#transaction_message>));

            let vis = if hidden.is_some() {
                quote! { pub(crate) }
            } else {
                quote! { pub }
            };

            methods_out.extend(quote! {
                #vis async fn run_transaction<T: Send + Sync + 'static>(
                    &self,
                    transaction: impl for<'a> FnOnce(#transaction_ref<'a>) -> T + Send + Sync + 'static,
                ) -> T {
                    let (response, receiver) = #auditeur::tokio::sync::oneshot::channel();
                    self.spawn_transaction(move |reference| {
                        let res = transaction(reference);
                        let _ = response.send(res);
                    })
                    .await;
                    receiver.await.expect(concat!(stringify!(#item_ident), " transaction panicked"))
                }

                #vis async fn spawn_transaction(
                    &self,
                    transaction: impl for<'a> FnOnce(#transaction_ref<'a>) + Send + Sync + 'static,
                ) {
                    self.transactions
                        .give(#transaction_message::from(transaction))
                        .await;
                }
            });
        }

        if let Some(fields) = &config.fields {
            for field in &fields.list {
                let Field { ident, ty, .. } = &field;
                field_idents.push((*ident).clone());
                field_types.push(ty.clone());
            }
        }

        let ref_ident = &config.query.ref_ident;

        let new_docs = format!(
            "Create a new [`{ref_ident}`] using a [`Registry`]({}::app::Registry).",
            auditeur.to_token_stream().to_string().replace(' ', "")
        );

        let resource_ref_impl = if cfg!(feature = "runtime-loading") {
            #[cfg(feature = "path-any")]
            let any_impl = super::path_any(ref_ident, &CustomGenerics::default());
            #[cfg(not(feature = "path-any"))]
            let any_impl = quote! {};

            let dependency_handle_ident = format_ident!("dependency_handle");

            let generics = syn::Generics::default();

            let resource_ref_impl = super::clone_as_struct(
                ref_ident,
                std::iter::once(&dependency_handle_ident).chain(&field_idents),
                &generics,
            );

            quote! {
                #any_impl
                #resource_ref_impl
            }
        } else {
            quote! {}
        };

        quote! {
            #hidden
            pub struct #ref_ident {
                dependency_handle: #auditeur::input::dependency::Handle,
                #(#field_idents: #field_types),*
            }

            impl #auditeur::input::Input for #ref_ident {
                async fn create(
                    registry: &#auditeur::app::Registry,
                    dependent: #auditeur::app::ComponentIdRef<'_>,
                ) -> Result<Self, #auditeur::Error> {
                    Self::new(registry, dependent).await
                }

                fn constraints() -> #auditeur::input::Constraints {
                    #auditeur::input::Constraints::empty()
                        .after_resource(#id_expr)
                        #(.extend(<#field_types as #auditeur::input::Input>::constraints()))*
                }
            }

            impl std::fmt::Debug for #ref_ident {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{} {{ .. }}", stringify!(#ref_ident))
                }
            }

            #resource_ref_impl

            #[allow(clippy::missing_panics_doc)]
            impl #ref_ident {
                #[doc = #new_docs]
                pub async fn new(
                    registry: &#auditeur::app::Registry,
                    dependent: #auditeur::app::ComponentIdRef<'_>,
                ) -> ::std::result::Result<Self, #auditeur::Error> {
                    let event_sender = registry.event_sender();
                    let id = #id_expr.to_owned();
                    let dependency = #auditeur::input::dependency::Full {
                        link: #auditeur::input::dependency::Link {
                            dependency: #auditeur::app::ComponentId::Resource(id),
                            dependent: dependent.into_owned(),
                        },
                        removable: #auditeur::input::dependency::Removable::Inherit,
                    };
                    let dependency_handle = #auditeur::input::dependency::Handle::new(
                        event_sender,
                        dependency,
                    );
                    #[allow(unused_variables)]
                    Ok(Self {
                        dependency_handle,
                        #(#field_idents: <#field_types as #auditeur::input::Input>::create(registry, dependent).await?),*
                    })
                }

                #methods_out
            }
        }
    }

    fn define_transaction_struct(
        config: &TakeConfig,
        hidden: Option<&proc_macro2::TokenStream>,
    ) -> proc_macro2::TokenStream {
        let TakeConfig {
            message_ident,
            message_id,
            ref_ident,
        } = config;

        let message_impl =
            super::derive_message(message_ident, message_id, &CustomGenerics::default());

        quote! {
            #hidden
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
        let anyhow = auditeur.anyhow();

        Ok(quote! {
            impl #auditeur::resource::listener::Take<#message_ident> for #item_ident {
                type OnInput<'a> = #input_reference_list;

                async fn on(&mut self, transaction: #message_ident, input: Self::OnInput<'_>) -> #anyhow::Result<()> {
                    let reference = #ref_ident {
                        deref: self,
                        input,
                    };
                    (transaction.func)(reference);
                    Ok(())
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
        raw_fns: &RawFns,
        hidden: Option<&proc_macro2::TokenStream>,
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
            let vis = &method.vis;
            let asyncness = &method.asyncness;
            let maybe_await = method.maybe_await();

            let input = method
                .combination
                .selections
                .iter()
                .enumerate()
                .filter(|(_, selection)| selection.in_set)
                .map(|(i, selection)| format_ident!("self.input.{i}", span = selection.span));

            let mut body = quote! {
                self.deref.#method_ident(#(#arg1,)* (#(#input),*)) #maybe_await
            };

            let unsafety = method.unsafety;
            if let Some(unsafe_kw) = unsafety {
                body = quote! { #unsafe_kw { #body } };
            }

            methods_out.extend(quote! {
                #(#docs)*
                #vis #asyncness #unsafety fn #method_ident(& #mutability self, #(#arg0: #ty),*) -> #return_type {
                    #body
                }
            });
        }

        for method in raw_fns.ref_fns() {
            method.to_tokens(&mut methods_out);
        }

        let input_list = input_list.clone().into_reference_list().with_lifetime("'a");

        quote! {
            #hidden
            pub struct #ref_ident<'a> {
                deref: &'a mut #item_ident,
                #[allow(dead_code)]
                input: #input_list,
            }

            impl std::ops::Deref for #ref_ident<'_> {
                type Target = #item_ident;
                fn deref(&self) -> &Self::Target {
                    &self.deref
                }
            }

            impl std::ops::DerefMut for #ref_ident<'_> {
                fn deref_mut(&mut self) -> &mut Self::Target {
                    &mut self.deref
                }
            }

            impl #ref_ident<'_> { #methods_out }
        }
    }

    fn define_macro(
        item_ident: &Ident,
        config: &Config,
        input_list: &InputList,
    ) -> proc_macro2::TokenStream {
        let macro_ident = macro_ident(item_ident);

        let query_message = &config.query.message_ident;
        let mut take = quote! {
            #query_message(input = #input_list),
        };

        if let Some(transaction_config) = &config.transaction {
            let transaction_message = &transaction_config.message_ident;
            take.extend(quote! {
                #transaction_message(input = #input_list),
            });
        }

        quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, clippy::redundant_pub_crate)]
            #[allow(unused_imports)]
            pub(crate) mod #macro_ident {
                #[macro_export]
                #[doc(hidden)]
                macro_rules! #macro_ident {
                    (($callback:path $(, $($callbacks:tt)*)?) ($($_params:tt)*) $_ident:ident ($($meta:tt)*) $($rest:tt)*) => {
                        $callback!(($($($callbacks)*)?) ($($_params)*) $_ident ($($meta)* take(#take)) $($rest)*);
                    };
                }
                pub use #macro_ident;
            }
        }
    }

    fn impl_get_ref(
        item_ident: &Ident,
        config: &Config,
        generics: &Generics,
    ) -> proc_macro2::TokenStream {
        let auditeur = &*crate::AUDITEUR;
        let connection = &config.query.ref_ident;

        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        quote! {
            impl #impl_generics #auditeur::input::get::GetRef for #item_ident #ty_generics #where_clause {
                type Ref = #connection;

                async fn get_ref(
                    &self,
                    registry: &#auditeur::app::Registry,
                    dependent: #auditeur::app::ComponentIdRef<'_>,
                ) -> Result<Self::Ref, #auditeur::Error> {
                    #connection::new(registry, dependent).await
                }
            }
        }
    }

    fn impl_respond(
        item_ident: &Ident,
        config: &Config,
        generics: &Generics,
    ) -> proc_macro2::TokenStream {
        let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

        let query_ident = &config.query.message_ident;
        let connection_ident = &config.query.ref_ident;

        let auditeur = &*crate::AUDITEUR;
        quote! {
            impl #impl_generics #auditeur::resource::listener::Respond for #item_ident #ty_generics
            #where_clause
            {
                type Query = #query_ident;
                type Connection = #connection_ident;
            }
        }
    }
}

struct RawFns {
    raw_fns: Vec<(ImplItemFn, RawFnType)>,
}

impl RawFns {
    fn connection_fns(&self) -> impl Iterator<Item = &ImplItemFn> {
        self.raw_fns
            .iter()
            .filter(|(_, typ)| typ.connection_fn())
            .map(|(func, _)| func)
    }

    fn ref_fns(&self) -> impl Iterator<Item = &ImplItemFn> {
        self.raw_fns
            .iter()
            .filter(|(_, typ)| typ.ref_fn())
            .map(|(func, _)| func)
    }

    fn iter(&self) -> impl Iterator<Item = &ImplItemFn> {
        self.raw_fns.iter().map(|(func, _)| func)
    }
}

#[derive(Default)]
struct MethodAttributes {
    attributes: Punctuated<MethodAttribute, Token![,]>,
}

impl Parse for MethodAttributes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            attributes: input.parse_terminated(MethodAttribute::parse, Token![,])?,
        })
    }
}

impl MethodAttributes {
    fn into_config(self) -> syn::Result<MethodAttributeConfig> {
        let span = self.attributes.span();

        let (is_connection_fn, is_ref_fn, timeout, on_error) = self.attributes.into_iter().fold(
            Default::default(),
            |(mut is_connection_fn, mut is_ref_fn, mut timeout, mut on_error), attr| {
                match attr {
                    MethodAttribute::ConnectionFn(_) => is_connection_fn = true,
                    MethodAttribute::RefFn(_) => is_ref_fn = true,
                    MethodAttribute::Timeout(nv) => timeout = Some(Box::new(nv.value)),
                    MethodAttribute::OnError(nv) => on_error = Some(nv.value.into_type()),
                }
                (is_connection_fn, is_ref_fn, timeout, on_error)
            },
        );

        let raw_fn_attr = if is_connection_fn {
            Some("connection_fn")
        } else if is_ref_fn {
            Some("ref_fn")
        } else {
            None
        };

        let method_attr = if timeout.is_some() {
            Some("timeout")
        } else if on_error.is_some() {
            Some("on_error")
        } else {
            None
        };

        if let (Some(raw_fn_attr), Some(method_attr)) = (raw_fn_attr, method_attr) {
            return Err(syn::Error::new(
                span,
                format!("{raw_fn_attr} and {method_attr} are mutually exclusive"),
            ));
        }

        match (is_connection_fn, is_ref_fn) {
            (false, false) => Ok(MethodAttributeConfig::Method {
                timeout,
                on_error: on_error.unwrap_or_default(),
            }),
            (false, true) => Ok(MethodAttributeConfig::RawFn(RawFnType::Ref)),
            (true, false) => Ok(MethodAttributeConfig::RawFn(RawFnType::Connection)),
            (true, true) => Ok(MethodAttributeConfig::RawFn(RawFnType::Both)),
        }
    }
}

enum MethodAttributeConfig {
    RawFn(RawFnType),
    Method {
        timeout: Option<Box<Expr>>,
        on_error: OnErrorType,
    },
}

enum MethodAttribute {
    ConnectionFn(Ident),
    RefFn(Ident),
    Timeout(NameValue<Expr>),
    OnError(NameValue<OnError>),
}

impl Parse for MethodAttribute {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "connection_fn" => Ok(Self::ConnectionFn(ident)),
            "ref_fn" => Ok(Self::RefFn(ident)),
            "timeout" => NameValue::parse_after(ident, input).map(Self::Timeout),
            "on_error" => NameValue::parse_after(ident, input).map(Self::OnError),
            other => Err(input.error(format!(
                "expected 'connection_fn', 'ref_fn', 'timeout', or 'on_error', found {other}"
            ))),
        }
    }
}

impl ToTokens for MethodAttribute {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::ConnectionFn(ident) | Self::RefFn(ident) => ident.to_tokens(tokens),
            Self::Timeout(name_value) => name_value.to_tokens(tokens),
            Self::OnError(name_value) => name_value.to_tokens(tokens),
        }
    }
}

enum OnError {
    Panic(Ident),
    WrapInResult(Ident),
    ReturnErr(Ident, Option<Parenthesized<NameValue<Box<Type>>>>),
    WrapInOption(Ident),
    ReturnNone(Ident),
    Return(NameValue<Box<Expr>>),
}

impl OnError {
    fn into_type(self) -> OnErrorType {
        match self {
            Self::Panic(_) => OnErrorType::Panic,
            Self::WrapInResult(_) => OnErrorType::WrapInResult,
            Self::ReturnErr(_, option) => {
                OnErrorType::ReturnErr(option.map(|parenthesized| parenthesized.value.value))
            }
            Self::WrapInOption(_) => OnErrorType::WrapInOption,
            Self::ReturnNone(_) => OnErrorType::ReturnNone,
            Self::Return(name_value) => OnErrorType::Return(name_value.value),
        }
    }
}

impl Parse for OnError {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "panic" => Ok(Self::Panic(ident)),
            "wrap_in_result" => Ok(Self::WrapInResult(ident)),
            "return_err" => Ok(Self::ReturnErr(ident, {
                if input.peek(token::Paren) {
                    Some(Parenthesized::parse_with(input, |input| {
                        NameValue::parse(input, "type")
                    })?)
                } else {
                    None
                }
            })),
            "wrap_in_option" => Ok(Self::WrapInOption(ident)),
            "return_none" => Ok(Self::ReturnNone(ident)),
            "return" => NameValue::parse_after(ident, input).map(Self::Return),
            other => Err(input.error(format!(
                "expected 'panic', 'wrap_in_result', 'return_err', 'wrap_in_option', 'return_none', or 'return', found {other}"
            ))),
        }
    }
}

impl ToTokens for OnError {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Panic(ident)
            | Self::WrapInResult(ident)
            | Self::WrapInOption(ident)
            | Self::ReturnNone(ident) => ident.to_tokens(tokens),
            Self::ReturnErr(ident, parenthesized) => {
                ident.to_tokens(tokens);
                parenthesized.to_tokens(tokens);
            }
            Self::Return(name_value) => name_value.to_tokens(tokens),
        }
    }
}

#[derive(Default)]
#[expect(dead_code)] // currently unimplemented
enum OnErrorType {
    #[default]
    Default,
    Panic,
    WrapInResult,
    ReturnErr(Option<Box<Type>>),
    WrapInOption,
    ReturnNone,
    Return(Box<Expr>),
}

impl OnErrorType {
    /// Returns `true` if the on error type is [`Default`].
    ///
    /// [`Default`]: OnErrorType::Default
    #[must_use]
    const fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

#[derive(Clone, Copy)]
enum RawFnType {
    Connection,
    Ref,
    Both,
}

impl RawFnType {
    #[must_use]
    const fn connection_fn(self) -> bool {
        matches!(self, Self::Connection | Self::Both)
    }

    #[must_use]
    const fn ref_fn(self) -> bool {
        matches!(self, Self::Ref | Self::Both)
    }
}

struct Method<'a> {
    snake_case_ident: &'a Ident,
    camel_case_ident: Ident,
    args: Vec<Arg<'a>>,
    combination: Combination,
    return_type: Option<&'a Type>,
    unsafety: Option<Token![unsafe]>,
    asyncness: Option<Token![async]>,
    mutability: Option<Token![mut]>,
    vis: Visibility,
    docs: Vec<&'a Attribute>,
    // timeout: Option<Expr>,
    // on_error: MethodOnError,
}

struct Arg<'a> {
    ident: &'a Ident,
    ty: &'a Type,
}

impl<'a> Method<'a> {
    pub fn new(
        item: &'a ImplItemFn,
        vis: Visibility,
        all_inputs: &mut InputList,
    ) -> syn::Result<Self> {
        if !item.sig.generics.params.is_empty() || item.sig.generics.where_clause.is_some() {
            return Err(syn::Error::new_spanned(
                &item.sig.generics,
                "method cannot have generics",
            ));
        }

        let snake_case_ident = &item.sig.ident;
        let camel_case_ident = Ident::new(
            &crate::snake_to_upper_camel_case(&snake_case_ident.to_string()),
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

        let unsafety = item.sig.unsafety;
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
            unsafety,
            asyncness,
            mutability,
            vis,
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

    fn maybe_await(&self) -> Option<proc_macro2::TokenStream> {
        if self.asyncness.is_some() {
            Some(quote! { .await })
        } else {
            None
        }
    }
}

pub struct Config<'a> {
    pub query: TakeConfig<'a>,
    pub transaction: Option<TakeConfig<'a>>,
    pub get_ref: bool,
    pub fields: Option<&'a Fields>,
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

#[derive(Debug)]
pub struct Meta {
    pub id: Option<Comma<NamespacedId>>,
    params: Punctuated<MetaParam, Token![,]>,
}

impl Meta {
    pub fn to_config(&self, item_ident: &Ident) -> syn::Result<Config<'_>> {
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

        let mut fields_iter = self.params.iter().filter_map(MetaParam::as_fields);
        let fields = fields_iter.next().map(|nv| &nv.value);
        if let Some(fields) = fields_iter.next() {
            return Err(syn::Error::new_spanned(
                fields,
                "expected a maximum of one fields argument",
            ));
        }

        Ok(Config {
            query,
            transaction,
            get_ref,
            fields,
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
        let (message_ident, message_id, ref_ident) = params.fold(
            (None, None, None),
            |(message_ident, message_id, ref_ident), param| {
                (
                    param.message_ident().or(message_ident),
                    param.message_id().or(message_id),
                    param.ref_ident().or(ref_ident),
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

#[derive(Debug)]
enum MetaParam {
    Query(List<TakeTypesParam>),
    Transaction(List<TakeTypesParam>),
    TransactionAutoFlag(Ident),
    GetRefFlag(Ident),
    Fields(NameValue<Fields>),
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

    const fn as_fields(&self) -> Option<&NameValue<Fields>> {
        if let Self::Fields(v) = self {
            Some(v)
        } else {
            None
        }
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
            "fields" => NameValue::parse_after(ident, input).map(Self::Fields),
            other => Err(input.error(format!(
                "expected 'query', 'transaction', 'get_ref', or 'fields', found {other}"
            ))),
        }
    }
}

impl ToTokens for MetaParam {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Query(list) | Self::Transaction(list) => list.to_tokens(tokens),
            Self::TransactionAutoFlag(ident) | Self::GetRefFlag(ident) => ident.to_tokens(tokens),
            Self::Fields(fields) => fields.to_tokens(tokens),
        }
    }
}

#[derive(Debug)]
enum TakeTypesParam {
    MessageId(NameValue<NamespacedId>),
    MessageIdent(NameValue<Ident>),
    RefIdent(NameValue<Ident>),
}

impl TakeTypesParam {
    fn message_ident(&self) -> Option<&Ident> {
        match self {
            Self::MessageIdent(name_value) => Some(name_value),
            _ => None,
        }
    }

    fn message_id(&self) -> Option<&NamespacedId> {
        match self {
            Self::MessageId(name_value) => Some(name_value),
            _ => None,
        }
    }

    fn ref_ident(&self) -> Option<&Ident> {
        match self {
            Self::RefIdent(name_value) => Some(name_value),
            _ => None,
        }
    }
}

impl Parse for TakeTypesParam {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "message_ident" => NameValue::parse_after(ident, input).map(Self::MessageIdent),
            "message_id" => NameValue::parse_after(ident, input).map(Self::MessageId),
            "ref_ident" => NameValue::parse_after(ident, input).map(Self::RefIdent),
            other => Err(input.error(format!(
                "expected 'message_ident', 'message_id, or 'ref_ident', found {other}"
            ))),
        }
    }
}

impl ToTokens for TakeTypesParam {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::MessageId(name_value) => name_value.to_tokens(tokens),
            Self::MessageIdent(name_value) | Self::RefIdent(name_value) => {
                name_value.to_tokens(tokens);
            }
        }
    }
}

#[derive(Debug)]
pub struct Fields {
    brace: Brace,
    list: Punctuated<Field, Token![,]>,
}

impl Parse for Fields {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        Ok(Self {
            brace: braced!(content in input),
            list: content.parse_terminated(Field::parse, Token![,])?,
        })
    }
}

impl ToTokens for Fields {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.brace.surround(tokens, |tokens| {
            self.list.to_tokens(tokens);
        });
    }
}

#[derive(Debug)]
struct Field {
    ident: Ident,
    colon: Token![:],
    ty: Box<Type>,
}

impl Parse for Field {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            ident: input.parse()?,
            colon: input.parse()?,
            ty: input.parse()?,
        })
    }
}

impl ToTokens for Field {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.ident.to_tokens(tokens);
        self.colon.to_tokens(tokens);
        self.ty.to_tokens(tokens);
    }
}

#[derive(Default)]
struct SearchTypes {
    ty: HashMap<Ident, bool>,
    cnst: HashMap<Ident, bool>,
    lifetime: HashMap<Lifetime, bool>,
}

impl SearchTypes {
    fn from_generics(generics: &Generics) -> Self {
        let mut this = Self::default();
        for param in &generics.params {
            match param {
                GenericParam::Lifetime(lt) => this.lifetime.insert(lt.lifetime.clone(), false),
                GenericParam::Type(ty) => this.ty.insert(ty.ident.clone(), false),
                GenericParam::Const(cnst) => this.cnst.insert(cnst.ident.clone(), false),
            };
        }
        this
    }

    fn any(&self) -> bool {
        (self.ty.values())
            .chain(self.cnst.values())
            .chain(self.lifetime.values())
            .any(|val| *val)
    }
}

impl<'ast> Visit<'ast> for SearchTypes {
    fn visit_type_path(&mut self, i: &'ast syn::TypePath) {
        if i.path.segments.len() == 1
            && i.qself.is_none()
            && i.path.leading_colon.is_none()
            && let Some(segment) = i.path.segments.first()
            && let Some(is_present) = self.ty.get_mut(&segment.ident)
        {
            *is_present = true;
        }

        syn::visit::visit_type_path(self, i);
    }

    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if i.path.segments.len() == 1
            && i.qself.is_none()
            && i.path.leading_colon.is_none()
            && let Some(segment) = i.path.segments.first()
            && let Some(is_present) = self.cnst.get_mut(&segment.ident)
        {
            *is_present = true;
        }

        syn::visit::visit_expr_path(self, i);
    }

    fn visit_lifetime(&mut self, i: &'ast syn::Lifetime) {
        if let Some(is_present) = self.lifetime.get_mut(i) {
            *is_present = true;
        }

        syn::visit::visit_lifetime(self, i);
    }
}
