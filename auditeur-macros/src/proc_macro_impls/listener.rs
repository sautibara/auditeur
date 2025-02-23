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

use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parenthesized, parse::Parse, parse_quote, punctuated::Punctuated, spanned::Spanned,
    token::Paren, Ident, LitBool, Token, Type,
};

use crate::{
    custom_generics::CustomGenerics,
    field::{Comma, List, NameValue, Parenthesized},
    input_list::{Combination, InputList},
    namespaced_id::NamespacedId,
    Auditeur,
};

use super::input_reflection::{CallbackList, HasInputArgs};

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

pub fn runnable_derive(implementer: &Type, args: &HasInputArgs) -> syn::Result<TokenStream> {
    if args.owned() {
        return Err(syn::Error::new_spanned(args, "'owned' is not allowed here"));
    }

    let no_input = InputList::default();

    let runnable_trait = crate::type_from_ident(Ident::new("Runnable", Span::call_site()));
    let init_input_macro_ident =
        super::input_reflection::input_macro_of_method(&runnable_trait, implementer, "init")?;
    let exit_input_macro_ident =
        super::input_reflection::input_macro_of_method(&runnable_trait, implementer, "exit")?;

    let init_input_macro = super::export_input(&init_input_macro_ident, &no_input, args);
    let exit_input_macro = super::export_input(&exit_input_macro_ident, &no_input, args);

    let resource = crate::AUDITEUR.resource();
    let anyhow = crate::AUDITEUR.anyhow();

    Ok(quote! {
        #init_input_macro
        #exit_input_macro
        impl #resource::listener::Runnable for #implementer {
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
    init_input: Option<InputList>,
    exit_input: Option<InputList>,
    input_lists: Vec<InputList>,
}

impl Intermediate {
    fn new(implementer: Ident, listener: Listener) -> Self {
        Self {
            callbacks: Parenthesized::default(),
            params: Parenthesized::default(),
            implementer,
            listener: Parenthesized::default_with(listener),
            init_input: None,
            exit_input: None,
            input_lists: Vec::new(),
        }
    }

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

    fn not_enough_inputs_error(&self) -> syn::Result<()> {
        Err(syn::Error::new_spanned(
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
        ))
    }

    fn apply_input(&mut self) -> syn::Result<()> {
        let listen_components_len = self
            .listener
            .value
            .listen_components()
            .filter(|component| !component.has_input())
            .count();
        if listen_components_len > self.input_lists.len() {
            self.not_enough_inputs_error()?;
        }
        let listen_components = self
            .listener
            .value
            .listen_components_mut()
            .filter(|component| !component.has_input());
        let inputs = self.input_lists.drain(..listen_components_len);

        for (listen_component, input) in listen_components.zip(inputs) {
            listen_component.set_input(input);
        }

        let custom_components_len = self
            .listener
            .value
            .custom_components()
            .filter(|component| !component.has_input())
            .count();
        if custom_components_len > self.input_lists.len() {
            self.not_enough_inputs_error()?;
        }
        let custom_components = self
            .listener
            .value
            .custom_components_mut()
            .filter(|component| !component.has_input());
        let inputs = self.input_lists.drain(..custom_components_len);

        for (custom_component, input) in custom_components.zip(inputs) {
            custom_component.set_input(input);
        }
        Ok(())
    }

    pub fn expand(mut self) -> syn::Result<proc_macro2::TokenStream> {
        self.apply_input()?;
        self.expand_simple()
    }

    fn expand_simple(mut self) -> syn::Result<proc_macro2::TokenStream> {
        let (with_input, without_input): (Vec<_>, Vec<_>) =
            split_results(self.listener.value.components().map(ComponentRef::as_input));

        if let (Some(init_input), Some(exit_input), true) =
            (&self.init_input, &self.exit_input, without_input.is_empty())
        {
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

            let resource_impl = super::resource_impls_with_generics(
                &self.implementer,
                &self.listener.value.id.value,
                &CustomGenerics::default(),
                &auditeur,
            );

            Ok(quote! {
                #resource_impl

                impl #resource_mod::listener::Listener for #implementer {
                    type Input = #inputs;
                    type Components = (#(#components),*);
                    type ComponentCombinations = (#combinations);
                    type InitCombination = #init_combination;
                    type ExitCombination = #exit_combination;
                }
            })
        } else {
            // we have to fill the rest of the inputs :(

            let auditeur = Auditeur::new();
            let crate_name = auditeur.crate_name();

            // use closures here to get around a lack of view types
            let mut add_callback = |path| self.callbacks.value.list.push(path);
            let mut add_specifier_param = |typ| self.params.value.push(typ);

            let implementer_type = crate::type_from_ident(self.implementer.clone());

            let runnable = parse_quote!(Runnable);
            add_callback(
                super::input_reflection::input_macro_of_method(
                    &runnable,
                    &implementer_type,
                    "init",
                )?
                .into(),
            );
            add_callback(
                super::input_reflection::input_macro_of_method(
                    &runnable,
                    &implementer_type,
                    "exit",
                )?
                .into(),
            );

            for component in without_input {
                match component.specifier {
                    SpecifierRef::None => {
                        add_callback(component.macro_ident().into());
                        add_specifier_param(component.component.clone());
                        add_callback(parse_quote!( #crate_name::expand_input_specifier ));
                    }
                    SpecifierRef::Implementation(typ) => {
                        let input_macro =
                            super::input_reflection::input_macro_of(typ, &implementer_type)?;
                        add_callback(input_macro.into());
                    }
                    SpecifierRef::Input(_) => unreachable!(),
                }
            }

            add_callback(parse_quote!( #crate_name::expand_listener ));

            Ok(self
                .into_callback()
                .expect("there should be at least one callback"))
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
            .last()
            .map_or(true, |boolean| boolean.value.value)
    }

    fn listen_components(&self) -> impl Iterator<Item = &BuiltinComponent> {
        self.parameters
            .iter()
            .filter_map(|param| param.as_listen())
            .flat_map(|builtins| builtins.values.iter())
    }

    fn listen_components_mut(&mut self) -> impl Iterator<Item = &mut BuiltinComponent> {
        self.parameters
            .iter_mut()
            .filter_map(|param| param.as_listen_mut())
            .flat_map(|builtins| builtins.values.iter_mut())
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
            .listen_components()
            .map(|builtin| builtin.as_component_ref()))
        .chain(
            self.custom_components()
                .map(|component| component.reference()),
        )
    }

    pub fn expand(self, ident: Ident) -> syn::Result<proc_macro2::TokenStream> {
        let intermediate = Intermediate::new(ident, self);
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
    RegistryComponent(NameValue<LitBool>),
    Components(List<Component>),
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

    const fn as_listen(&self) -> Option<&List<BuiltinComponent>> {
        if let Self::Listen(v) = self {
            Some(v)
        } else {
            None
        }
    }

    const fn as_listen_mut(&mut self) -> Option<&mut List<BuiltinComponent>> {
        if let Self::Listen(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl Parse for Parameter {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        match &*ident.to_string() {
            "listen" => Ok(Self::Listen(List::parse_after_with(
                ident,
                input,
                |input| {
                    let resource_mod = Auditeur::new().resource();
                    BuiltinComponent::parse(
                        input,
                        |message| parse_quote!( Listen<#message> ),
                        |message| parse_quote!( #resource_mod::listener::ListenComponent<Self, #message> ),
                    )
                },
            )?)),
            "registry_component" => Ok(Self::RegistryComponent(NameValue::parse_after(
                ident, input,
            )?)),
            "components" => Ok(Self::Components(List::parse_after(ident, input)?)),
            "id" => Err(syn::Error::new_spanned(
                ident,
                "'id' is only accepted as the first parameter",
            )),
            _ => Err(syn::Error::new_spanned(
                ident,
                "expected one of 'listen', 'registry_component' or 'components'",
            )),
        }
    }
}

impl ToTokens for Parameter {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Listen(list) => list.to_tokens(tokens),
            Self::RegistryComponent(name_value) => name_value.to_tokens(tokens),
            Self::Components(list) => list.to_tokens(tokens),
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

impl BuiltinComponent {
    fn parse(
        input: syn::parse::ParseStream,
        implementation: impl FnOnce(&Type) -> Type,
        component: impl FnOnce(&Type) -> Type,
    ) -> syn::Result<Self> {
        let message_type = input.parse()?;
        Ok(Self {
            implementation_type: implementation(&message_type),
            component_type: component(&message_type),
            message_type,
            opts: input.peek(Paren).then(|| input.parse()).transpose()?,
        })
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
