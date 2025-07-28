use quote::{quote_spanned, ToTokens};
use syn::{
    parse::Parse,
    punctuated::{Pair, Punctuated},
    spanned::Spanned,
    token::Paren,
    Lifetime, Token, Type,
};

enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<I, L: Iterator<Item = I>, R: Iterator<Item = I>> Iterator for Either<L, R> {
    type Item = I;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(left) => left.next(),
            Self::Right(right) => right.next(),
        }
    }
}

use crate::field::Parenthesized;

#[derive(Clone, Debug)]
pub enum InputList {
    Single(Type),
    Multi(Parenthesized<Punctuated<Type, Token![,]>>),
}

impl Default for InputList {
    fn default() -> Self {
        Self::Multi(Parenthesized::default_with(Punctuated::default()))
    }
}

impl InputList {
    pub fn into_reference_list(self) -> InputReferenceList {
        match self {
            Self::Single(typ) => InputReferenceList::Single(typ.into()),
            Self::Multi(paren) => InputReferenceList::Multi(Parenthesized {
                paren: paren.paren,
                value: paren
                    .value
                    .into_pairs()
                    .map(|pair| {
                        let (input, comma) = pair.into_tuple();
                        Pair::new(input.into(), comma)
                    })
                    .collect(),
            }),
        }
    }

    fn to_empty_combination(&self) -> Combination {
        let mut list = Combination::new();
        for input in self.inputs() {
            list.push(SetSelection::new(false, input.span()));
        }
        list
    }

    fn inputs(&self) -> impl Iterator<Item = &Type> {
        match self {
            Self::Single(val) => Either::Left(std::iter::once(val)),
            Self::Multi(vals) => Either::Right(vals.value.iter()),
        }
    }

    pub fn merge(&mut self, other: &Self) -> syn::Result<Combination> {
        let mut combination = self.to_empty_combination();
        let mut last_added = None;

        for other_input in other.inputs() {
            let pos = self
                .inputs()
                .position(|self_input| self_input == other_input);

            if let Some(pos) = pos {
                combination.set_selected(pos);
                if last_added.is_some_and(|last_added| pos <= last_added) {
                    return Err(syn::Error::new_spanned(other_input, "incorrect order of inputs: all inputs to a resource must be in the same relative order to a limitation in SubsetMut"));
                }
                last_added = Some(pos);
            } else {
                self.push_back(other_input.clone());
                combination.push(SetSelection::new(true, other_input.span()));
            }
        }

        Ok(combination)
    }

    pub fn push_back(&mut self, val: Type) {
        let res = match std::mem::take(self) {
            Self::Single(first) => Self::Multi(Parenthesized::default_with(
                [first, val].into_iter().collect(),
            )),
            Self::Multi(vals) if vals.value.is_empty() => Self::Single(val),
            Self::Multi(mut vals) => {
                vals.value.push(val);
                Self::Multi(vals)
            }
        };
        *self = res;
    }

    pub fn push_front(&mut self, val: Type) {
        let res = match std::mem::take(self) {
            Self::Single(first) => Self::Multi(Parenthesized::default_with(
                [val, first].into_iter().collect(),
            )),
            Self::Multi(vals) if vals.value.is_empty() => Self::Single(val),
            Self::Multi(mut vals) => {
                vals.value.insert(0, val);
                Self::Multi(vals)
            }
        };
        *self = res;
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Multi(vals) => vals.value.len(),
        }
    }
}

impl Parse for InputList {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(Paren) {
            Ok(Self::Multi(Parenthesized::parse_punctuated(
                input,
                Token![,],
            )?))
        } else {
            Ok(Self::Single(input.parse()?))
        }
    }
}

impl ToTokens for InputList {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Single(val) => val.to_tokens(tokens),
            Self::Multi(vals) => vals.to_tokens(tokens),
        }
    }
}

#[derive(Clone, Debug)]
pub enum InputReferenceList {
    Single(InputReference),
    Multi(Parenthesized<Punctuated<InputReference, Token![,]>>),
}

