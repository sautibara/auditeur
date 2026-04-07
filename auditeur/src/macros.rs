//! A separate module for imported macros just to clean up `lib.rs`

#[allow(unused_imports)] // used for doc links
use crate::Message;
#[allow(unused_imports)] // used for doc links
use crate::prelude::*;
#[allow(unused_imports, clippy::wildcard_imports)] // used for doc links
use crate::resource::listener::*;

/// Implements [`Resource`] for any type.
///
/// This automatically implements [`Identifiable`], [`Any`](crate::Any), and if necessary,
/// [`MaybeGetRef`].
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::*;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[resource("test:useful_resource")]
/// struct UsefulResource;
///
/// fn assert_is_resource<R: auditeur::resource::Resource>(_: R) {}
///
/// assert_eq!(UsefulResource::id(), ident!("test:useful_resource"));
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
/// # use auditeur::*;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[identifiable("test:has_identifier")]
/// struct HasIdentifier;
///
/// fn assert_is_identifiable<I: auditeur::app::Identifiable>(_: I) {}
///
/// assert_eq!(HasIdentifier::id(), ident!("test:has_identifier"));
/// assert_is_identifiable(HasIdentifier);
/// # }
/// ```
pub use auditeur_macros::identifiable;

/// Implements [`Message`](crate::Message) for any type.
///
/// This automatically implements [`Identifiable`] and [`Any`](crate::Any).
///
/// Note that you will have to annotate [`#[derive(Clone)]`](macro@Clone) yourself to make it more
/// obvious that a [`Clone`](trait@Clone) implementation is required.
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::*;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[derive(Clone)]
/// #[message("test:message")]
/// struct Message;
///
/// fn assert_is_message<M: auditeur::Message>(_: M) {}
///
/// assert_eq!(Message::id(), ident!("test:message"));
/// assert_is_message(Message);
/// # }
/// ```
pub use auditeur_macros::message;

/// Implements [`Listener`] for any type.
///
/// # Parameters
///
/// ## `Listen`
///
/// To listen to messages using [`Listen`](crate::resource::listener::Listen), provide the list of
/// them wrapped by `listen(<message>,*)`.
///
/// ## `Take`
///
/// Much like [`Listen`], to take messages with [`Take`], provide the list of
/// them wrapped by `take(<message>,*)`.
///
/// ## Components
///
/// To use other [`Component`]s, provide a list of component types in a `components` list.
///
/// ## Registry Component
///
/// All listeners by default have a [`RegistryComponent`], which automatically handles exiting when
/// a [`Stopper`] tells it to. If the `runtime-loading` feature is enabled, it also fulfils
/// [`GetRequest`](crate::app::event::GetRequest)s. As such, only disable it if there is already
/// another component that handles these two things.
///
/// To disable adding a [`RegistryComponent`], specify `registry_component = false`.
///
/// ## Bias
///
/// All listeners by default try to poll their components fairly by randomly checking a component
/// to poll each time. However, if the `biased` parameter is used, the listener will check the
/// components in order instead. That is, the listener will always choose earlier components over
/// later components, if they have new things to do.
///
///
/// # Syntax
///
/// <details>
///   <summary>Detailed Syntax</summary>
///
/// ```text
/// <attribute>:
///   | #[listener(<parameter>,*)]
///
/// <parameter>:
///   | registry_component = <boolean>
///   | listen(<type>,*)
///   | take(<type>,*)
///   | components(<type>,*)
/// ```
///
/// </details>
pub use auditeur_macros::listener;

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
/// impl Runnable for Runner {
///     async fn init(&mut self) -> Result<()> { Ok(()) }
///     async fn exit(&mut self) -> Result<()> { Ok(()) }
/// }
///
/// fn assert_is_runnable<R: Runnable>() {}
/// # fn main() {
/// assert_is_runnable::<Runner>();
/// # }
/// ```
pub use auditeur_macros::Runnable;

// TODO: write documentation!
pub use auditeur_macros::create;

