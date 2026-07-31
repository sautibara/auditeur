use std::borrow::Cow;

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Ident, Token, braced, fold::Fold, parse::Parse, punctuated::Punctuated, spanned::Spanned,
};

use crate::{
    NamespacedId,
    attr::{self, FoldAttr},
    field,
    maybe_error::{
        Continue, ContinueExt, MaybeError, MaybeErrorExt, drain_continues_into, drain_results_into,
    },
    overlay,
};

// #[respond(
//     "core:library",
//     connection(ident = LibraryConnection),
//     query(ident = LibraryQuery, id = "core:library_query")
//     fields = {
//         field_name: field_type,
//     }
// )]

// #[respond(
//     connection_fn,
//     resource_fn,
//     timeout = Duration::from_secs(2),
//     on_error = panic,
// )]

// TODO: `timeout` and `on_error`

// TODO: private `dependency_handle` and `dependent` functions,
// meant for automatically adding dependencies when ingesting foreign closures

pub fn respond_attr(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let mut error = None;

    // Parsing
    let item: syn::ItemImpl = syn::parse2(item)?;
    let attr: Attrs = syn::parse2(attr)?;

    // Parsing config
    let config = Config::new(&attr, &item).drain_into(&mut error);
    let methods_to_remove = config.methods_to_remove();

    // Expansion
    let respond_impls = config.expand();

    // Mutation
    let item = FoldAttr::remove("respond").fold_item_impl(item);
    let item = item_impl_remove_methods(item, &methods_to_remove);

    // Combination
    let error = error.map(syn::Error::into_compile_error);
    Ok(quote! {
        #[allow(clippy::needless_pass_by_value)]
        #item

        #respond_impls
        #error
    })
}

fn item_impl_remove_methods(mut item: syn::ItemImpl, methods: &[usize]) -> syn::ItemImpl {
    item.items = (item.items.into_iter())
        .enumerate()
        .filter(|(i, _)| !methods.contains(i))
        .map(|(_, item)| item)
        .collect();
    item
}

struct Config<'a> {
    item: &'a syn::ItemImpl,

    query_ident: Cow<'a, Ident>,
    query_id: Cow<'a, NamespacedId>,
    connection_ident: Cow<'a, Ident>,

    transaction: bool,
    get_ref: bool,

    doc_vis: TokenStream,
    id_expr: TokenStream,

    methods: Vec<MethodConfig<'a>>,
    fields: Vec<&'a Field>,
}

/// Functions related to construction.
impl<'a> Config<'a> {
    fn new(attrs: &'a Attrs, item: &'a syn::ItemImpl) -> Continue<Self> {
        let mut error = None;

        let (query_ident, query_id, connection_ident) =
            Self::get_component_data(attrs, item, &mut error);

        let methods: Vec<_> = (item.items.iter())
            .enumerate()
            .filter_map(|(i, item)| impl_item_as_fn(item).map(|item| (i, item)))
            .map(|(i, item)| MethodConfig::new(item, i))
            .map(drain_continues_into(&mut error))
            .collect();
        let fields = (attrs.attrs.iter())
            .filter_map(Attr::as_fields)
            .flat_map(|fields| fields.fields.iter())
            .collect();

        let vis = if (methods.iter())
            .any(|method| matches!(method.method.vis, syn::Visibility::Public(_)))
        {
            syn::Visibility::Public(Token![pub](Span::call_site()))
        } else {
            syn::Visibility::Inherited
        };
        let doc_vis = doc_private(&vis);

        let transaction = attrs.attrs.iter().any(Attr::is_transaction_flag);
        let get_ref = attrs.attrs.iter().any(Attr::is_get_ref_flag);

        let id_expr = Self::id_expr(attrs, item).drain_into(&mut error);

        error.wrap(Self {
            item,
            query_ident,
            query_id,
            connection_ident,
            transaction,
            get_ref,
            doc_vis,
            id_expr,
            methods,
            fields,
        })
    }

