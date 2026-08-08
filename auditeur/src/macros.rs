//! A separate module for imported macros just to clean up `lib.rs`

#[allow(unused_imports)] // used for doc links
use crate::prelude::*;

/// Implements the traits required for [`Resource`](crate::component::resource::Resource) for any
/// type.
///
/// This automatically implements [`Identified`], [`Any`](crate::Any), and [`MaybeGetRef`]. You will
/// still have to manually implement [`ResourceParts`](crate::component::resource::ResourceParts),
/// [`GetRunnerWith`](crate::component::resource::GetRunnerWith), or use one of the macros that
/// will do so for you (like [`listener`]). See
/// [the module documentation for resource](crate::component::resource) for more information.
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
/// #[pure_resource] // Still need to implement [`ResourceParts`] somehow.
/// #[create(default)] // And create.
/// struct UsefulResource;
///
/// assert_eq!(UsefulResource::id(), ident!("test:useful_resource"));
/// # }
/// ```
pub use auditeur_macros::resource;

/// Implements [`Identified`] for any type.
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::*;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[identified("test:has_identifier")]
/// struct HasIdentifier;
///
/// fn assert_is_identified<I: auditeur::component::Identified>(_: I) {}
///
/// assert_eq!(HasIdentifier::id(), ident!("test:has_identifier"));
/// assert_is_identified(HasIdentifier);
/// # }
/// ```
pub use auditeur_macros::identified;

/// Implements [`ListenMessage`](crate::component::listen::ListenMessage) for any type.
///
/// This automatically implements [`Identified`] and [`Any`](crate::Any). You will still have to
/// manually implement [`Clone`].
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
/// #[listen_message("test:message")]
/// struct Message;
///
/// fn assert_is_message<M: auditeur::component::listen::ListenMessage>(_: M) {}
///
/// assert_eq!(Message::id(), ident!("test:message"));
/// assert_is_message(Message);
/// # }
/// ```
pub use auditeur_macros::listen_message;

/// Implements [`TakeMessage`](crate::component::take::TakeMessage) for any type.
///
/// This automatically implements [`Identified`] and [`Any`](crate::Any).
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::*;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[take_message("test:message")]
/// struct Message;
///
/// fn assert_is_message<M: auditeur::component::take::TakeMessage>(_: M) {}
///
/// assert_eq!(Message::id(), ident!("test:message"));
/// assert_is_message(Message);
/// # }
/// ```
pub use auditeur_macros::take_message;

/// Implements [`GetRunnerWith`](crate::component::resource::GetRunnerWith) using [`Listener`].
///
/// # Usage
///
/// ## Options
///
/// This macro is mainly used to fill out the type alias [`Listener::Streams`]. If you wish
/// to overwrite other parts of the trait, add an `options` flag to the macro and implement
/// [`ListenerDerive`]. The macro will fill out the other parts of the trait based on what is
/// in the implementation for [`ListenerDerive`].
///
/// # Parameters
///
/// ## `Listen`
///
/// To listen to messages using [`Listen`], provide the list of them wrapped by
/// `listen(<message>,*)`.
///
/// ## `Take`
///
/// Much like [`Listen`], to take messages with [`Take`], provide the list of them wrapped by
/// `take(<message>,*)`.
///
/// ## Streams
///
/// To use other [`Stream`](crate::component::resource::listener::Stream)s, provide a list of
/// stream types in a `streams` list.
///
/// ## Stop Stream
///
/// All listeners by default have a
/// [`StopStream`](crate::component::resource::listener::StopStream), which automatically handles
/// exiting when the app tells it to. Only disable this if you will manually listen for this.
///
/// To disable adding a [`StopStream`](crate::component::resource::listener::StopStream), specify
/// `stop_stream = false`.
///
/// ## Bias
///
/// All listeners by default try to poll their components fairly by randomly checking a component
/// to poll each time. However, if the `biased` parameter is used, the listener will check the
/// components in order instead. That is, the listener will always choose earlier components over
/// later components, if they have new things to do.
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
///   | weak
///   | options
///   | biased
///   | stop_stream = <boolean>
///   | listen(<type>,*)
///   | take(<type>,*)
///   | on_add_in(<type>,*)
///   | streams(<type>,*)
/// ```
///
/// </details>
pub use auditeur_macros::listener;

/// Implements [`GetRunnerWith`](crate::component::resource::GetRunnerWith) using
/// [`resource::pure`](mod@crate::component::resource::pure).
///
/// # Examples
///
/// See the module documentation of [`resource::pure`](mod@crate::component::resource::pure).
pub use auditeur_macros::pure_resource;