pub use auditeur_macros::Input;

/// Implements [`GetRef`](crate::input::get::GetRef) using [`Clone`].
pub use auditeur_macros::GetRef;

/// Automatically provides an implementation of [`MaybeGetRef`](crate::input::get::MaybeGetRef).
///
/// Usually, though, this is already implemented by [`resource`](macro@resource) or one of its
/// derivatives.
///
/// See [`MaybeGetRef`](crate::input::get::MaybeGetRef) for more information on why this is necessary
#[cfg(feature = "runtime-loading")]
pub use auditeur_macros::MaybeGetRef;

/// Automatically provides an implementation of [`CloneAs`](crate::input::CloneAs).
pub use auditeur_macros::CloneAs;

/// Automatically provides an implementation of [`PathAny`] by implementing
/// [`PathAnyConst`](crate::path_any::PathAnyConst)
#[cfg(feature = "path-any")]
pub use auditeur_macros::PathAny;

/// Allows [`Listener`] methods with return values to be called by other resources.
///
/// Other resources are given a `Connection` struct that can ask the listener (through a message)
/// to run a certain method. Then, the listener sends the response through a [oneshot
/// channel](tokio::sync::oneshot) contained in the message. This allows the methods to take a
/// mutable reference to the listener, take in [`Input`]s (which aren't reflected in the
/// connection's methods), and return values back to the resource.
///
/// # Examples
///
/// ```
/// use auditeur::prelude::*;
/// use tokio::sync::broadcast;
/// # use auditeur::{input, app, async_trait, anyhow, path_any, Message, resource, Error, tokio, try_join};
/// # use namespaced_id;
///
/// #[derive(Runnable)]
/// // All that needs to be done in the `#[listener]` macro is to pass in a `respond` flag.
/// // The macro then pulls the necessary information from the `impl` block for which messages
/// // to listen to.
/// #[create(default)]
/// #[resource("test:counter")]
/// #[listener(respond)]
/// struct Counter {
///     value: usize,
///     #[create(input)]
///     sender: Shouter<CounterChanged>,
/// }
///
/// // The macro takes the NamespacedId of the listener as the first argument to fill out the id
/// // for the `Query` message (which is automatically generated).
/// // The `get_ref` flag makes `Get<Counter>` return the created `CounterConnection` struct.
/// // The connection has methods that call the corresponding methods on the actual `Counter`.
/// #[respond("test:counter", get_ref)]
/// impl Counter {
///     // These methods can have return values, but note that neither the parameters or return
///     // types can hold references (since they're sent using messages). To get around this,
///     // use transactions, clone items, or use `Arc`s.
///     // Thankfully here we don't have to worry about that, since `usize` is copyable.
///     pub fn get(&self) -> usize {
///         self.value
///     }
///
///     // If methods are provided that return nothing, when they're called on the connection
///     // and awaited, the future will await until the listener is finished handling the method.
///     // This is used here to allow other resources to wait until the value is fully set.
///     pub fn set(&mut self, value: usize) {
///         self.value = value;
///         self.sender.send(CounterChanged);
///     }
/// }
///
/// #[derive(Clone)]
/// #[message("test:counter_changed")]
/// struct CounterChanged;
///
/// #[tokio::main]
/// async fn main() -> Result<(), auditeur::Error> {
///     let app = App::new().with_bundle(
///         new_bundle!("test:test")
///             .with_listener::<Counter>(())
///             .with_listen_channel::<CounterChanged>()
///     );
///     app.run_with(
///         |(counter, receiver): (
///             Get<Counter>,
///             auditeur::app::channels::Listener<CounterChanged>
///         ) | async move {
///             // All methods on a `Connection` struct take `&self` and are asynchronous since they
///             // send and receive messages.
///             assert!(receiver.is_empty());
///             assert_eq!(counter.get().await, 0);
///             counter.set(5).await;
///             assert_eq!(counter.get().await, 5);
///             assert!(!receiver.is_empty());
///         }
///     ).await
/// }
/// ```
///
/// # Transaction
///
/// If specified, `Connection`s can also accept transactions, which allow running multiple methods
/// at the same time on the listener.
///
/// Note that these aren't SQL type transactions - if they don't succeed, they will not
/// automatically be cancelled.
///
/// ## Examples
///
/// ```
/// use auditeur::prelude::*;
/// # use auditeur::{input, app, async_trait, anyhow, path_any, Message, resource, Error, tokio, try_join};
///
/// #[derive(Default, Runnable)]
/// #[create(default)]
/// #[resource("test:vec_resource")]
/// #[listener(respond)]
/// pub struct VecResource {
///     items: Vec<usize>,
/// }
///
/// #[respond("test:vec_resource", get_ref, transaction)]
/// impl VecResource {
///     pub fn push_usize(&mut self, value: usize) {
///         self.items.push(value);
///     }
///
///     pub fn get_unwrap(&self, index: usize) -> usize {
///         *self.items.get(index).unwrap()
///     }
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<(), auditeur::Error> {
///     let app = App::new().with_bundle(new_bundle!("test:test").with_listener::<VecResource>(()));
///     app.run_with(|vec: Get<VecResource>| async move {
///         // Use `spawn_transaction` to run a transaction without returning a value.
///         // While the future does have to be `await`ed, it will not await the listener fully
///         // running the future - use `run_transaction` for that.
///         vec.spawn_transaction(async |vec: &mut VecResource| {
///             for val in (0..5).rev() {
///                 // Note that transactions are run by the listener itself, so we don't have to
///                 // await any messages being sent.
///                 vec.push_usize(val);
///             }
///         }).await;
///
///         // Here, run_transaction is used to return a value: the first five elements in the vec.
///         let vals: Vec<usize> = vec.run_transaction(async |vec: &mut VecResource| {
///             (0..5).map(|index| vec.get_unwrap(index)).collect()
///         }).await;
///
///         assert_eq!(&[4, 3, 2, 1, 0], &*vals);
///     }).await
/// }
/// ```
///
/// # Method Attributes
///
/// ## `connection_fn`
///
/// Indicates that this method should be put directly in the `Connection` struct. Thus, the method
/// will only be able to call the other methods in `Connection`, but may use
/// things that the other methods may not accept - like generics.
///
// /// <div class="warning">
// /// This works with `ref_fn`, but is exclusive with `timeout` and `on_error`.
// /// </div>
// ///
/// ## `resource_fn`
///
/// Indicates that this method should be unchanged and put in the resource struct.
///
// /// <div class="warning">
// /// This works with `connection_fn`, but is exclusive with `timeout` and `on_error`.
// /// </div>
// ///
/// ### Examples
///
/// ```
/// use auditeur::prelude::*;
/// use auditeur::app::Identifiable;
/// # use auditeur::{input, app, async_trait, anyhow, path_any, Message, resource, Error, tokio, try_join};
///
/// #[derive(Default, Runnable)]
/// #[create(default)]
/// #[resource("test:attribute_example")]
/// #[listener(respond)]
/// pub struct IdentList {
///     idents: Vec<NamespacedId>,
/// }
///
/// #[respond("test:attribute_example", get_ref, transaction)]
/// impl IdentList {
///     pub fn push_ident(&mut self, ident: NamespacedId) {
///         self.idents.push(ident);
///     }
///
///     // These use generics, which other methods can't.
///     #[respond(resource_fn)]
///     pub fn push_ident_of<I: Identifiable>(&mut self) {
///         self.push_ident(I::id().to_owned())
///     }
///
///     #[respond(connection_fn)]
///     pub async fn push_ident_of<I: Identifiable>(&self) {
///         self.push_ident(I::id().to_owned()).await
///     }
///
///     pub fn index(&self, index: usize) -> NamespacedId {
///         self.idents[index].clone()
///     }
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<(), auditeur::Error> {
///     let app = App::new().with_bundle(new_bundle!("test:test").with_listener::<IdentList>(()));
///     app.run_with(|conn: Get<IdentList>| async move {
///         #[identifiable("test:identifiable1")]
///         struct TestIdentifiable1;
///         #[identifiable("test:identifiable2")]
///         struct TestIdentifiable2;
///
///         conn.push_ident(ident!("test:ident1").to_owned()).await;
///         conn.push_ident_of::<TestIdentifiable1>().await;
///         conn.run_transaction(async |reference: &mut IdentList| {
///             reference.push_ident_of::<TestIdentifiable2>();
///         }).await;
///
///         assert_eq!(*ident!("test:ident1"), conn.index(0).await);
///         assert_eq!(*ident!("test:identifiable1"), conn.index(1).await);
///         assert_eq!(*ident!("test:identifiable2"), conn.index(2).await);
///     }).await
/// }
/// ```
// /// ## `timeout = <expr>`
// ///
// /// Allows putting a timeout on the `Connection` side of this method, after which it will
// /// automatically error (see `on_error` below). The default is to not have a timeout.
// ///
// /// <div class="warning">
// /// This works with `on_error`, but is exclusive with `connection_fn` and `ref_fn`.
// /// </div>
// ///
// /// ## `on_error = <variant>`
// ///
// /// All methods in `Connection` may end up erroring if they time out (see `timeout` above) or the
// /// listener has unloaded (for example, if there was a panic or it willingly
// /// [stopped](crate::app::Stopper). This allows setting an action for when errors happen.
// ///
// /// <div class="warning">
// /// This works with `timeout`, but is exclusive with `connection_fn` and `ref_fn`.
// /// </div>
// ///
// /// ### Variants
// ///
// /// - `panic` - Panic when an error happens. This is the default.
// /// - `wrap_in_result` - Wrap the method in a [`Result`], with [`RespondError`](std::todo) as the
// ///   error variant.
// /// - `return_err` - Propagate the error (as a [`RespondError`](std::todo)) using the try operator
// ///   (`?`). This assumes the return type is already wrapped in a [`Result`].
// ///     - `return_err(type = <error>)` - Propagate the error as above, but use a different error
// ///       type for the `Connection` method than the other methods.
// /// - `wrap_in_option` - Wrap the method in an [`Option`], with [`None`] as the error.
// /// - `return_none` - Return [`None`] on error, assuming the return type is already wrapped in an
// ///   [`Option`].
// /// - `return = <expr>` - Return the value of `expr` when an error happens.
// ///
/// # Syntax
///
/// ```text
/// <RespondMeta>:
///   | <NamespacedId>, <Param>,*
///   | <Param>,*
///
/// <Param>:
///   | connection(<TypesParam>,*)
///   | query(<TypesParam>,*)
///   | fields = {<Field>,*}
///   | transaction
///   | get_ref
///
/// <TypesParam>:
///   | id = <NamespacedId>
///   | ident = <Ident>
///
/// <Field>:
///   | <Ident>: <Type>
/// ```
///
/// Components:
/// - `<NamespacedId>`: The base id to use for both `query` and `transaction`'s message ids.
/// - `query`
///     - `message_ident`: The message with variants for each method that could be called.
///         - Default: `<ident>Query`
///     - `message_id`: The identifier of that message.
///         - Default: `<namespace>:<id>_query`
///     - `ref_ident`: The `Connection` with methods that wrap sending and receiving messages.
///         - Default: `<ident>Connection`
/// - `transaction`
///     - `message_ident`: The message that contains a transaction closure.
///         - Default: `<ident>Transaction`
///     - `message_id`: The identifier of that message.
///         - Default: `<namespace>:<id>_transaction`
///     - `ref_ident`: The `Ref` that holds a reference to the listener, allowing direct access of
///       the methods.
///         - Default: `<ident>Ref`
/// - `get_ref`: If the macro should generate a [`GetRef`] implementation for the listener.
pub use auditeur_macros::respond;