    fn get_component_data(
        attrs: &'a Attrs,
        item: &'a syn::ItemImpl,
        error: &mut MaybeError,
    ) -> (Cow<'a, Ident>, Cow<'a, NamespacedId>, Cow<'a, Ident>) {
        let mut id = None;
        let mut ident = None;

        let query_ident = attrs
            .get_component_data(Attr::as_query, ComponentInfo::as_ident)
            .map_or_else(
                || Cow::Owned(Self::default_query_ident(item, &mut ident, error)),
                Cow::Borrowed,
            );

        let query_id = attrs
            .get_component_data(Attr::as_query, ComponentInfo::as_id)
            .map_or_else(
                || Cow::Owned(Self::default_query_id(attrs, &mut id, error)),
                Cow::Borrowed,
            );

        let connection_ident = attrs
            .get_component_data(Attr::as_connection, ComponentInfo::as_ident)
            .map_or_else(
                || Cow::Owned(Self::default_connection_ident(item, &mut ident, error)),
                Cow::Borrowed,
            );

        if let Some(id) = attrs.get_component_data(Attr::as_connection, ComponentInfo::as_id) {
            error.combine(syn::Error::new(
                id.span(),
                "unexpected id argument - connection does not use an id",
            ));
        }

        (query_ident, query_id, connection_ident)
    }

    fn default_connection_ident(
        item: &'a syn::ItemImpl,
        ident: &mut Option<Cow<'a, Ident>>,
        error: &mut MaybeError,
    ) -> Ident {
        format_ident!("{}Connection", Self::item_ident(item, ident, error))
    }

    fn default_query_ident(
        item: &'a syn::ItemImpl,
        ident: &mut Option<Cow<'a, Ident>>,
        error: &mut MaybeError,
    ) -> Ident {
        format_ident!("{}Query", Self::item_ident(item, ident, error))
    }

