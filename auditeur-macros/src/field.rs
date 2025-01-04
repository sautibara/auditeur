use syn::{
    bracketed,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token::Bracket,
    Ident, Token,
};

pub struct Field<T: Parse> {
    pub _ident: Ident,
    pub _colon: Token![:],
    pub value: T,
}

impl<T: Parse> Field<T> {
    pub fn parse(input: ParseStream, expected_ident: &str) -> syn::Result<Self> {
        Ok(Self {
            _ident: {
                let ident: Ident = input.parse()?;
                if ident == expected_ident {
                    Ok(ident)
                } else {
                    Err(syn::Error::new_spanned(
                        ident,
                        format!("expected '{expected_ident}'"),
                    ))
                }
            }?,
            _colon: input.parse()?,
            value: input.parse()?,
        })
    }
}

#[allow(clippy::module_name_repetitions)] // dnc
pub struct ArrayField<T: Parse> {
    pub _ident: Ident,
    pub _colon: Token![:],
    pub bracket: Bracket,
    pub values: Punctuated<T, Token![,]>,
}

impl<T: Parse> ArrayField<T> {
    pub fn parse(input: ParseStream, expected_ident: &str) -> syn::Result<Self> {
        let content;
        Ok(Self {
            _ident: {
                let ident: Ident = input.parse()?;
                if ident == expected_ident {
                    Ok(ident)
                } else {
                    Err(syn::Error::new_spanned(
                        ident,
                        format!("expected '{expected_ident}'"),
                    ))
                }
            }?,
            _colon: input.parse()?,
            bracket: bracketed!(content in input),
            values: content.parse_terminated(T::parse, Token![,])?,
        })
    }
}
