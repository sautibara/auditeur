//! A separate module for imported macros just to clean up `lib.rs`

#[allow(unused_imports)] // used for doc links
use crate::prelude::*;
#[allow(unused_imports, clippy::wildcard_imports)] // used for doc links
use crate::resource::listener::*;

/// Returns a parsed static reference to a [`NamespacedIdRef`].
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::namespaced_id;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// assert_eq!(
///     ident!("namespace:id"),
///     &NamespacedId::new("namespace", "id"),
/// );
/// # }
/// ```
pub use auditeur_macros::ident;

/// Implements [`Resource`] for any type.
///
/// This automatically implements [`Identifiable`], [`Any`], and if necessary, [`MaybeGetRef`].
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::{namespaced_id, resource, path_any, input};
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[resource("test:useful_resource")]
/// struct UsefulResource;
///
/// fn assert_is_resource<R: auditeur::resource::Resource>(_: R) {}
///
/// assert_eq!(UsefulResource.ident(), ident!("test:useful_resource"));
/// assert_is_resource(UsefulResource);
/// # }
/// ```
pub use auditeur_macros::resource;

/// Implements [`Identifiable`] for any type.
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::{namespaced_id, resource, path_any, input};
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[identifiable("test:has_identifier")]
/// struct HasIdentifier;
///
/// fn assert_is_identifiable<I: auditeur::resource::Identifiable>(_: I) {}
///
/// assert_eq!(HasIdentifier.ident(), ident!("test:has_identifier"));
/// assert_is_identifiable(HasIdentifier);
/// # }
/// ```
pub use auditeur_macros::identifiable;

/// Implements [`Message`] for any type.
///
/// This automatically implements [`Identifiable`] and [`Any`].
///
/// Note that you will have to annotate [`#[derive(Clone)]`](macro@Clone) yourself to make it more
/// obvious that a [`Clone`](trait@Clone) implementation is required.
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::{namespaced_id, resource, path_any, input, Message};
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[derive(Clone)]
/// #[message("test:message")]
/// struct Message;
///
/// fn assert_is_message<M: auditeur::Message>(_: M) {}
///
/// assert_eq!(Message.ident(), ident!("test:message"));
/// assert_is_message(Message);
/// # }
/// ```
pub use auditeur_macros::message;

// these two are used in [`macro@listener`]
#[doc(hidden)]
pub use auditeur_macros::expand_input_specifier;
#[doc(hidden)]
pub use auditeur_macros::expand_listener;