    fn item_ident(
        item: &'a syn::ItemImpl,
        ident: &mut Option<Cow<'a, Ident>>,
        error: &mut MaybeError,
    ) -> Cow<'a, Ident> {
        let cow = ident.get_or_insert_with(|| {
            if let syn::Type::Path(path) = &*item.self_ty
                && let Some(segment) = path.path.segments.last()
            {
                Cow::Borrowed(&segment.ident)
            } else {
                error.combine(syn::Error::new(
                    item.self_ty.span(),
                    "expected a single identifier here, found a complex type",
                ));
                Cow::Owned(Ident::new("RespondRecover", item.self_ty.span()))
            }
        });
        cow.clone()
    }

    fn default_query_id(
        attrs: &'a Attrs,
        id: &mut Option<Cow<'a, NamespacedId>>,
        error: &mut MaybeError,
    ) -> NamespacedId {
        let id = Self::item_id(attrs, id, error);
        let id = id.into_owned();
        id.concat("_query")
    }

    fn item_id(
        attrs: &'a Attrs,
        id: &mut Option<Cow<'a, NamespacedId>>,
        error: &mut MaybeError,
    ) -> Cow<'a, NamespacedId> {
        let cow = id.get_or_insert_with(|| {
            attrs.id.as_ref().map_or_else(
                || {
                    error.combine(syn::Error::new(
                        Span::call_site(),
                        "cannot get the item's `NamespacedId`, expected one as the first argument",
                    ));
                    Cow::Owned(NamespacedId::new("error:recovery", Span::call_site()))
                },
                |id| Cow::Borrowed(&id.value),
            )
        });
        cow.clone()
    }

    fn id_expr(attrs: &'a Attrs, item: &'a syn::ItemImpl) -> Continue<TokenStream> {
        let mut error = None;

        let auditeur = &*crate::AUDITEUR;
        let expr = if item.generics.params.is_empty() && item.generics.where_clause.is_none() {
            let item_ident = &item.self_ty;
            quote! { <#item_ident as #auditeur::component::IdentifiedType>::id() }
        } else if let Some(id) = &attrs.id {
            let id = &id.value;
            quote! { #id }
        } else {
            error.combine(syn::Error::new(
                Span::call_site(),
                "id must be specified for a resource with generics",
            ));
            quote! { #auditeur::namespaced_id::ident!("error:recovery") }
        };

        let expr = quote! {
            #auditeur::component::ComponentId {
                registry: <#auditeur::component::resource::Resources
                    as #auditeur::component::IdentifiedType>::id(),
                component: #auditeur::component::ComponentIdPath::Static(#expr),
            }
        };

        error.wrap(expr)
    }
}

/// Methods related to expansion.
impl Config<'_> {
    fn expand(&self) -> TokenStream {
        let mut tokens = TokenStream::new();
        self.expand_query_enum(&mut tokens);
        self.expand_take_impl(&mut tokens);
        self.expand_connection_struct(&mut tokens);
        self.expand_respond_impl(&mut tokens);
        if self.get_ref {
            self.expand_get_ref_impl(&mut tokens);
        }
        tokens
    }

    fn expand_query_enum(&self, tokens: &mut TokenStream) {
        let mut variants = TokenStream::new();
        (self.methods.iter()).for_each(|method| method.expand_query_variant(&mut variants));
        if self.transaction {
            self.expand_query_transaction_variant(&mut variants);
        }

        let ident = &self.query_ident;
        let doc_vis = &self.doc_vis;

        let message_impl =
            super::derive_take_message(ident, &self.query_id, &syn::Generics::default());

        tokens.extend(quote! {
            #[non_exhaustive]
            #doc_vis
            pub enum #ident {
                #variants
            }

            #message_impl
        });
    }

    fn expand_query_transaction_variant(&self, tokens: &mut TokenStream) {
        // Box<FnOnce(&mut self) -> Pin<Box<Future -> ()>>>
        let self_ty = &self.item.self_ty;
        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            Transaction(#auditeur::component::resource::listener::BoxedTransactionFnOnce<#self_ty>),
        });
    }

    fn expand_take_impl(&self, tokens: &mut TokenStream) {
        let (impl_generics, _, where_clause) = self.item.generics.split_for_impl();

        let self_ty = &self.item.self_ty;
        let query_ident = &self.query_ident;

        let mut variants = TokenStream::new();
        (self.methods.iter())
            .for_each(|method| method.expand_take_variant(query_ident, &mut variants));
        if self.transaction {
            self.expand_take_transaction_variant(&mut variants);
        }

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            impl #impl_generics #auditeur::component::resource::listener::Take<#query_ident> for #self_ty #where_clause {
                async fn on(&mut self, query: #query_ident) -> #auditeur::anyhow::Result<()> {
                    match query {
                        #variants
                    }
                    Ok(())
                }
            }
        });
    }

    fn expand_take_transaction_variant(&self, tokens: &mut TokenStream) {
        let query_ident = &self.query_ident;
        tokens.extend(quote! {
            #query_ident::Transaction(func) => func.run(self).await,
        });
    }

    fn field_idents(&self) -> impl Iterator<Item = &Ident> {
        self.fields.iter().map(|field| &field.ident)
    }

    fn field_types(&self) -> impl Iterator<Item = &syn::Type> {
        self.fields.iter().map(|field| &field.ty)
    }

    fn expand_connection_struct(&self, tokens: &mut TokenStream) {
        let query_ident = &self.query_ident;
        let connection_ident = &self.connection_ident;
        let doc_vis = &self.doc_vis;

        let field_idents = self.field_idents();
        let field_types = self.field_types();

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            #doc_vis
            pub struct #connection_ident {
                dependency: #auditeur::app::map::Dependency<()>,
                giver: #auditeur::component::take::Giver<#query_ident>,
                #(#field_idents: #field_types,)*
            }
        });

        self.expand_connection_impl_new(tokens);
        self.expand_connection_methods(tokens);
        self.expand_connection_impl_input(tokens);
        self.expand_connection_impl_any(tokens);
        self.expand_connection_impl_clone_as(tokens);
        self.expand_connection_impl_debug(tokens);
    }

    fn expand_connection_methods(&self, tokens: &mut TokenStream) {
        let mut methods = TokenStream::new();
        for method in &self.methods {
            method.expand_connection_method(&self.item.self_ty, &self.query_ident, &mut methods);
        }

        if self.transaction {
            self.expand_connection_transaction_method(&mut methods);
        }

        let connection_ident = &self.connection_ident;
        tokens.extend(quote! {
            impl #connection_ident {
                #methods
            }
        });
    }

    fn expand_connection_transaction_method(&self, tokens: &mut TokenStream) {
        let self_ty = &self.item.self_ty;
        let query_ident = &self.query_ident;

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            pub async fn run_transaction<T: Send + Sync + 'static>(
                &self,
                transaction: impl for<'a>
                    #auditeur::component::resource::listener::SendAsyncFnOnce<&'a mut #self_ty, T>,
            ) -> T {
                let (response, receiver) = #auditeur::tokio::sync::oneshot::channel();
                self.giver.give(#query_ident::Transaction(
                    #auditeur::component::resource::listener::BoxedTransactionFnOnce::new_with_sender(
                        transaction,
                        response,
                    ),
                ));

                receiver.await.expect(concat!(
                    stringify!(#self_ty),
                    " transaction panicked"
                ))
            }

            pub fn spawn_transaction(
                &self,
                transaction: impl for<'a>
                    #auditeur::component::resource::listener::SendAsyncFnOnce<&'a mut #self_ty, ()>,
            ) {
                self.giver.give(#query_ident::Transaction(
                    #auditeur::component::resource::listener::BoxedTransactionFnOnce::new(transaction)
                ));
            }
        });
    }

    fn expand_connection_impl_new(&self, tokens: &mut TokenStream) {
        let connection_ident = &self.connection_ident;

        let auditeur = &*crate::AUDITEUR;
        let new_docs = format!(
            "Create a new [`{connection_ident}`] using an [`App`]({}::app::App).",
            auditeur.to_token_stream().to_string().replace(' ', "")
        );

        let id_expr = &self.id_expr;
        let field_idents = self.field_idents();
        let field_types = self.field_types();

        tokens.extend(quote! {
            impl #connection_ident {
                #[doc = #new_docs]
                pub async fn new(
                    app: &mut #auditeur::app::App,
                ) -> ::std::result::Result<Self, #auditeur::Error> {
                    #[allow(unused_variables)]
                    Ok(Self {
                        dependency: app.components_mut().load_lazy(#id_expr).await?,
                        giver: app.take_mut().giver().await?,
                        #(#field_idents:
                            <#field_types as #auditeur::input::Input>::create(app).await?,)*
                    })
                }
            }
        });
    }

    fn expand_connection_impl_input(&self, tokens: &mut TokenStream) {
        let connection_ident = &self.connection_ident;

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            impl #auditeur::input::Input for #connection_ident {
                async fn create(
                    app: &mut #auditeur::app::App,
                ) -> Result<Self, #auditeur::Error> {
                    Self::new(app).await
                }
            }
        });
    }

    fn expand_connection_impl_any(&self, tokens: &mut TokenStream) {
        tokens.extend(super::any(
            &self.connection_ident,
            &syn::Generics::default(),
        ));
    }

    fn expand_connection_impl_clone_as(&self, tokens: &mut TokenStream) {
        tokens.extend(super::clone_as_struct(
            &self.connection_ident,
            [format_ident!("dependency"), format_ident!("giver")]
                .iter()
                .chain(self.field_idents()),
            &syn::Generics::default(),
        ));
    }

    fn expand_connection_impl_debug(&self, tokens: &mut TokenStream) {
        let connection_ident = &self.connection_ident;
        tokens.extend(quote! {
            impl ::std::fmt::Debug for #connection_ident {
                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                    write!(f, "{} {{ .. }}", stringify!(#connection_ident))
                }
            }
        });
    }

    fn expand_respond_impl(&self, tokens: &mut TokenStream) {
        let self_ty = &self.item.self_ty;
        let query_ident = &self.query_ident;
        let (impl_generics, _, where_clause) = self.item.generics.split_for_impl();

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            impl #impl_generics #auditeur::component::resource::listener::Respond for #self_ty
            #where_clause
            {
                type Query = #query_ident;
            }
        });
    }

    fn expand_get_ref_impl(&self, tokens: &mut TokenStream) {
        let self_ty = &self.item.self_ty;
        let connection_ident = &self.connection_ident;
        let (impl_generics, _, where_clause) = self.item.generics.split_for_impl();

        let auditeur = &*crate::AUDITEUR;
        tokens.extend(quote! {
            impl #impl_generics #auditeur::input::get::GetRefWith for #self_ty
            #where_clause
            {
                type Method = #auditeur::input::get::GetInput<#connection_ident>;
            }
        });
    }
}

