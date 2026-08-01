use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, TokenStreamExt};
use syn::Token;

pub struct Signature<'a> {
    pub vis: &'a syn::Visibility,
    pub sig: &'a syn::Signature,
    pub asyncness: Option<bool>,
    pub receiver: Option<TokenStream>,
    pub output: Option<TokenStream>,
}

impl<'a> Signature<'a> {
    pub const fn new(vis: &'a syn::Visibility, sig: &'a syn::Signature) -> Self {
        Self {
            vis,
            sig,
            asyncness: None,
            receiver: None,
            output: None,
        }
    }

    pub fn asyncness(self, asyncness: bool) -> Self {
        Self {
            asyncness: Some(asyncness),
            ..self
        }
    }

    pub fn receiver(self, receiver: TokenStream) -> Self {
        Self {
            receiver: Some(receiver),
            ..self
        }
    }

    pub fn output(self, output: TokenStream) -> Self {
        Self {
            output: Some(output),
            ..self
        }
    }
}

impl ToTokens for Signature<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.vis.to_tokens(tokens);
        self.sig.constness.to_tokens(tokens);
        if let Some(asyncness) = self.asyncness {
            if asyncness {
                Token![async](Span::call_site()).to_tokens(tokens);
            }
        } else {
            self.sig.asyncness.to_tokens(tokens);
        }
        self.sig.unsafety.to_tokens(tokens);
        self.sig.abi.to_tokens(tokens);
        self.sig.fn_token.to_tokens(tokens);
        self.sig.ident.to_tokens(tokens);
        self.sig.generics.to_tokens(tokens);
        self.sig.paren_token.surround(tokens, |tokens| {
            if let Some(receiver) = &self.receiver {
                for pair in self.sig.inputs.pairs() {
                    if matches!(pair.value(), syn::FnArg::Receiver(_)) {
                        receiver.to_tokens(tokens);
                    } else {
                        pair.value().to_tokens(tokens);
                    }
                    pair.punct().to_tokens(tokens);
                }
            } else {
                self.sig.inputs.to_tokens(tokens);
            }
            if let Some(variadic) = &self.sig.variadic {
                if !self.sig.inputs.empty_or_trailing() {
                    <Token![,]>::default().to_tokens(tokens);
                }
                variadic.to_tokens(tokens);
            }
        });
        if let Some(output) = &self.output {
            output.to_tokens(tokens);
        } else {
            self.sig.output.to_tokens(tokens);
        }
        self.sig.generics.where_clause.to_tokens(tokens);
    }
}

pub struct ImplItemFn<'a> {
    pub item: &'a syn::ImplItemFn,
    pub remove_attr: Option<&'static str>,
}

impl<'a> ImplItemFn<'a> {
    pub const fn new(item: &'a syn::ImplItemFn) -> Self {
        Self {
            item,
            remove_attr: None,
        }
    }

    pub const fn remove_attr(self, attr: &'static str) -> Self {
        Self {
            remove_attr: Some(attr),
            ..self
        }
    }
}

impl ToTokens for ImplItemFn<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if let Some(path) = self.remove_attr {
            tokens.append_all(
                outer_attrs(&self.item.attrs).filter(|attr| !attr.path().is_ident(path)),
            );
        } else {
            tokens.append_all(outer_attrs(&self.item.attrs));
        }
        self.item.vis.to_tokens(tokens);
        self.item.defaultness.to_tokens(tokens);
        self.item.sig.to_tokens(tokens);
        self.item.block.brace_token.surround(tokens, |tokens| {
            if let Some(path) = self.remove_attr {
                tokens.append_all(
                    inner_attrs(&self.item.attrs).filter(|attr| !attr.path().is_ident(path)),
                );
            } else {
                tokens.append_all(inner_attrs(&self.item.attrs));
            }
            tokens.append_all(&self.item.block.stmts);
        });
    }
}

fn outer_attrs(attrs: &[syn::Attribute]) -> impl Iterator<Item = &syn::Attribute> {
    attrs.iter().filter(|attr| match attr.style {
        syn::AttrStyle::Outer => true,
        syn::AttrStyle::Inner(_) => false,
    })
}

fn inner_attrs(attrs: &[syn::Attribute]) -> impl Iterator<Item = &syn::Attribute> {
    attrs.iter().filter(|attr| match attr.style {
        syn::AttrStyle::Inner(_) => true,
        syn::AttrStyle::Outer => false,
    })
}