/// Implements [`GetRunnerWith`](crate::component::resource::GetRunnerWith) using
/// [`Watcher`](crate::component::resource::watcher::Watcher).
///
/// # Examples
///
/// See the module documentation of [`watcher`](crate::component::resource::watcher).
pub use auditeur_macros::watcher;

/// Implements [`Create`] for any type.
///
/// This macro is only supported for `struct`s, as it goes field by field to determine how to create
/// the type. If a field has a `#[create(input)]` or `#[create(create)]` attribute, it will
/// automatically be initialized using the [`Input`] or [`Create`] trait respectively. Fields
/// without attributes will be wrapped into a `Plain` struct generated by the macro. Then, types
/// using this attribute must also implement the [`CreateDerive`] trait, which describes how to
/// initialize the `Plain` struct.
///
/// ## Examples
///
/// ```
/// use auditeur::prelude::*;
/// use auditeur::*;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), auditeur::Error> {
/// // Here's an example of a typical [`Create`] implementation.
/// #[create]
/// struct Foo {
///     // [`Vec`] doesn't implement [`Input`] or [`Create`], so we'll initialize it manually.
///     // Usually, fields without attributes are put above fields with them.
///     list: Vec<()>,
///
///     // Since `input` is an [`App`], we could automatically obtain it using [`Input`].
///     #[create(input)]
///     app: App,
///
///     // [`Bar`] and [`Baz`], on the other hand, implement [`Create`], so we use that instead.
///     #[create(create)]
///     bar: Bar,
///     #[create(create)]
///     baz: Baz,
/// }
///
/// impl CreateDerive for Foo {
///     async fn create((): &Self::External, _: &mut App) -> Result<Self::Plain, Error> {
///         Ok(Self::Plain {
///             // `list` didn't have any attributes, so we initialize it here.
///             list: Vec::new(),
///
///             // `input` is initialized using [`Input`], so we omit it.
///
///             // `bar` and `baz` are initialized using [`Create`],
///             // so we give them their external input.
///             bar: true,
///             baz: (),
///         })
///     }
/// }
///
/// // Custom external input can be specified with `external_input`.
/// #[create(external_input(bool))]
/// struct Bar {
///     boolean: bool,
/// }
///
/// impl CreateDerive for Bar {
///     // Since we specified the external input, it appears here,
///     async fn create(boolean: &bool, _: &mut App) -> Result<Self::Plain, Error> {
///         // and we could use it to initialize the type.
///         Ok(Self::Plain { boolean: *boolean })
///     }
/// }
///
/// // If all fields implement [`Default`], we could use `default` to initialize them instead.
/// #[create(default)]
/// struct Baz {
///     number: usize,
/// }
///
///
/// let mut app = App::builder().spawn();
///
/// // Now, when we create [`Foo`],
/// let foo = <Foo as Create>::create(&(), &mut app).await?;
///
/// // We could access all the fields however we want.
/// assert!(foo.list.is_empty());
/// let _ = foo.app.map().get().await;
/// assert_eq!(foo.bar.boolean, true);
/// assert_eq!(foo.baz.number, 0);
///
/// drop(foo);
/// auditeur::test::finish(app, false).await
/// # }
/// ```
///
/// ## Outer Attributes
///
/// ### Default
///
/// If the outer attribute includes a `default` flag, it will derive [`Default`] for the `Plain`
/// struct and [`CreateDerive`] for the outer struct using `Plain`'s default implementation.
///
/// ### External Input
///
/// This macro defaults to the external input of `()`. If a different external input is wanted,
/// specify `external_input(<type>,*)`.
///
/// ### Derive for Plain
///
/// If you want to derive traits for the `Plain` struct, specify `derive_for_plain(<derive>,*)`.
/// This is most useful for the [`GetCreate`](crate::component::resource::get::GetCreate) get
/// method.
///
/// ### Syntax
///
/// ```text
/// <attribute>:
///   | #[create(<parameter>,*)]
///
/// <parameter>:
///   | default
///   | external_input(<type>,*)
///   | derive_for_plain(<macro>,*)
/// ```
///
/// ## Field Attributes
///
/// ### Input
///
/// To automatically create this field using the [`Input`] trait, specify `input` in the field
/// attribute. This field will then be completely omitted from the `Plain` struct.
///
/// ### Create
///
/// To automatically create this field using the [`Create`] trait, specify `create` in the field
/// attribute. This field will then obtain its external input from the `Plain` struct.
///
/// ### Syntax
///
/// ```text
/// <attribute>:
///   | #[create(<parameter>,*)]
///
/// <parameter>:
///   | input
///   | create
/// ```
pub use auditeur_macros::create;

/// Automatically provides an implementation of [`Input`](trait@Input).
///
/// This simply uses the [`Input`](trait@Input) implementations of each field.
pub use auditeur_macros::Input;