/// Methods related to mutation.
impl Config<'_> {
    fn methods_to_remove(&self) -> Vec<usize> {
        self.methods
            .iter()
            .filter(|method| method.info.is_connection_fn())
            .map(|method| method.index)
            .collect()
    }
}

struct MethodConfig<'a> {
    method: &'a syn::ImplItemFn,
    docs: Vec<&'a syn::Attribute>,
    info: MethodInfo,
    index: usize,
}

enum MethodInfo {
    ConnectionFn,
    ResourceFn,
    Default { camel_case_ident: Ident },
}

impl<'a> MethodConfig<'a> {
    fn new(method: &'a syn::ImplItemFn, index: usize) -> Continue<Self> {
        let mut error = None;

        let attrs = attr::collect_attrs::<MethodAttr, MethodAttrs>("respond", &method.attrs)
            .filter_map(drain_results_into(&mut error));
        // We need to go through *all* attrs *once* to get as many potential errors as possible.
        let (connection_fn, resource_fn) =
            attrs.fold((None, None), |(connection_fn, resource_fn), attr| {
                (
                    connection_fn.or_else(|| attr.as_connection_fn_flag()),
                    resource_fn.or_else(|| attr.as_resource_fn_flag()),
                )
            });

        let info = match (connection_fn, resource_fn) {
            (Some(connection_fn_span), Some(resource_fn_span)) => {
                error.combine(syn::Error::new(
                    method.span(),
                    "cannot combine 'connection_fn' and 'resource_fn'",
                ));
                error.combine(syn::Error::new(
                    connection_fn_span,
                    "'connection_fn' declared here",
                ));
                error.combine(syn::Error::new(
                    resource_fn_span,
                    "'resource_fn' declared here",
                ));
                MethodInfo::ResourceFn
            }
            (Some(_), _) => MethodInfo::ConnectionFn,
            (_, Some(_)) => MethodInfo::ResourceFn,
            _ => {
                let camel_case_ident = Ident::new(
                    &crate::snake_to_upper_camel_case(&method.sig.ident.to_string()),
                    method.sig.ident.span(),
                );

                MethodInfo::Default { camel_case_ident }
            }
        };

        let docs = (method.attrs.iter())
            .filter(|item| item.path().is_ident("doc"))
            .collect();

        error.wrap(Self {
            method,
            docs,
            info,
            index,
        })
    }

