// #[listener(
//     id = "core:walker",
//     listen(message::Queue, message::Audio(input = (Sender, Get<Library>, Get<Player>))),
//     respond(...), // not implemented yet, same as listen though
//     registry_component = true,
//     components((
//         type = ListenComponent<Self, message::Queue>,
//         // can give input with one of the following two:
//         implementation = Listen<message::Queue>::on,
//         input = (Sender, Get<Library>),
//         // it could also get the implementation directly from the type
//         // using a macro with the same path
//     )),
// )]

// TODO: allow specifying inputs for Runnable

use std::ops::{Deref, DerefMut};

use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parenthesized,
    parse::Parse,
    parse_quote,
    punctuated::{Pair, Punctuated},
    spanned::Spanned,
    token::Paren,
    Generics, Ident, LitBool, Token, Type, WhereClause,
};

use crate::{
    custom_generics::CustomGenerics,
    field::{Comma, List, NameValue, Parenthesized},
    input_list::{Combination, InputList},
    namespaced_id::NamespacedId,
    type_from_ident, Auditeur,
};

use super::input_reflection::CallbackList;

fn split_results<Ok, Err, I, OkAcc, ErrAcc>(iter: I) -> (OkAcc, ErrAcc)
where
    I: Iterator<Item = Result<Ok, Err>>,
    OkAcc: Default + Extend<Ok>,
    ErrAcc: Default + Extend<Err>,
{
    iter.fold(
        (OkAcc::default(), ErrAcc::default()),
        |(mut ok_acc, mut err_acc), res| {
            match res {
                Ok(ok) => ok_acc.extend(Some(ok)),
                Err(err) => err_acc.extend(Some(err)),
            }
            (ok_acc, err_acc)
        },
    )
}

pub fn runnable_derive(implementer: &Type, generics: &Generics) -> syn::Result<TokenStream> {
    let no_input = InputList::default();

    let runnable_trait = crate::type_from_ident(Ident::new("Runnable", Span::call_site()));
    let init_input_macro_ident =
        super::input_reflection::input_macro_ident_of_method(&runnable_trait, implementer, "init")?;
    let exit_input_macro_ident =
        super::input_reflection::input_macro_ident_of_method(&runnable_trait, implementer, "exit")?;

    let init_input_macro = super::export_input(&init_input_macro_ident, &no_input);
    let exit_input_macro = super::export_input(&exit_input_macro_ident, &no_input);

    let resource = crate::AUDITEUR.resource();
    let anyhow = crate::AUDITEUR.anyhow();

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        #init_input_macro
        #exit_input_macro
        impl #impl_generics #resource::Runnable for #implementer #type_generics #where_clause {
            type InitInput<'a> = ();
            type ExitInput<'a> = ();

            async fn init(&mut self, (): Self::InitInput<'_>) -> #anyhow::Result<()> { Ok(()) }
            async fn exit(&mut self, (): Self::ExitInput<'_>) -> #anyhow::Result<()> { Ok(()) }
        }
    })
}

/// An intermediate representation of a [`Listener`] and its callbacks
#[derive(Debug)]
pub struct Intermediate {
    callbacks: Parenthesized<CallbackList>,
    params: Parenthesized<Vec<Type>>,
    implementer: Ident,
    listener: Parenthesized<Listener>,
    generics: Parenthesized<Generics>,
    where_clause: Parenthesized<Option<WhereClause>>,
    init_input: Option<InputList>,
    exit_input: Option<InputList>,
    input_lists: Vec<InputList>,
}

impl Intermediate {
    fn new(implementer: Ident, mut generics: Generics, listener: Listener) -> Self {
        Self {
            callbacks: Parenthesized::default(),
            params: Parenthesized::default(),
            implementer,
            listener: Parenthesized::default_with(listener),
            where_clause: Parenthesized::default_with(std::mem::take(&mut generics.where_clause)),
            generics: Parenthesized::default_with(generics),
            init_input: None,
            exit_input: None,
            input_lists: Vec::new(),
        }
    }