/// Implements [`Listener`] for any type.
///
/// Like [`#[resource]`](resource), the first argument in the attribute must be a [`NamespacedId`]
/// string literal (see [`ident!`]). Using this, the macro implements everything that
/// [`#[resource]`](resource) does, making the item a valid [`Resource`]. After, arguments can be
/// inserted in any order to give the [`Listener`] implementation
/// [`Component`](crate::resource::listener::Component)s. Each component added automatically pulls
/// inputs from [`has_input`] - unless the inputs are manually specified.
///
/// # Examples
///
/// ```no_run
/// # use auditeur::*;
/// # use auditeur::resource::listener::{Listener, Listen};
/// use auditeur::prelude::*;
/// use auditeur::resource::listener::{ListenComponent, RegistryComponent};
///
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::*;
///
/// // This implements [`Listener`] by listening to both [`Message1`] and [`Message2`].
/// // The inputs for these blocks are collected by the attribute and combined into a single tuple,
/// //     so that the two components can share them :).
/// #[derive(Runnable)]
/// #[listener("test:double_listener", listen(Message1, Message2))]
/// struct DoubleListener;
/// #
/// # fn main() {}
/// ```
///
/// <details>
///   <summary>Shared code for all examples:</summary>
///
/// ```no_run
/// # #[listener("test:listener")]
/// # #[derive(Runnable)]
/// # struct DoubleListener;
/// # use auditeur::*;
/// mod shared {
///     use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
///     use super::DoubleListener;
///
///     #[derive(Clone)]
///     #[message("test:message1")]
///     pub struct Message1;
///
///     #[derive(Clone)]
///     #[message("test:message2")]
///     pub struct Message2;
///
/// # /*
///     // (code omitted for brevity - they both just return `Ok(())`)
///     #[has_input] impl Listen<Message1> for DoubleListener { ... }
///     #[has_input] impl Listen<Message2> for DoubleListener { ... }
/// # */
/// # #[has_input] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// }
/// # fn main() {}
/// ```
///
/// </details>
///
/// # Parameters
///
/// ## Listen
///
/// To listen to messages using [`Listen`](crate::resource::listener::Listen), provide the list of
/// them wrapped by `listen(<message>,*)`. To manually provide inputs for a message, use
/// `listen(<message>(input = <inputs>))`.
///
/// ### Examples
///
/// ```no_run
/// # use auditeur::*;
/// # use auditeur::resource::listener::{Listener, Listen};
/// use auditeur::prelude::*;
/// use auditeur::resource::listener::{ListenComponent, RegistryComponent};
///
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::*;
///
///
/// #[listener("test:double_listener", listen(
///     // You can pull the inputs from a `#[has_input]` block,
///     Message1,
///     // or specify the inputs directly (which just happen to be empty).
///     Message2(input = ()),
/// ))]
/// #[derive(Runnable)]
/// struct DoubleListener;
///
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            RegistryComponent,
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
/// ```
///
/// ## Respond
///
/// Much like [`Listen`], to respond to messages with [`Respond`], provide the list of
/// them wrapped by `respond(<message>,*)`. To manually provide inputs for a message, use
/// `respond(<message>(input = <inputs>))`.
///
/// ### Examples
///
/// See [`derive(Response)`](macro@Response)
///
/// ## Components
///
/// To use other [`Component`]s, use a `components` list. Each component is surrounded in
/// parenthesees and at least specifies `component = <component_type>` - the type of the component
/// including its generics. After, it optionally follows with a [specifier](#specifier). If a
/// specifier isn't given, it queries the component for the specifier (see [`#[component]`]).
///
/// ### Specifier
///
/// A specifier gives more clarity to where to find the inputs of a component. It could either give
/// a trait that the component wraps (like [`Listen`] for [`Listener`]) with `implementation =
/// <trait>`, or the inputs themselves with `input = <inputs>`.
///
/// ### Examples
///
/// ```no_run
/// # use auditeur::*;
/// # use auditeur::resource::listener::{Listener, Listen};
/// use auditeur::prelude::*;
/// use auditeur::resource::listener::{ListenComponent, RegistryComponent};
///
///
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::*;
///
/// #[listener("test:double_listener", components(
///     // (
///     //      // You can pull a default specifier from the component itself.
///     //      component = ListenComponent<Self, Message1>
///     // ),
///     // // (This is only commented off since we can't specify this component twice)
///     (
///         component = ListenComponent<Self, Message1>,
///         // You can pull the inputs from a `#[has_input]` block implementing the
///         //   `implementation` trait.
///         // For example, this is the default specifier for ListenComponent.
///         implementation = Listen<Message1>,
///     ),
///     (
///         component = ListenComponent<Self, Message2>,
///         // Or you can also specify the inputs directly.
///         input = (),
///     ),
/// ))]
/// #[derive(Runnable)]
/// struct DoubleListener;
///
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            RegistryComponent,
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
/// ```
///
/// ## Registry Component
///
/// All listeners by default have a [`RegistryComponent`], which automatically handles exiting when
/// a [`Stopper`] tells it to. If the `runtime-loading` feature is enabled, it also fulfils
/// [`GetRequest`](crate::app::GetRequest)s. As such, only disable it if there is already
/// another component that handles these two things.
///
/// To disable adding a [`RegistryComponent`], specify `registry_component = false`.
///
/// ### Examples
///
/// ```no_run
/// # use auditeur::*;
/// # use auditeur::resource::listener::{Listener, Listen};
/// use auditeur::prelude::*;
/// use auditeur::resource::listener::{ListenComponent, RegistryComponent};
///
///
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::*;
///
/// #[listener(
///     "test:double_listener",
///     registry_component = false,
///     listen(Message1, Message2)
/// )]
/// #[derive(Runnable)]
/// struct DoubleListener;
///
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            // notice the lack of RegistryComponent
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
/// ```
///
/// # Errors
///
/// Most errors are caught directly by the macro and have dedicated error messages, but some
/// errors can arise from missing macro callbacks. That is, when the macro gets inputs defined in
/// [`#[has_input]`](has_input) blocks or [`#[component]`](component) specifiers.
///
/// - "use of undeclared crate or module `auditeur_input_of_impl_<trait>_for_<type>`"
///     - This macro was trying to get the input from a [`#[has_input]`](has_input) block, but it
///       couldn't find it. This could be caused by the necessary trait not being implemented, a
///       missing [`#[has_input]`](has_input) annotation, or the trait being implemented in another
///       crate. To fix this, make sure that the trait is implemented with a
///       [`#[has_input]`](has_input) annotation or specify the input with `input = <input>`. Note
///       that, in the string representation of the trait and type, all instances of `<` are
///       replaced with `_lt_`, `>` with `_gt_`, `::` with `_sep_`, and `_` with `__`.
/// - "use of undeclared crate or module `auditeur_input_of_<method>_in_impl_Runnable_for_<type>`"
///     - The macro couldn't find the input for the listener's [`Runnable`] implementation. See
///       above for possible fixes.
/// - "cannot find macro `<component_type>` in this scope"
///     - The macro was trying to get a specifier from a [`#[component]`](component) attribute, but
///       it couldn't find it. To fix this, try specifying the input directly, or if you defined the
///       component, make sure the attribute is attached.
///
/// # Syntax
///
/// <details>
///   <summary>Detailed Syntax</summary>
///
/// ```text
/// <attribute>:
///   | #[listener(id = "<namespace>:<id>", <parameter>,*)]
///
/// <parameter>:
///   | registry_component = <boolean>
///   | listen(<message>,*)
///   | components(<component>,*)
///
/// <message>:
///   | <type>
///   | <type>(input = <input>)
///
/// <component>:
///   | (component = <component_type>)
///   | (component = <component_type>, <specifier>)
///
/// <specifier>:
///   | /* an empty specifier is accepted, in which case it will pull the from the component */
///   | implementation = <trait>
///   | input = <input>
/// ```
///
/// </details>
pub use auditeur_macros::listener;