    fn expand_query_variant(&self, ts: &mut TokenStream) {
        let MethodInfo::Default { camel_case_ident } = &self.info else {
            return;
        };

        let vis = doc_private(&self.method.vis);

        let return_type = if let syn::ReturnType::Type(_, ty) = &self.method.sig.output {
            ty.to_token_stream()
        } else {
            quote! { () }
        };

        let docs = &self.docs;

        let fields = (self.method.sig.inputs)
            .iter()
            .filter(|fn_arg| matches!(fn_arg, syn::FnArg::Typed(_)));

        let auditeur = &*crate::AUDITEUR;
        ts.extend(quote! {
            #(#docs)*
            #[allow(rustdoc::broken_intra_doc_links)]
            #vis #camel_case_ident {
                response: #auditeur::tokio::sync::oneshot::Sender<#return_type>,
                #(#fields),*
            },
        });
    }

    fn expand_take_variant(&self, query_ident: &Ident, ts: &mut TokenStream) {
        let MethodInfo::Default { camel_case_ident } = &self.info else {
            return;
        };

        let ident = &self.method.sig.ident;

        let args = (self.method.sig.inputs.iter())
            .filter_map(fn_arg_as_typed)
            .map(|fn_arg| &fn_arg.pat);

        let call_args = args.clone();
        let await_kw = (self.method.sig.asyncness)
            .map(|kw| Token![await](kw.span))
            .into_iter();
        let unsafe_kw = self.method.sig.unsafety;

        ts.extend(quote! {
            #query_ident::#camel_case_ident { response, #(#args),* } => {
                let result = #unsafe_kw { self.#ident(#(#call_args),*) #(.#await_kw)* } ;
                let _ = response.send(result);
            }
        });
    }

    fn expand_connection_method(
        &self,
        self_ty: &syn::Type,
        query_ident: &Ident,
        tokens: &mut TokenStream,
    ) {
        let MethodInfo::Default { camel_case_ident } = &self.info else {
            if matches!(&self.info, MethodInfo::ConnectionFn) {
                overlay::ImplItemFn::new(self.method)
                    .remove_attr("respond")
                    .to_tokens(tokens);
            }
            return;
        };

        let auditeur = &*crate::AUDITEUR;
        let ident = &self.method.sig.ident;

        let sig = overlay::Signature::new(&self.method.vis, &self.method.sig)
            .asyncness(true)
            .receiver(quote! { &self });

        let args = (self.method.sig.inputs.iter())
            .filter_map(fn_arg_as_typed)
            .map(|fn_arg| &fn_arg.pat);

        let docs = (self.method.attrs.iter()).filter(|attr| attr.path().is_ident("doc"));

        tokens.extend(quote! {
            #[allow(clippy::needless_pass_by_value)]
            #(#docs)*
            #sig {
                let (response, auditeur_receiver) = #auditeur::tokio::sync::oneshot::channel();

                let query = #query_ident::#camel_case_ident { response, #(#args),* };
                self.giver.give(query);

                auditeur_receiver.await.expect(concat!(
                    "respond: ",
                    stringify!(#self_ty), "::", stringify!(#ident),
                    " panicked"
                ))
            }
        });
    }
}