impl InputReferenceList {
    #[expect(dead_code)]
    pub fn owned_to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Single(reference) => reference.to_tokens(tokens),
            Self::Multi(multi) => multi.to_tokens_with(tokens, |tokens| {
                for pair in multi.value.pairs() {
                    pair.value().owned_to_tokens(tokens);
                    if let Some(punct) = pair.punct() {
                        punct.to_tokens(tokens);
                    }
                }
            }),
        }
    }

    pub fn into_owned_list(self) -> InputList {
        match self {
            Self::Single(typ) => InputList::Single(typ.into()),
            Self::Multi(paren) => InputList::Multi(Parenthesized {
                paren: paren.paren,
                value: paren
                    .value
                    .into_pairs()
                    .map(|pair| {
                        let (input, comma) = pair.into_tuple();
                        Pair::new(input.into(), comma)
                    })
                    .collect(),
            }),
        }
    }

    fn inputs_mut(&mut self) -> impl Iterator<Item = &mut InputReference> {
        match self {
            Self::Single(val) => Either::Left(std::iter::once(val)),
            Self::Multi(vals) => Either::Right(vals.value.iter_mut()),
        }
    }

    pub fn with_lifetime(mut self, lifetime: &str) -> Self {
        for input in self.inputs_mut() {
            input.lifetime = Some(syn::Lifetime::new(lifetime, input.and_token.span()));
        }
        self
    }
}

impl Parse for InputReferenceList {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(Paren) {
            Ok(Self::Multi(Parenthesized::parse_punctuated(
                input,
                Token![,],
            )?))
        } else {
            Ok(Self::Single(input.parse()?))
        }
    }
}

impl ToTokens for InputReferenceList {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Single(val) => val.to_tokens(tokens),
            Self::Multi(vals) => vals.to_tokens(tokens),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InputReference {
    and_token: Token![&],
    lifetime: Option<Lifetime>,
    mut_token: Token![mut],
    input: Box<Type>,
}

impl InputReference {
    pub fn owned_to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.input.to_tokens(tokens);
    }
}

impl From<Type> for InputReference {
    fn from(input: Type) -> Self {
        let span = input.span();
        Self {
            and_token: Token![&](span),
            lifetime: None,
            mut_token: Token![mut](span),
            input: Box::new(input),
        }
    }
}

impl From<InputReference> for Type {
    fn from(value: InputReference) -> Self {
        *value.input
    }
}

impl Parse for InputReference {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            and_token: input.parse().map_err(|err| {
                syn::Error::new(err.span(), format!("{err}. Are you missing an input list?"))
            })?,
            lifetime: input
                .peek(syn::Lifetime)
                .then(|| input.parse())
                .transpose()?,
            mut_token: input.parse()?,
            input: input.parse()?,
        })
    }
}

impl ToTokens for InputReference {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.and_token.to_tokens(tokens);
        self.lifetime.to_tokens(tokens);
        self.mut_token.to_tokens(tokens);
        self.input.to_tokens(tokens);
    }
}

#[derive(Clone, Debug)]
pub struct SetSelection {
    pub in_set: bool,
    pub span: proc_macro2::Span,
}

impl SetSelection {
    pub const fn new(in_set: bool, span: proc_macro2::Span) -> Self {
        Self { in_set, span }
    }
}

impl ToTokens for SetSelection {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let input_mod = crate::AUDITEUR.input();
        if self.in_set {
            quote_spanned! { self.span => #input_mod::subset::InSet }.to_tokens(tokens);
        } else {
            quote_spanned! { self.span => #input_mod::subset::OutOfSet }.to_tokens(tokens);
        }
    }
}

pub struct Combination {
    pub selections: Punctuated<SetSelection, Token![,]>,
}

impl Combination {
    pub const fn new() -> Self {
        Self {
            selections: Punctuated::new(),
        }
    }

    pub fn push(&mut self, selection: SetSelection) {
        self.selections.push(selection);
    }

    pub fn pad_to_length(&mut self, length: usize) {
        let span = self.span();
        while self.selections.len() < length {
            self.selections.push(SetSelection::new(false, span));
        }
    }

    pub fn set_selected(&mut self, index: usize) {
        self.selections[index].in_set = true;
    }
}

impl ToTokens for Combination {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let span = self.selections.span();
        Paren(span).surround(tokens, |tokens| {
            self.selections.to_tokens(tokens);
        });
    }
}