/// Allows specifying a default [specifier](listener#specifier) for a
/// [`listener::Component`](crate::resource::listener::Component).
///
/// The arguments to the attribute must be in the form of `(<generic_arguments>) => (<specifier>)`,
/// where `<generic_arguments>` are the generic arguments for the component, and `<specifier>` is
/// a [specifier](listener#specifier). This is then compiled into a declaritve macro with the same
/// identifier as the component, so all macro metavariables work.
///
/// # Examples
///
/// ```no_run
/// use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// use auditeur::ListenMessage;
///
/// // If a specifier isn't given, the default is `input = ()`.
/// // That is, this expects no generic arguments and outputs a specifier with no inputs.
/// #[component(() => ())]
/// pub struct RegistryComponent {
/// # /*
///     ...
/// # */
/// }
///
/// // Since the message type is given as the second generic argument, we delegate to the
/// //   implementation of the `Listen` trait for that message.
/// // More specifically, this expects two generic arguments, ignores the first one, and takes the
/// //   second one as a metavariable. Then, it outputs a specifier delegating to the
/// //   implementation of the `Listen` trait with the metavariable as a generic argument.
/// #[component((<$_:ty, $message:ty>) => (implementation = Listen<$message>))]
/// pub struct ListenComponent<L: Listen<M>, M: ListenMessage> {
/// # /*
///     ...
/// # */
/// # phantom_data: std::marker::PhantomData<(L, M)>,
/// }
///
/// # fn main() {
/// # /*
/// // NOTE: these can't actually be run because callback macros can't be used in this context
/// //       just trust...
/// assert_eq!(
///     "() ((implementation = Listen<Message>))",
///     ListenComponent!((stringify) (ListenComponent<Struct, Message>)),
/// );
///
/// assert_eq!(
///     "() (())",
///     RegistryComponent!((stringify) (RegistryComponent)),
/// );
/// # */
/// # }
/// ```
pub use auditeur_macros::component;