impl MethodInfo {
    /// Returns `true` if the method info is [`ConnectionFn`].
    ///
    /// [`ConnectionFn`]: MethodInfo::ConnectionFn
    #[must_use]
    const fn is_connection_fn(&self) -> bool {
        matches!(self, Self::ConnectionFn)
    }
}

/// The parsable representation of the arguments to the respond macro.
struct Attrs {
    id: Option<field::Comma<NamespacedId>>,
    attrs: Punctuated<Attr, Token![,]>,
}

impl Attrs {
    fn get_component_data<T>(
        &self,
        attr_map: impl Fn(&Attr) -> Option<&field::List<ComponentInfo>>,
        component_map: impl Fn(&ComponentInfo) -> Option<&T>,
    ) -> Option<&T> {
        (self.attrs.iter())
            .filter_map(attr_map)
            .flatten()
            .filter_map(component_map)
            .next_back()
    }
}

impl Parse for Attrs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let id = input
            .peek(syn::LitStr)
            .then(|| field::Comma::parse(input))
            .transpose()?;
        let attrs = (id.as_ref().is_none_or(field::Comma::has_comma))
            .then(|| input.parse_terminated(Attr::parse, Token![,]))
            .transpose()?
            .unwrap_or_default();
        Ok(Self { id, attrs })
    }
}

impl IntoIterator for Attrs {
    type IntoIter = syn::punctuated::IntoIter<Attr>;
    type Item = Attr;

    fn into_iter(self) -> Self::IntoIter {
        self.attrs.into_iter()
    }
}

enum Attr {
    Connection(field::List<ComponentInfo>),
    Query(field::List<ComponentInfo>),
    Fields(field::NameValue<Fields>),
    TransactionFlag,
    GetRefFlag,
}