    #[allow(clippy::result_large_err)] // to give back self - can't box
    fn into_callback(mut self) -> Result<proc_macro2::TokenStream, Self> {
        let mut pairs = self.callbacks.value.list.into_pairs();
        let first = pairs.next();
        self.callbacks.value.list = pairs.collect();

        let Some(pair) = first else {
            return Err(self);
        };

        let first_callback = pair.into_value();
        Ok(quote! { #first_callback!(#self); })
    }

    fn not_enough_inputs_error(&self) -> syn::Error {
        syn::Error::new_spanned(
            &self.listener.value,
            format!(
                "didn't receive enough input lists through macros - expected {}, received: {}",
                2 + self
                    .listener
                    .value
                    .components()
                    .filter(|component| component.as_input().is_err())
                    .count(),
                (self.init_input.iter())
                    .chain(self.exit_input.iter())
                    .chain(self.input_lists.iter())
                    .map(|list| list.into_token_stream().to_string())
                    .fold(String::new(), |string, list| string + &list + " ")
            ),
        )
    }

    fn apply_input(&mut self) -> syn::Result<()> {
        let mut inputs = self.input_lists.drain(..);

        {
            let mut listen_components = self
                .listener
                .value
                .builtin_components_mut()
                .filter(|component| !component.has_input());

            for (listen_component, input) in listen_components.by_ref().zip(inputs.by_ref()) {
                listen_component.set_input(input);
            }

            let has_remaining_component = listen_components.next().is_some();
            if has_remaining_component {
                drop((inputs, listen_components));
                return Err(self.not_enough_inputs_error());
            }
        }

        {
            let mut custom_components = self
                .listener
                .value
                .custom_components_mut()
                .filter(|component| !component.has_input());

            for (custom_component, input) in custom_components.by_ref().zip(inputs.by_ref()) {
                custom_component.set_input(input);
            }

            let has_remaining_component = custom_components.next().is_some();
            if has_remaining_component {
                drop((inputs, custom_components));
                return Err(self.not_enough_inputs_error());
            }
        }

        let inputs: Vec<_> = inputs.collect();
        self.input_lists = inputs;

        Ok(())
    }

    pub fn expand(mut self) -> syn::Result<proc_macro2::TokenStream> {
        self.apply_input()?;
        self.expand_simple()
    }

    fn expand_simple(mut self) -> syn::Result<proc_macro2::TokenStream> {
        self.expand_respond()?;

        let (with_input, without_input): (Vec<_>, Vec<_>) =
            split_results(self.listener.value.components().map(ComponentRef::as_input));

        if let (Some(init_input), Some(exit_input), true) = (
            &self.init_input,
            &self.exit_input,
            without_input.is_empty() && self.callbacks.value.list.is_empty(),
        ) {
            // we have all the inputs, bless
            let mut inputs = InputList::default();
            let mut combinations: Punctuated<_, Token![,]> = Punctuated::new();

            let include_registry_component = self.listener.value.include_registry_component();
            let registry_component = if include_registry_component {
                let resource_mod = crate::AUDITEUR.resource();
                combinations.push(Combination::new());
                Some(parse_quote!( #resource_mod::listener::RegistryComponent ))
            } else {
                None
            };

            let mut init_combination = inputs.merge(init_input)?;
            let mut exit_combination = inputs.merge(exit_input)?;

            for component in &with_input {
                let combination = inputs.merge(component.specifier)?;
                combinations.push(combination);
            }

            init_combination.pad_to_length(inputs.len());
            exit_combination.pad_to_length(inputs.len());

            for combination in &mut combinations {
                combination.pad_to_length(inputs.len());
            }

            // the first input must be () because of Subset
            inputs.push_front(parse_quote!(()));

            let unit_type = parse_quote!(());
            let components = std::iter::once(&unit_type)
                .chain(registry_component.as_ref())
                .chain(with_input.iter().map(|component| component.component));

            let implementer = &self.implementer;

            let auditeur = Auditeur::new();
            let resource_mod = auditeur.resource();

            self.generics.value.where_clause = self.where_clause.value;

            let resource_impl = super::resource_impls_with_generics(
                &self.implementer,
                &self.listener.value.id.value,
                &CustomGenerics::from_generics(&self.generics.value),
            );

            let (impl_generics, type_generics, where_clause) = self.generics.value.split_for_impl();

            Ok(quote! {
                #resource_impl

                impl #impl_generics #resource_mod::listener::Listener for #implementer #type_generics #where_clause {
                    type Input = #inputs;
                    type Components = (#(#components),*);
                    type ComponentCombinations = (#combinations);
                    type InitCombination = #init_combination;
                    type ExitCombination = #exit_combination;
                }
            })
        } else {
            // we have to fill the rest of the inputs :(

            let auditeur = &*crate::AUDITEUR;

            // use closures here to get around a lack of view types
            let mut add_callback = |path| self.callbacks.value.list.push(path);
            let mut add_specifier_param = |typ| self.params.value.push(typ);

            let implementer_type = crate::type_from_ident(self.implementer.clone());

            let runnable = parse_quote!(Runnable);
            if self.init_input.is_none() {
                add_callback(super::input_reflection::input_macro_of_method(
                    &runnable,
                    &implementer_type,
                    "init",
                )?);
            }
            if self.exit_input.is_none() {
                add_callback(super::input_reflection::input_macro_of_method(
                    &runnable,
                    &implementer_type,
                    "exit",
                )?);
            }

            for component in without_input {
                match component.specifier {
                    SpecifierRef::None => {
                        add_callback(component.macro_ident().into());
                        add_specifier_param(component.component.clone());
                        add_callback(parse_quote!( #auditeur::expand_input_specifier ));
                    }
                    SpecifierRef::Implementation(typ) => {
                        let input_macro =
                            super::input_reflection::input_macro_of(typ, &implementer_type)?;
                        add_callback(input_macro);
                    }
                    SpecifierRef::Input(_) => unreachable!(),
                }
            }

            add_callback(parse_quote!( #auditeur::expand_listener ));

            Ok(self
                .into_callback()
                .expect("there should be at least one callback"))
        }
    }

    fn expand_respond(&mut self) -> syn::Result<()> {
        let parameters = std::mem::take(&mut self.listener.value.parameters);
        let mut parameters: Punctuated<Parameter, Token![,]> = parameters
            .into_pairs()
            .filter_map(|pair| {
                let (param, punct) = pair.into_tuple();
                let res = self.handle_parameter(param);
                let res = res.transpose()?;
                Some(res.map(|param| Pair::new(param, punct)))
            })
            .collect::<syn::Result<_>>()?;
        if !parameters.trailing_punct() && !parameters.is_empty() {
            parameters.push_punct(Token![,](Span::call_site()));
        }
        self.listener.value.parameters = parameters;

        Ok(())
    }

    fn handle_parameter(&mut self, parameter: Parameter) -> syn::Result<Option<Parameter>> {
        match parameter {
            Parameter::RespondFlag(_) => {
                let macro_path = super::respond_macro_ident(&self.implementer);
                self.callbacks
                    .value
                    .list
                    .push(parse_quote!(#macro_path::#macro_path));
                Ok(None)
            }
            Parameter::Respond(ident, mut meta) => {
                if meta.value.id.is_none() {
                    meta.value.id = Some(self.listener.value.id.clone());
                }
                let config = meta.value.to_config(&self.implementer)?;
                let mut values = Punctuated::new();
                values.push(BuiltinComponent::new(
                    type_from_ident((*config.query.message_ident).clone()),
                    None,
                    BuiltinComponentType::Take,
                ));
                if let Some(config) = &config.transaction {
                    values.push(BuiltinComponent::new(
                        type_from_ident((*config.message_ident).clone()),
                        None,
                        BuiltinComponentType::Take,
                    ));
                }
                Ok(Some(Parameter::Take(List {
                    ident: Ident::new("take", ident.span()),
                    parens: meta.paren,
                    values,
                })))
            }
            other => Ok(Some(other)),
        }
    }
}

impl Parse for Intermediate {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            callbacks: Parenthesized::parse_empty(input)?,
            params: Parenthesized::parse_empty(input)?,
            implementer: input.parse()?,
            listener: input.parse()?,
            generics: input.parse()?,
            where_clause: Parenthesized::parse_with(input, |input| {
                input.peek(Token![where]).then(|| input.parse()).transpose()
            })?,
            init_input: (!input.is_empty()).then(|| input.parse()).transpose()?,
            exit_input: (!input.is_empty()).then(|| input.parse()).transpose()?,
            input_lists: {
                let mut lists = Vec::new();
                while !input.is_empty() {
                    lists.push(input.parse()?);
                }
                lists
            },
        })
    }
}

impl ToTokens for Intermediate {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.callbacks.to_tokens(tokens);
        self.params.paren.surround(tokens, |tokens| {
            for param in &self.params.value {
                param.to_tokens(tokens);
            }
        });
        self.implementer.to_tokens(tokens);
        self.listener.to_tokens(tokens);

        self.generics.to_tokens(tokens);
        self.where_clause.to_tokens(tokens);

        self.init_input.to_tokens(tokens);
        self.exit_input.to_tokens(tokens);
        for input_list in &self.input_lists {
            input_list.to_tokens(tokens);
        }
    }
}

#[derive(Debug)]
pub struct Listener {
    id: Comma<NamespacedId>,
    parameters: Punctuated<Parameter, Token![,]>,
}

impl Listener {
    fn include_registry_component(&self) -> bool {
        self.parameters
            .iter()
            .filter_map(Parameter::as_registry_component)
            .next_back()
            .is_none_or(|boolean| boolean.value.value)
    }

    fn builtin_components(&self) -> impl Iterator<Item = &BuiltinComponent> {
        self.parameters.iter().flat_map(Parameter::as_builtins)
    }

    fn builtin_components_mut(&mut self) -> impl Iterator<Item = &mut BuiltinComponent> {
        self.parameters
            .iter_mut()
            .flat_map(Parameter::as_builtins_mut)
    }

    fn custom_components(&self) -> impl Iterator<Item = &Component> {
        self.parameters
            .iter()
            .filter_map(|param| param.as_components())
            .flat_map(|list| list.values.iter())
    }

    fn custom_components_mut(&mut self) -> impl Iterator<Item = &mut Component> {
        self.parameters
            .iter_mut()
            .filter_map(|param| param.as_components_mut())
            .flat_map(|list| list.values.iter_mut())
    }

    fn components(&self) -> impl Iterator<Item = ComponentRef> {
        (self
            .builtin_components()
            .map(|builtin| builtin.as_component_ref()))
        .chain(
            self.custom_components()
                .map(|component| component.reference()),
        )
    }

    pub fn expand(self, ident: Ident, generics: Generics) -> syn::Result<proc_macro2::TokenStream> {
        let intermediate = Intermediate::new(ident, generics, self);
        intermediate.expand_simple()
    }
}

impl Parse for Listener {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let id: Comma<NamespacedId> = input.parse()?;
        let parameters = id
            .continue_with(input, |input| {
                input.parse_terminated(Parameter::parse, Token![,])
            })?
            .unwrap_or_default();
        Ok(Self { id, parameters })
    }
}

impl ToTokens for Listener {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.id.to_tokens_with(tokens, |tokens| {
            self.id.value.to_lit_str().to_tokens(tokens);
        });
        self.parameters.to_tokens(tokens);
    }
}

#[derive(Debug)]
enum Parameter {
    Listen(List<BuiltinComponent>),
    Take(List<BuiltinComponent>),
    OnLoad(List<BuiltinComponent>),
    RegistryComponent(NameValue<LitBool>),
    Components(List<Component>),
    RespondFlag(Ident),
    Respond(Ident, Parenthesized<super::RespondMeta>),
    ListenToRegistry(Box<BuiltinComponent>),
}

impl Parameter {
    const fn as_registry_component(&self) -> Option<&NameValue<LitBool>> {
        if let Self::RegistryComponent(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_components(&self) -> Option<&List<Component>> {
        if let Self::Components(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_components_mut(&mut self) -> Option<&mut List<Component>> {
        if let Self::Components(v) = self {
            Some(v)
        } else {
            None
        }
    }

    fn as_builtins(&self) -> impl Iterator<Item = &BuiltinComponent> {
        let (list, single) = match self {
            Self::Listen(list) | Self::Take(list) | Self::OnLoad(list) => (Some(list.iter()), None),
            Self::ListenToRegistry(single) => (None, Some(single)),
            _ => (None, None),
        };
        (list.into_iter().flatten()).chain(single.into_iter().map(Deref::deref))
    }

    fn as_builtins_mut(&mut self) -> impl Iterator<Item = &mut BuiltinComponent> {
        let (list, single) = match self {
            Self::Listen(list) | Self::Take(list) | Self::OnLoad(list) => {
                (Some(list.iter_mut()), None)
            }
            Self::ListenToRegistry(single) => (None, Some(single)),
            _ => (None, None),
        };
        (list.into_iter().flatten()).chain(single.into_iter().map(DerefMut::deref_mut))
    }
}

impl Parse for Parameter {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "listen" => Ok(Self::Listen(List::parse_after_with(
                ident,
                input,
                |input| BuiltinComponent::parse(input, BuiltinComponentType::Listen),
            )?)),
            "take" => Ok(Self::Take(List::parse_after_with(ident, input, |input| {
                BuiltinComponent::parse(input, BuiltinComponentType::Take)
            })?)),
            "on_load" => Ok(Self::OnLoad(List::parse_after_with(ident, input, |input| {
                BuiltinComponent::parse(input, BuiltinComponentType::OnLoad)
            })?)),
            "registry_component" => Ok(Self::RegistryComponent(NameValue::parse_after(
                ident, input,
            )?)),
            "components" => Ok(Self::Components(List::parse_after(ident, input)?)),
            "respond" => {
                if input.peek(syn::token::Paren) {
                    input
                        .parse::<Parenthesized<super::RespondMeta>>()
                        .map(|parenthesized| Self::Respond(ident, parenthesized))
                } else {
                    Ok(Self::RespondFlag(ident))
                }
            }
            "listen_to_registry" => BuiltinComponent::parse_after_ident(
                type_from_ident(ident),
                input,
                BuiltinComponentType::ListenToRegistry,
            )
            .map(Box::new)
            .map(Self::ListenToRegistry),
            "id" => Err(syn::Error::new_spanned(
                ident,
                "'id' is only accepted as the first parameter",
            )),
            _ => Err(syn::Error::new_spanned(
                ident,
                "expected one of 'listen', 'take', 'respond', 'on_load', 'registry_component', 'listen_to_registry', or 'components'",
            )),
        }
    }
}

impl ToTokens for Parameter {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Listen(list) | Self::Take(list) | Self::OnLoad(list) => list.to_tokens(tokens),
            Self::RegistryComponent(name_value) => name_value.to_tokens(tokens),
            Self::Components(list) => list.to_tokens(tokens),
            Self::RespondFlag(ident) => ident.to_tokens(tokens),
            Self::Respond(ident, parenthesized) => {
                ident.to_tokens(tokens);
                parenthesized.to_tokens(tokens);
            }
            Self::ListenToRegistry(component) => {
                component.to_tokens(tokens);
            }
        }
    }
}

#[derive(Debug)]
struct BuiltinComponent {
    implementation_type: Type,
    component_type: Type,
    message_type: Type,
    opts: Option<BuiltinComponentOpts>,
}

#[derive(Clone, Copy)]
enum BuiltinComponentType {
    Listen,
    Take,
    OnLoad,
    ListenToRegistry,
}

impl BuiltinComponent {
    fn new(message: Type, opts: Option<BuiltinComponentOpts>, typ: BuiltinComponentType) -> Self {
        let auditeur = &*crate::AUDITEUR;
        let (implementation_type, component_type) = match typ {
            BuiltinComponentType::Listen => (
                parse_quote!( Listen<#message> ),
                parse_quote!( #auditeur::resource::listener::ListenComponent<Self, #message> ),
            ),
            BuiltinComponentType::Take => (
                parse_quote!( Take<#message> ),
                parse_quote!( #auditeur::resource::listener::TakeComponent<Self, #message> ),
            ),
            BuiltinComponentType::OnLoad => (
                parse_quote!( OnLoad<#message> ),
                parse_quote!( #auditeur::resource::listener::OnLoadComponent<Self, #message> ),
            ),
            BuiltinComponentType::ListenToRegistry => (
                parse_quote!(ListenToRegistry),
                parse_quote!( #auditeur::resource::listener::ListenToRegistryComponent<Self> ),
            ),
        };
        Self {
            implementation_type,
            component_type,
            message_type: message,
            opts,
        }
    }

    fn parse(input: syn::parse::ParseStream, typ: BuiltinComponentType) -> syn::Result<Self> {
        let message_type = input.parse()?;
        Self::parse_after_ident(message_type, input, typ)
    }

    fn parse_after_ident(
        message_type: Type,
        input: syn::parse::ParseStream,
        typ: BuiltinComponentType,
    ) -> syn::Result<Self> {
        let opts = input.peek(Paren).then(|| input.parse()).transpose()?;
        Ok(Self::new(message_type, opts, typ))
    }

    fn as_component_ref(&self) -> ComponentRef {
        ComponentRef {
            component: &self.component_type,
            specifier: self.opts.as_ref().map_or_else(
                || SpecifierRef::Implementation(&self.implementation_type),
                |opts| SpecifierRef::Input(&opts.input.value.value),
            ),
        }
    }

    const fn has_input(&self) -> bool {
        self.opts.is_some()
    }

    fn set_input(&mut self, input: InputList) {
        self.opts = Some(BuiltinComponentOpts {
            paren: Paren(input.span()),
            input: Comma::new(NameValue::new(Ident::new("input", input.span()), input)),
        });
    }
}

impl ToTokens for BuiltinComponent {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.message_type.to_tokens(tokens);
        self.opts.to_tokens(tokens);
    }
}

#[derive(Debug)]
struct BuiltinComponentOpts {
    paren: Paren,
    input: Comma<NameValue<InputList>>,
}

impl Parse for BuiltinComponentOpts {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        Ok(Self {
            paren: parenthesized!(content in input),
            input: Comma::parse_with(&content, |content| NameValue::parse(content, "input"))?,
        })
    }
}

impl ToTokens for BuiltinComponentOpts {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.paren
            .surround(tokens, |tokens| self.input.to_tokens(tokens));
    }
}

#[derive(Debug)]
#[expect(clippy::struct_field_names)]
struct Component {
    paren: Paren,
    component: Comma<NameValue<Type>>,
    specifier: Comma<Specifier>,
}

impl Component {
    const fn reference(&self) -> ComponentRef {
        ComponentRef {
            component: &self.component.value.value,
            specifier: self.specifier.value.reference(),
        }
    }

    const fn has_input(&self) -> bool {
        self.specifier.value.is_input()
    }

    fn set_input(&mut self, inputs: InputList) {
        self.specifier.value =
            Specifier::Input(NameValue::new(Ident::new("input", inputs.span()), inputs));
    }
}

impl Parse for Component {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        let paren = parenthesized!(content in input);

        let component = Comma::parse_with(&content, |input| NameValue::parse(input, "component"))?;
        let specifier = Specifier::peek(&content)
            .then(|| component.continue_with(&content, Comma::<Specifier>::parse))
            .transpose()?
            .flatten()
            .unwrap_or_default();

        if Specifier::peek(&content) {
            return if component.has_comma() {
                Err(content.error("only one specifier allowed - prioritize input"))
            } else {
                Err(content.error("expected comma"))
            };
        }

        Ok(Self {
            paren,
            component,
            specifier,
        })
    }
}

impl ToTokens for Component {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.paren.surround(tokens, |tokens| {
            self.component.to_tokens(tokens);
            self.specifier.to_tokens(tokens);
        });
    }
}

#[derive(Clone, Copy)]
struct ComponentRef<'a, S: Copy = SpecifierRef<'a>> {
    component: &'a Type,
    specifier: S,
}

impl<'a> ComponentRef<'a> {
    const fn as_input(self) -> Result<ComponentRef<'a, &'a InputList>, Self> {
        match self.specifier {
            SpecifierRef::Input(list) => Ok(ComponentRef {
                component: self.component,
                specifier: list,
            }),
            _ => Err(self),
        }
    }

    fn macro_ident(&self) -> Ident {
        let specifier_macro = self.component.to_token_stream().to_string();
        let mut specifier_macro = if let Some((begin, _)) = specifier_macro.split_once('<') {
            begin.to_string()
        } else {
            specifier_macro
        };
        specifier_macro.retain(|character| !character.is_whitespace());

        let span = self.component.span();
        Ident::new(&specifier_macro, span)
    }
}

#[derive(Default, Debug)]
pub enum Specifier {
    #[default]
    None,
    Implementation(NameValue<Type>),
    Input(NameValue<InputList>),
}

impl Specifier {
    fn peek(input: syn::parse::ParseStream) -> bool {
        input.peek(syn::Ident)
    }

    const fn reference(&self) -> SpecifierRef<'_> {
        match self {
            Self::None => SpecifierRef::None,
            Self::Implementation(name_value) => SpecifierRef::Implementation(&name_value.value),
            Self::Input(name_value) => SpecifierRef::Input(&name_value.value),
        }
    }

    /// Returns `true` if the specifier is [`Input`].
    ///
    /// [`Input`]: Specifier::Input
    #[must_use]
    pub const fn is_input(&self) -> bool {
        matches!(self, Self::Input(..))
    }
}

impl Parse for Specifier {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if !input.peek(syn::Ident) {
            return Ok(Self::None);
        }

        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "implementation" => Ok(Self::Implementation(NameValue::parse_after(ident, input)?)),
            "input" => Ok(Self::Input(NameValue::parse_after(ident, input)?)),
            _ => Err(syn::Error::new_spanned(
                ident,
                "expected one of 'implementation' or 'input'",
            )),
        }
    }
}

impl ToTokens for Specifier {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::None => (),
            Self::Implementation(name_value) => name_value.to_tokens(tokens),
            Self::Input(name_value) => name_value.to_tokens(tokens),
        }
    }
}

#[derive(Clone, Copy)]
enum SpecifierRef<'a> {
    None,
    Implementation(&'a Type),
    Input(&'a InputList),
}

pub struct ComponentCallback {
    in_: Parenthesized<TokenStream>,
    _arrow: Token![=>],
    out: Parenthesized<TokenStream>,
}

impl ComponentCallback {
    pub fn expand(self, ident: &Ident) -> TokenStream {
        let Self { in_, out, .. } = self;
        let in_ = in_.value;
        let ident_mapped = Ident::new(&format!("auditeur_{ident}_macro"), ident.span());
        // NOTE: $component is taken in as a parameter (and ignored) because of `use _ as _` bindings - we
        // don't know for sure that it'll begin with #ident.
        quote! {
            #[macro_export] #[doc(hidden)]
            macro_rules! #ident_mapped {
                (($callback:path $(, $($callbacks:tt)*)?) ($component:tt #in_ $($params:tt)*) $($rest:tt)*) => {
                    $callback!(($($($callbacks)*)?) (#out $($params)*) $($rest)*);
                };
            }
            #[doc(hidden)] pub use #ident_mapped as #ident;
        }
    }
}

impl Parse for ComponentCallback {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Self {
            in_: input.parse()?,
            _arrow: input.parse()?,
            out: input.parse()?,
        })
    }
}