/// Used on implementation blocks with [`Input`]s.
///
/// This macro attaches to `impl` blocks, collects the inputs for each method into associated
/// types so that traits can see them, and, most importantly, exports the inputs for macros like
/// [`#[listener]`](listener).
///
/// More specifically, each method's last argument is captured as that method's [`Input`]s. Then,
/// the input list gets added as an associated type with the identifier of the method's identifier
/// suffixed by the word `Input` (for example, the associated type for a method named `on` will be
/// named `OnInput`). Traits that take inputs are expected to have their associated types titled in
/// this manner.
///
/// # Examples
///
/// ```no_run
/// use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use auditeur::*;
/// use tokio::sync::broadcast::Sender;
///
/// trait TypeEqual<T> {}
/// impl<T> TypeEqual<T> for T {}
/// // This only allows compilation if the types T and U are equal
/// const fn assert_types_equal<T, U: TypeEqual<T>>() {}
///
/// #[derive(Clone)]
/// #[message("test:message")]
/// struct Message1;
///
/// #[derive(Runnable)]
/// #[listener("test:listener", listen(Message1))]
/// struct Listener;
///
///
/// #[has_input] // The attribute :)
/// impl Listen<Message1> for Listener {
///     async fn on(
///         &mut self,
///         // Since this is not the last argument, it is not seen as an input.
///         _: Message1,
///         // Instead, the last argument in the method is always treated as the input,
///         // so the input of this block becomes (&mut Stopper, &mut Sender<Message1>).
///         // (These are just some dummy inputs to explain how the attribute works).
///         (stopper, sender): (&mut Stopper, &mut Sender<Message1>)
///     ) -> Result<()> {
///         Ok(())
///     }
/// }
///
///
/// fn associated_type_test<'a>() {
///     // The attribute adds an associated type with the input of the method,
///     assert_types_equal::<
///         <Listener as Listen<Message1>>::OnInput<'a>,
///         (&'a mut Stopper, &'a mut Sender<Message1>)
///     >();
/// }
///
/// fn export_test() {
///     // and also exports the inputs for other macros, which are collected by `#[listener]`
///     assert_types_equal::<
///         <Listener as auditeur::resource::listener::Listener>::Input,
///         // All listener inputs must be prefixed by a unit tuple, but the rest is the same
///         ((), Stopper, Sender<Message1>)
///     >();
/// }
/// # fn main() {}
/// ```
pub use auditeur_macros::has_input;

/// Implements [`Runnable`](crate::resource::Runnable) through default implementations of `init`
/// and `exit`.
///
/// # Examples
///
/// These two are equivalent:
///
/// ```
/// use auditeur::prelude::*;
/// # use auditeur::*;
///
/// #[derive(Runnable)]
/// struct Runner;
///
/// fn assert_is_runnable<R: Runnable>() {}
/// # fn main() {
/// assert_is_runnable::<Runner>();
/// # }
/// ```
///
/// ```
/// use auditeur::prelude::*;
///
/// struct Runner;
///
/// #[has_input]
/// impl Runnable for Runner {
///     async fn init(&mut self, (): ()) -> Result<()> { Ok(()) }
///     async fn exit(&mut self, (): ()) -> Result<()> { Ok(()) }
/// }
///
/// fn assert_is_runnable<R: Runnable>() {}
/// # fn main() {
/// assert_is_runnable::<Runner>();
/// # }
/// ```
pub use auditeur_macros::Runnable;

pub use auditeur_macros::create;
pub use auditeur_macros::Create;

pub use auditeur_macros::Input;

/// Implements [`GetRef`](crate::input::get::GetRef) using [`Clone`].
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::{input, path_any};
/// # pub fn main() {
/// use auditeur::prelude::*;
/// use auditeur::input::get::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef, PathAny)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let reference: Shared = shared.get_ref();
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
pub use auditeur_macros::GetRef;

/// Automatically provides an implementation of [`MaybeGetRef`](crate::input::get::MaybeGetRef).
///
/// Usually, though, this is already implemented by [`resource`](macro@resource) or one of its
/// derivatives.
///
/// See [`MaybeGetRef`](crate::input::get::MaybeGetRef) for more information on why this is necessary
#[cfg(feature = "runtime-loading")]
pub use auditeur_macros::MaybeGetRef;

/// Automatically provides an implementation of [`PathAny`] by implementing
/// [`PathAnyConst`](crate::path_any::PathAnyConst)
#[cfg(feature = "path-any")]
pub use auditeur_macros::PathAny;
