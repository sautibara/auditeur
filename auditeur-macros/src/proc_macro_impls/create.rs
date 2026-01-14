use proc_macro2::Span;
use quote::quote;
use syn::{
    FnArg, Ident, ImplItem, ItemImpl, Pat, Type, parse_quote, punctuated::Punctuated,
    spanned::Spanned,
};

use crate::extract_attr;

struct InputList {
    name: &'static str,
    types: Vec<Type>,
    patterns: Vec<Pat>,
}

impl InputList {
    const fn new(name: &'static str) -> Self {
        Self {
            name,
            types: vec![],
            patterns: vec![],
        }
    }

    fn push(&mut self, ty: Type, pat: Pat) {
        self.types.push(ty);
        self.patterns.push(pat);
    }

    fn ty(&self) -> proc_macro2::TokenStream {
        let ty = self.types.iter();
        quote! { (#(#ty),*) }
    }

    fn pat(&self) -> proc_macro2::TokenStream {
        let pat = self.patterns.iter();
        quote! { (#(#pat),*) }
    }

    fn type_def(&self) -> ImplItem {
        let name = Ident::new(self.name, Span::call_site());
        let ty = self.ty();
        parse_quote! { type #name = #ty; }
    }

    fn arg(&self) -> FnArg {
        let ty = self.ty();
        let pat = self.pat();
        parse_quote!(#pat: #ty)
    }
}

pub fn create(mut implementation: ItemImpl) -> proc_macro2::TokenStream {
    let mut external_inputs = InputList::new("ExternalInput");
    let mut registry_inputs = InputList::new("RegistryInput");

    let mut funcs: Vec<_> = implementation
        .items
        .iter_mut()
        .filter_map(|item| {
            if let ImplItem::Fn(func) = item {
                Some(func)
            } else {
                None
            }
        })
        .collect();

    let [func] = &mut funcs[..] else {
        // silence missing type errors
        implementation.items.push(parse_quote!(
            type ExternalInput = ();
        ));
        implementation.items.push(parse_quote!(
            type RegistryInput = ();
        ));
        // the compiler will already complain if there is no create function present
        return quote! { #implementation };
    };

    for param in &mut func.sig.inputs {
        let FnArg::Typed(pat) = param else {
            return syn::Error::new(param.span(), "cannot have a self input").into_compile_error();
        };

        let external = extract_attr(&mut pat.attrs, "external").next().is_some();
        if external {
            external_inputs.push((*pat.ty).clone(), (*pat.pat).clone());
        } else {
            registry_inputs.push((*pat.ty).clone(), (*pat.pat).clone());
        }
    }

    let mut inputs = Punctuated::new();
    inputs.push(registry_inputs.arg());
    inputs.push(external_inputs.arg());
    func.sig.inputs = inputs;

    implementation.items.push(registry_inputs.type_def());
    implementation.items.push(external_inputs.type_def());

    quote! {
        #implementation
    }
}