/// Automatically provides an implementation of
/// [`MaybeGetRef`](crate::component::resource::get::MaybeGetRef).
///
/// Usually, though, this is already implemented by [`resource`](macro@resource) or one of its
/// derivatives.
///
/// See [`MaybeGetRef`](crate::component::resource::get::MaybeGetRef) for more information on why
/// this is  necessary
pub use auditeur_macros::MaybeGetRef;

/// Automatically provides an implementation of [`CloneIn`](crate::input::CloneIn).
///
/// This uses specialization for each field to either use the field's
/// [`CloneIn`](crate::input::CloneIn) implementation or its [`Clone`] implementation,
/// whichever exists.
pub use auditeur_macros::CloneIn;

/// Automatically provides an implementation of [`PathAny`](crate::path_any::PathAny) by
/// implementing [`PathAnyConst`](crate::path_any::PathAnyConst)
///
/// If the `path-any` feature is disabled, this is a no-op.
pub use auditeur_macros::Any;

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
/// # use auditeur::*;
///
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
///         self.sender.shout(CounterChanged);
///     }
/// }
///
/// #[derive(Clone)]
/// #[listen_message("test:counter_changed")]
/// struct CounterChanged;
///
/// #[tokio::main]
/// async fn main() -> Result<(), auditeur::Error> {
///     let mut app = App::builder()
///         .with::<Counter>(())
///         .with::<CounterChanged>(())
///         .spawn();
///
///     let listener = app.listen_mut().listener::<CounterChanged>().await?;
///     let counter = app.resources_mut().get::<Counter>().await?;
///
///     assert!(listener.is_empty());
///     assert_eq!(counter.get().await, 0);
///
///     counter.set(5).await;
///
///     assert!(!listener.is_empty());
///     assert_eq!(counter.get().await, 5);
///
///     drop((listener, counter));
///     auditeur::test::finish(app, true).await
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
/// # use auditeur::*;
///
/// #[resource("test:vec_resource")]
/// #[listener(respond)]
/// #[create(default)]
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
///     let mut app = App::builder()
///         .with::<VecResource>(())
///         .spawn();
///
///     let vec = Get::<VecResource>::create(&mut app).await?;
///
///     // Use `spawn_transaction` to run a transaction without returning a value.
///     // Note that this won't wait for the resource to run the future
///     // - use `spawn_transaction` for that
///     vec.spawn_transaction(async |vec: &mut VecResource| {
///         for val in (0..5).rev() {
///             // Note that transactions are run by the listener itself, so we don't have to
///             // await any messages being sent.
///             vec.push_usize(val);
///         }
///     });
///
///     // Here, run_transaction is used to return a value: the first five elements in the vec.
///     let vals: Vec<usize> = vec.run_transaction(async |vec: &mut VecResource| {
///         (0..5).map(|index| vec.get_unwrap(index)).collect()
///     }).await;
///
///     assert_eq!(&[4, 3, 2, 1, 0], &*vals);
///
///     drop(vec);
///     auditeur::test::finish(app, true).await
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
/// use auditeur::component::IdentifiedType;
/// # use auditeur::*;
///
/// #[resource("test:attribute_example")]
/// #[listener(respond)]
/// #[create(default)]
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
///     pub fn push_ident_of<I: IdentifiedType>(&mut self) {
///         self.push_ident(I::id().to_owned())
///     }
///
///     #[respond(connection_fn)]
///     pub async fn push_ident_of<I: IdentifiedType>(&self) {
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
///     let mut app = App::builder()
///         .with::<IdentList>(())
///         .spawn();
///
///     #[identified("test:identifiable1")]
///     struct TestIdentifiable1;
///     #[identified("test:identifiable2")]
///     struct TestIdentifiable2;
///
///     let conn = Get::<IdentList>::create(&mut app).await?;
///
///     conn.push_ident(ident!("test:ident1").to_owned()).await;
///     conn.push_ident_of::<TestIdentifiable1>().await;
///     conn.run_transaction(async |reference: &mut IdentList| {
///         reference.push_ident_of::<TestIdentifiable2>();
///     }).await;
///
///     assert_eq!(*ident!("test:ident1"), conn.index(0).await);
///     assert_eq!(*ident!("test:identifiable1"), conn.index(1).await);
///     assert_eq!(*ident!("test:identifiable2"), conn.index(2).await);
///
///     drop(conn);
///     auditeur::test::finish(app, true).await
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
/// - `get_ref`: If the macro should generate a [`GetRef`](crate::component::resource::get::GetRef)
///   implementation for the listener.
pub use auditeur_macros::respond;