impl Attr {
    const fn as_connection(&self) -> Option<&field::List<ComponentInfo>> {
        if let Self::Connection(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_query(&self) -> Option<&field::List<ComponentInfo>> {
        if let Self::Query(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_fields(&self) -> Option<&field::NameValue<Fields>> {
        if let Self::Fields(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns `true` if the attr is [`TransactionFlag`].
    ///
    /// [`TransactionFlag`]: Attr::TransactionFlag
    #[must_use]
    const fn is_transaction_flag(&self) -> bool {
        matches!(self, Self::TransactionFlag)
    }

    /// Returns `true` if the attr is [`GetRefFlag`].
    ///
    /// [`GetRefFlag`]: Attr::GetRefFlag
    #[must_use]
    const fn is_get_ref_flag(&self) -> bool {
        matches!(self, Self::GetRefFlag)
    }
}

impl Parse for Attr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident_res = input.parse();
        let ident: Ident = match ident_res {
            Ok(ident) => ident,
            Err(err) => {
                return Err(if input.peek(syn::LitStr) {
                    input.error(format!(
                        "{err}: an id can only be specified as the first argument",
                    ))
                } else {
                    err
                });
            }
        };

        if ident == "connection" {
            field::List::parse_after(ident, input).map(Self::Connection)
        } else if ident == "query" {
            field::List::parse_after(ident, input).map(Self::Query)
        } else if ident == "fields" {
            field::NameValue::parse_after(ident, input).map(Self::Fields)
        } else if ident == "transaction" {
            Ok(Self::TransactionFlag)
        } else if ident == "get_ref" {
            Ok(Self::GetRefFlag)
        } else {
            Err(input.error(format!(
                "expected 'connection', 'query', 'fields', 'transaction', or 'get_ref', found {ident}"
            )))
        }
    }
}

enum ComponentInfo {
    Id(field::NameValue<NamespacedId>),
    Ident(field::NameValue<Ident>),
}

impl ComponentInfo {
    const fn as_id(&self) -> Option<&NamespacedId> {
        if let Self::Id(v) = self {
            Some(&v.value)
        } else {
            None
        }
    }

    const fn as_ident(&self) -> Option<&Ident> {
        if let Self::Ident(v) = self {
            Some(&v.value)
        } else {
            None
        }
    }
}

impl Parse for ComponentInfo {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "ident" {
            field::NameValue::parse_after(ident, input).map(Self::Ident)
        } else if ident == "id" {
            field::NameValue::parse_after(ident, input).map(Self::Id)
        } else {
            Err(input.error(format!("expected 'ident' or 'id', found {ident}")))
        }
    }
}

struct Fields {
    fields: Punctuated<Field, Token![,]>,
}

impl Parse for Fields {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        braced!(content in input);
        Ok(Self {
            fields: content.parse_terminated(Field::parse, Token![,])?,
        })
    }
}

struct Field {
    ident: Ident,
    ty: syn::Type,
}

impl Parse for Field {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty = input.parse()?;
        Ok(Self { ident, ty })
    }
}

struct MethodAttrs {
    attrs: Punctuated<MethodAttr, Token![,]>,
}

impl Parse for MethodAttrs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(MethodAttr::parse, Token![,])
            .map(|attrs| Self { attrs })
    }
}

impl IntoIterator for MethodAttrs {
    type IntoIter = syn::punctuated::IntoIter<MethodAttr>;
    type Item = MethodAttr;

    fn into_iter(self) -> Self::IntoIter {
        self.attrs.into_iter()
    }
}

enum MethodAttr {
    ConnectionFnFlag(Span),
    ResourceFnFlag(Span),
}

impl MethodAttr {
    const fn as_connection_fn_flag(&self) -> Option<Span> {
        if let Self::ConnectionFnFlag(v) = self {
            Some(*v)
        } else {
            None
        }
    }

    const fn as_resource_fn_flag(&self) -> Option<Span> {
        if let Self::ResourceFnFlag(v) = self {
            Some(*v)
        } else {
            None
        }
    }
}

impl Parse for MethodAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        if ident == "connection_fn" {
            Ok(Self::ConnectionFnFlag(ident.span()))
        } else if ident == "resource_fn" {
            Ok(Self::ResourceFnFlag(ident.span()))
        } else {
            Err(input.error(format!(
                "expected 'connection_fn' or 'resource_fn', found {ident}"
            )))
        }
    }
}

const fn impl_item_as_fn(item: &syn::ImplItem) -> Option<&syn::ImplItemFn> {
    if let syn::ImplItem::Fn(func) = item {
        Some(func)
    } else {
        None
    }
}

const fn fn_arg_as_typed(fn_arg: &syn::FnArg) -> Option<&syn::PatType> {
    if let syn::FnArg::Typed(typed) = fn_arg {
        Some(typed)
    } else {
        None
    }
}

fn doc_private(vis: &syn::Visibility) -> TokenStream {
    if matches!(
        vis,
        syn::Visibility::Inherited | syn::Visibility::Restricted(_)
    ) {
        quote! {
            #[doc(hidden)]
            #[allow(private_interfaces)]
        }
    } else {
        quote! {}
    }
}
