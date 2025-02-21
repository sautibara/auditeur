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
/// This also automatically implements everything that [`resource`] does. To give the listener a
/// [`NamespacedId`], the first item in the macro's argument list must be in the form of `id =
/// "<namespace>:<id>"`. All other arguments can be inserted in any order.
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
/// # #[has_input(crate_root = true)] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input(crate_root = true)] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::{Message1, Message2};
///
/// #[derive(Runnable)]
/// #[listener(id = "test:double_listener", listen(Message1, Message2))]
/// # #[auditeur_runnable(crate_root = true)]
/// struct DoubleListener;
/// ```
///
/// <details>
///   <summary>Shared code for all examples:</summary>
///
/// ```no_run
/// # #[listener(id = "test:listener")]
/// # #[derive(Runnable)] #[auditeur_runnable(crate_root = true)]
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
/// # #[has_input(crate_root = true)] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input(crate_root = true)] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
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
/// # #[has_input(crate_root = true)] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input(crate_root = true)] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::{Message1, Message2};
///
/// #[listener(id = "test:double_listener", listen(
///     // pull the inputs from an impl block
///     Message1,
///     // specify the inputs directly
///     Message2(input = ()),
/// ))]
/// #[derive(Runnable)]
/// # #[auditeur_runnable(crate_root = true)]
/// struct DoubleListener;
///
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            RegistryComponent,
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
/// ```
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
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input(crate_root = true)] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input(crate_root = true)] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::{Message1, Message2};
///
/// #[listener(id = "test:double_listener", components(
///     // NOTE: (component = ListenComponent<Self, Message1>) would also work here
///     (
///         component = ListenComponent<Self, Message1>,
///         // pull the inputs from an impl block
///         implementation = Listen<Message1>,
///     ),
///     (
///         component = ListenComponent<Self, Message2>,
///         // specify the inputs directly
///         input = (),
///     ),
/// ))]
/// #[derive(Runnable)]
/// # #[auditeur_runnable(crate_root = true)]
/// struct DoubleListener;
///
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            RegistryComponent,
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
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
/// # mod shared {
/// # use auditeur::prelude::*;
/// # use auditeur::resource::listener::Listen;
/// # use super::DoubleListener;
/// # #[derive(Clone)] #[message("test:message1")] pub struct Message1;
/// # #[derive(Clone)] #[message("test:message2")] pub struct Message2;
/// # #[has_input(crate_root = true)] impl Listen<Message1> for DoubleListener { async fn on(&mut self, _: Message1, (): ()) -> Result<()> { Ok(()) } }
/// # #[has_input(crate_root = true)] impl Listen<Message2> for DoubleListener { async fn on(&mut self, _: Message2, (): ()) -> Result<()> { Ok(()) } }
/// # }
/// use shared::{Message1, Message2};
///
/// #[listener(
///     id = "test:double_listener",
///     registry_component = false,
///     listen(Message1, Message2)
/// )]
/// #[derive(Runnable)]
/// # #[auditeur_runnable(crate_root = true)]
/// struct DoubleListener;
///
/// fn assert_components_for<L>(_: L)
/// where
///     L: Listener<Components = (
///            (), // all components lists start with ()
///            // notice the lack of RegistryComponent
///            ListenComponent<L, Message1>,
///            ListenComponent<L, Message2>
///        )> + Listen<Message1> + Listen<Message2>
/// {}
///
/// # fn main() {
/// assert_components_for(DoubleListener);
/// # }
/// ```
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

pub use auditeur_macros::component;
pub use auditeur_macros::has_input;
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
