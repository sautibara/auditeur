use syn::{fold::Fold, parse::Parse};

use crate::Either;

pub fn collect_attrs<T, C>(
    ident: &'static str,
    attrs: &[syn::Attribute],
) -> impl Iterator<Item = syn::Result<T>>
where
    C: Parse + IntoIterator<Item = T>,
{
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident(ident))
        .flat_map(|attr| match &attr.meta {
            syn::Meta::List(list) => match syn::parse2::<C>(list.tokens.clone()) {
                Ok(collection) => Either::Left(collection.into_iter().map(Ok)),
                Err(err) => Either::Right(Either::Left(std::iter::once(Err(err)))),
            },
            syn::Meta::Path(_) => Either::Right(Either::Right(std::iter::empty())),
            syn::Meta::NameValue(name_value) => Either::Right(Either::Left(std::iter::once(Err(
                syn::Error::new_spanned(name_value, "expected list, found name-value"),
            )))),
        })
}

pub struct FoldAttr {
    to_remove: &'static str,
}

impl FoldAttr {
    pub const fn remove(name: &'static str) -> Self {
        Self { to_remove: name }
    }
}

impl Fold for FoldAttr {
    fn fold_attributes(&mut self, i: Vec<syn::Attribute>) -> Vec<syn::Attribute> {
        let to_remove = self.to_remove;
        i.into_iter()
            .filter(|attr| {
                attr.meta
                    .path()
                    .get_ident()
                    .is_none_or(|ident| ident != to_remove)
            })
            .map(|node| syn::fold::fold_attribute(self, node))
            .collect()
    }
}
