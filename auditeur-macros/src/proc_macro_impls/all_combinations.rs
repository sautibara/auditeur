use proc_macro2::{Group, Ident, TokenStream, TokenTree};
use quote::{quote, ToTokens};
use syn::{
    bracketed, parenthesized,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    token::{Bracket, Paren},
    Token,
};

use crate::field::{NameValue, NameValueArray};

pub fn all_combinations(config: &AllCombinations) -> syn::Result<proc_macro2::TokenStream> {
    let mut lists = config
        .rules
        .values
        .iter()
        .flat_map(|rule| [&rule.in_set, &rule.out_of_set])
        .filter_map(|output| output.value.as_list());

    let Some(first_list) = lists.next() else {
        return Err(syn::Error::new(
            config.rules.bracket.span.span(),
            "Expected at least one 'List' rule.",
        ));
    };

    for list in lists {
        if list.tokens.len() != first_list.tokens.len() {
            return Err(syn::Error::new_spanned(
                &list.tokens,
                format!(
                    "Expected to have {} items, had {}",
                    first_list.tokens.len(),
                    list.tokens.len()
                ),
            ));
        }
    }

    let mut combinations = TokenStream::new();

    let num_items = first_list.tokens.len();
    let rule_count = config.rules.values.len();

    // this is a dummy value for the list
    // since we're taking note of the lengths of each rule's output, it's fine to fill the array
    // with dummy values
    let unit_token_tree = TokenTree::Group(Group::new(
        proc_macro2::Delimiter::Parenthesis,
        TokenStream::default(),
    ));

    // the outputs of each rule squashed into one vector
    // each rule's entire output is put one after another
    let mut rule_values = vec![&unit_token_tree; rule_count * num_items];
    let mut rule_lengths = vec![0; rule_count];

    let combination_count = 1 << num_items;
    for combination in 0..combination_count {
        // the values don't need to be filled since we're taking note of the lengths anyways
        rule_lengths.fill(0);

        for item_index in 0..num_items {
            let index_bit = 1 << item_index;
            let in_set = combination & index_bit > 0;

            for (rule_index, rule) in config.rules.values.iter().enumerate() {
                let output = if in_set {
                    &rule.in_set
                } else {
                    &rule.out_of_set
                };

                let to_add = match &output.value {
                    OutputType::Repeated(repeated) => Some(&repeated.value),
                    OutputType::List(list) => list.tokens.get(item_index),
                    OutputType::None { .. } => None,
                };

                if let Some(value) = to_add {
                    let current_rule_length = &mut rule_lengths[rule_index];
                    rule_values[rule_index * num_items + *current_rule_length] = value;
                    *current_rule_length += 1;
                }
            }
        }

        let rule_values = rule_values
            .chunks_exact(num_items)
            .zip(&rule_lengths)
            .map(|(chunk, length)| &chunk[..*length]);

        let macro_ident = &config.macro_ident.value;
        let passthru = config
            .passthru()
            .map_or_else(Default::default, ToTokens::to_token_stream);

        combinations = quote! {
            #combinations
            #macro_ident!([#((#(#rule_values),*)),*] #passthru);
        };
    }

    Ok(combinations)
}

pub struct AllCombinations {
    macro_ident: NameValue<Ident>,
    _comma1: Token![,],
    rules: NameValueArray<Rule>,
    passthru: Option<(Token![,], NameValue<TokenTree>)>,
}

impl AllCombinations {
    fn passthru(&self) -> Option<&TokenTree> {
        self.passthru.as_ref().map(|val| &val.1.value)
    }
}

impl Parse for AllCombinations {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let macro_ident = NameValue::<Ident>::parse(input, "macro_ident")?;
        let comma1 = input.parse()?;
        let rules = NameValueArray::<Rule>::parse(input, "rules")?;

        let passthru = input
            .peek(Token![,])
            .then(|| {
                Ok::<_, syn::Error>((
                    input.parse()?,
                    NameValue::<TokenTree>::parse(input, "passthru")?,
                ))
            })
            .transpose()?;

        Ok(Self {
            macro_ident,
            _comma1: comma1,
            rules,
            passthru,
        })
    }
}

struct Rule {
    _parens: Paren,
    in_set: NameValue<OutputType>,
    _comma1: Token![,],
    out_of_set: NameValue<OutputType>,
    _comma2: Option<Token![,]>,
}

impl Parse for Rule {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        Ok(Self {
            _parens: parenthesized!(content in input),
            in_set: NameValue::<OutputType>::parse(&content, "in_set")?,
            _comma1: content.parse()?,
            out_of_set: NameValue::<OutputType>::parse(&content, "out_of_set")?,
            _comma2: content
                .peek(Token![,])
                .then(|| content.parse())
                .transpose()?,
        })
    }
}

struct ListOutput {
    _list_ident: Ident,
    _bracket: Bracket,
    pub tokens: Punctuated<TokenTree, Token![,]>,
}

struct RepeatedOutput {
    _just_ident: Ident,
    _parens: Paren,
    pub value: TokenTree,
}

enum OutputType {
    Repeated(RepeatedOutput),
    List(ListOutput),
    None { _none_ident: Ident },
}

impl OutputType {
    pub const fn as_list(&self) -> Option<&ListOutput> {
        if let Self::List(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl Parse for OutputType {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;

        let content;
        match &*ident.to_string() {
            "Just" => Ok(Self::Repeated(RepeatedOutput {
                _just_ident: ident,
                _parens: parenthesized!(content in input),
                value: content.parse()?,
            })),
            "List" => Ok(Self::List(ListOutput {
                _list_ident: ident,
                _bracket: bracketed!(content in input),
                tokens: content.parse_terminated(TokenTree::parse, Token![,])?,
            })),
            "None" => Ok(Self::None { _none_ident: ident }),
            _ => Err(syn::Error::new_spanned(
                ident,
                "Expected one of Just, List, or None",
            )),
        }
    }
}
