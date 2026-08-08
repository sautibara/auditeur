//! A [`Resource`] that is made up of many concurrently-run [event streams](Stream).
//!
//! Use [`#[listener]`](macro@crate::listener) to create one of these resources.
//!
//! # Streams
//!
//! [`Listener`]s are broken up into [`Stream`]s of events. Each stream is run concurrently, all
//! waiting for new events at the same time(see [`Stream::wait`]). Once a stream receives an
//! event, it obtains mutable access to the listener and handles the event (see
//! [`Stream::handle`] ). After handling it, it yields control back to the main loop so that other
//! streams can wait  for events as well.
//!
//! For example, the [`Listen`] trait provides [`ListenStream`]: which allows listening to a
//! [`ListenMessage`] stream shared between all other resources (a broadcast).
//!
//! However, since [`Listen`] is a broadcast, it can't form a message channel to send a response
//! back to the original sender. For this case, you can instead use [`Take`], which in turn
//! provides [`TakeStream`]. This trait allows you to send messages that only a single listener
//! can see, providing a mechanism for forming one-to-one conversations. For example,
//! [`#[respond]` ] uses `Take` to allow other resources to call methods with return values on a
//! listener.
//!
//! # Examples
//!
//! ```
//! use std::time::Duration;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! // [`Listener`]s are resources that primarily "listen" to message streams.
//!
//! // Here's a simple [`ListenMessage`],
//! #[derive(Clone, Debug)]
//! #[listen_message("test:exit")]
//! struct Exit;
//!
//! // and a listener that listens to that message.
//! #[listener(listen(Exit))] // (take note of the `listen(Exit)` part!)
//! #[resource("test:exit_listener")]
//! #[create(default)]
//! struct ExitListener {
//!     #[create(input)]
//!     app: App,
//! }
//!
//! impl Listen<Exit> for ExitListener {
//!     // When the listener hears an [`Exit`] message,
//!     async fn on(&mut self, _: Exit) -> Result<()> {
//!         // it exits the app.
//!         self.app.exit().await;
//!         Ok(())
//!     }
//! }
//!
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), auditeur::Error> {
//! let mut app = App::builder()
//!     .with::<Exit>(())
//!     .with::<ExitListener>(())
//!     .spawn();
//!
//! // Now, if we make sure that the listener is loaded (and thus listening),
//! let load = app
//!     .components_mut()
//!     .load(ExitListener::component_id())
//!     .await?;
//!
//! // we could send an exit message,
//! let sender = app.listen_mut().shouter::<Exit>().await?;
//! sender.shout(Exit);
//!
//! // and the app will exit.
//! drop((sender, load));
//! auditeur::test::finish(app, false).await
//!
//! // Note that if we didn't make sure the listener was already loaded, there would be a chance
//! // that it wouldn't see the message due to [`ListenMessage`]s using broadcast channels.
//! # }
//! ```

use std::{convert::Infallible, marker::PhantomData, pin::Pin};

use crate::{
    app::{AppFn, AppFnOnce, AppMutFnOnce, Event, event::ComponentLifecycle},
    component::{
        ComponentIdPath,
        listen::{self, ListenMessage},
        registry::Registry,
        resource::{Options, Resource, ResourceParts, Runner, StopListener},
        take::{TakeMessage, TakeMessages, Taker},
    },
    prelude::*,
};

use bevy_utils_proc_macros::all_tuples;
use futures::Future;
use tokio::select;

/// A [`Resource`] that is made up of many concurrently-run [event streams](Stream).
///
/// See the [module documentation](self) for more information.
pub trait Listener: Resource {
    /// The list of [`Stream`]s, starting with either [`Unbiased`] or [`Biased`].
    type Streams: StreamList<Self>;

    /// A method that is run after the listener is loaded and before it starts listening.
    ///
    /// The default implementation is to do nothing.
    fn init(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    /// A method that is run after it ends listening and before it exits.
    ///
    /// The default implementation is to do nothing.
    fn exit(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    /// Provides any children that this listener should be added with.
    #[must_use]
    fn children(children: Children) -> Children {
        children
    }

    /// Options to overlay on the default options.
    const OPTIONS: Options = Options::new();
}

/// A stream of data that a [`Listener`] listens to.
pub trait Stream<L: Listener>: Input {
    /// A value that is obtained from the stream and should be [handled](Self::handle).
    type Value: Send;

    /// Wait for the next value to come from the stream.
    ///
    /// Returns [`ControlFlow::Continue`] when a value arrives, or [`ControlFlow::Break`] if the
    /// listener should exit.
    fn wait(
        &mut self,
        listener: &L,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<(), Self::Value>>> + Send;

    /// Handles the value that came from the stream, using the listener.
    ///
    /// Returns [`ControlFlow::Continue`] on success, or [`ControlFlow::Break`] if the
    /// listener should exit.
    fn handle(
        &mut self,
        value: Self::Value,
        listener: &mut L,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<()>>> + Send;

    /// Provides any children that this listener should be added with.
    #[must_use]
    fn children(children: Children) -> Children {
        children
    }
}

/// A [`Listener`] that wants to [listen](listen::Listener::listen) to messages of type `M`.
pub trait Listen<M: ListenMessage>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// A [`Stream`] that listens to a [`listen::Listener`] and calls [`Listen::on`] for every message.
pub struct ListenStream<M: ListenMessage> {
    receiver: listen::Listener<M>,
}

impl<M: ListenMessage> Input for ListenStream<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        Ok(Self {
            receiver: app.listen_mut().listener().await?,
        })
    }
}

impl<L: Listen<M>, M: ListenMessage> Stream<L> for ListenStream<M> {
    type Value = M;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        Ok(ControlFlow::Continue(self.receiver.listen().await))
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        listener.on(message).await?;
        Ok(ControlFlow::Continue(()))
    }
}

/// A [`Listener`] that wants to [take](Taker::take) messages of type `M`.
pub trait Take<M: TakeMessage>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// A [`Stream`] that listens to a [`take::Taker`](Taker) and calls [`Take::on`] for every message.
pub struct TakeStream<M: TakeMessage> {
    taker: Taker<M>,
}

impl<M: TakeMessage> Input for TakeStream<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        Ok(Self {
            taker: app.take_mut().taker().await?,
        })
    }
}

impl<R: Take<M>, M: TakeMessage> Stream<R> for TakeStream<M> {
    type Value = M;

    async fn wait(&mut self, _: &R) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        Ok(ControlFlow::Continue(self.taker.take().await))
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        responder: &mut R,
    ) -> anyhow::Result<ControlFlow<()>> {
        responder.on(message).await?;
        Ok(ControlFlow::Continue(()))
    }

    fn children(children: Children) -> Children {
        children.within::<TakeMessages, M>(())
    }
}

/// A [`Stream`] that listens to a [`StopListener`] and stops the listener when it says to.
#[derive(Input)]
pub struct StopStream {
    stop: StopListener,
}

impl<L: Listener> Stream<L> for StopStream {
    type Value = Infallible;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        self.stop.wait().await;
        Ok(ControlFlow::Break(()))
    }

    async fn handle(&mut self, _: Self::Value, _: &mut L) -> anyhow::Result<ControlFlow<()>> {
        // NOTE: This can't ever be called, since event is (should be) the never type.
        Ok(ControlFlow::Break(()))
    }
}

/// A [`Listener`] that wants to be notified whenever a component is added in the [`Registry`] `R`.
pub trait OnAddIn<R: Registry>: Listener {
    fn on_add(&mut self, id: ComponentIdPath) -> impl Future<Output = anyhow::Result<()>> + Send;
}

/// A [`Stream`] that listens to the [`App`] and calls [`OnAddIn::on_add`] whenever a component is
/// added in the [`Registry`] `R`.
pub struct OnAddInStream<R: Registry> {
    listener: listen::Listener<Event>,
    queue: Vec<ComponentIdPath>,
    pd: PhantomData<fn() -> R>,
}

impl<R: Registry> Input for OnAddInStream<R> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        let (map, listener) = (AppFn.map().get().using_mut())
            .then(AppFn.get_event_listener())
            .call(app)
            .await;
        let listener = listener?;

        let queue = (map.components.keys())
            .filter(|component| component.registry == R::id())
            .map(|id| id.component.clone())
            .collect();

        Ok(Self {
            listener,
            queue,
            pd: PhantomData,
        })
    }
}

impl<L: OnAddIn<R>, R: Registry> Stream<L> for OnAddInStream<R> {
    type Value = ComponentIdPath;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        if let Some(id) = self.queue.pop() {
            return Ok(ControlFlow::Continue(id));
        }

        loop {
            match self.listener.listen().await {
                Event::Component { id, lifecycle }
                    if id.registry == R::id() && lifecycle == ComponentLifecycle::Add =>
                {
                    return Ok(ControlFlow::Continue(id.component));
                }
                _ => (),
            }
        }
    }

    async fn handle(
        &mut self,
        value: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        listener.on_add(value).await?;
        Ok(ControlFlow::Continue(()))
    }
}

/// A [`Listener`] that uses the [`#[respond]`](macro@crate::respond) macro.
pub trait Respond: Listener {
    type Query: TakeMessage;
}

/// An [`FnOnce`] that returns a [`Send`] [`Future`].
// Credit: `https://stackoverflow.com/a/79744535`
pub trait SendAsyncFnOnce<A, O>: FnOnce(A) -> Self::Fut + Send + 'static {
    type Fut: Future<Output = O> + Send;
}

impl<A, O, Out, F> SendAsyncFnOnce<A, O> for F
where
    Out: Future<Output = O> + Send,
    F: FnOnce(A) -> Out + Send + 'static,
{
    type Fut = Out;
}

/// A boxed [`SendAsyncFnOnce`] that uses dynamic dispatch to run, used in the
/// [`#[respond]`](macro@crate::respond) macro.
#[allow(clippy::type_complexity)] // wi wi wi
pub struct BoxedTransactionFnOnce<R: Respond>(
    Box<dyn for<'a> FnOnce(&'a mut R) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send>,
);

impl<R: Respond> BoxedTransactionFnOnce<R> {
    #[doc(hidden)]
    pub fn new(func: impl for<'a> SendAsyncFnOnce<&'a mut R, ()>) -> Self {
        Self(Box::new(|resource| Box::pin((func)(resource))))
    }

    #[doc(hidden)]
    pub fn new_with_sender<T: Send + 'static>(
        func: impl for<'a> SendAsyncFnOnce<&'a mut R, T>,
        sender: tokio::sync::oneshot::Sender<T>,
    ) -> Self {
        Self(Box::new(|resource| {
            Box::pin(async {
                let res = (func)(resource).await;
                let _ = sender.send(res);
            })
        }))
    }

    #[doc(hidden)]
    pub async fn run(self, resource: &mut R) {
        (self.0)(resource).await;
    }
}

/// A [`Stream`] that listens to [`#[respond]`](macro@crate::respond) requests and responds to
/// them.
pub type RespondStream<L> = TakeStream<<L as Respond>::Query>;

/// A flag to tell a [`StreamList`] that its [`select!`] should be unbiased.
#[derive(Input)]
pub struct Unbiased;
/// A flag to tell a [`StreamList`] that its [`select!`] should be biased.
#[derive(Input)]
pub struct Biased;

/// A list of [`Stream`]s that all listen independently.
///
/// This is always a tuple that begins with [`Unbiased`] or [`Biased`] and ends with a variable
/// amount of [`Stream`]s.
#[allow(private_bounds)]
pub trait StreamList<L: Listener>: Sealed<L> + Input + Send {
    #[doc(hidden)]
    fn run(&mut self, listener: &mut L, app: &mut App) -> impl Future<Output = ()> + Send;
    #[doc(hidden)]
    fn children(children: Children) -> Children;
}

pub(crate) trait Sealed<L> {}

macro_rules! impl_stream_list {
    ($(($T:tt, $c:tt)),*) => {
        impl_stream_list!((Unbiased) () $(($T, $c)),*);
        impl_stream_list!((Biased) (biased;) $(($T, $c)),*);
    };
    (($bias_marker:ident) ($($bias:tt)*) $(($T:tt, $c:tt)),*) => {
        #[allow(unused_parens)]
        impl<L, $($T,)*> StreamList<L> for ($bias_marker, $($T),*)
        where
            Self: Input,
            $($T: Stream<L>,)*
            L: Listener<Streams = Self>,
        {
            async fn run(&mut self, listener: &mut L, app: &mut App) {
                let (_, $($c),*) = self;

                loop {
                    select! { $($bias)* $(
                        res = $c.wait(&listener) => {
                            let Some(val) = app.errors_mut().drain_any_result(res).await
                                else { continue; };
                            let ControlFlow::Continue(val) = val else { break; };

                            let res = $c.handle(val, listener).await;

                            let Some(val) = app.errors_mut().drain_any_result(res).await
                                else { continue; };
                            if val.is_break() { break; }
                        },
                    )*}
                }
            }

            fn children(children: Children) -> Children {
                $(let children = $T::children(children);)*
                children
            }
        }

        #[allow(unused_parens)]
        impl<L, $($T,)*> Sealed<L> for ($bias_marker, $($T),*)
        where
            Self: Input,
            $($T: Stream<L>,)*
            L: Listener<Streams = Self>,
        {}
    };
}

all_tuples!(impl_stream_list, 1, 16, T, c);

/// Used in the [`listener`](macro@crate::listener) attribute macro.
///
/// Use the `options` flag in the macro to use this trait.
pub trait ListenerDerive: Send {
    const OPTIONS: Options = Options::new();

    fn init(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    fn exit(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    #[must_use]
    fn children(children: Children) -> Children {
        children
    }
}

/// A [`GetRunnerWith`](super::GetRunnerWith) [`Method`](super::GetRunnerWith::Method) that uses
/// [`Listener`] to form its [`Runner`].
pub struct ListenerMethod;
impl<L: Listener> ResourceParts<L> for ListenerMethod {
    type Runner = ListenerRunner<L>;

    async fn runner(listener: L, app: &mut App) -> Result<Self::Runner, Error> {
        Ok(ListenerRunner {
            listener,
            components: L::Streams::create(app).await?,
        })
    }

    fn children(children: Children) -> Children {
        let children = <L as Listener>::children(children);
        L::Streams::children(children)
    }

    const OPTIONS: Options = <L as Listener>::OPTIONS;
}

/// The [`Runner`] for [`ListenerMethod`].
pub struct ListenerRunner<L: Listener> {
    listener: L,
    components: L::Streams,
}

impl<L: Listener> Runner for ListenerRunner<L> {
    async fn run(mut self, app: &mut App) {
        let res = self.listener.init().await;
        app.errors_mut().drain_any_result(res).await;

        self.components.run(&mut self.listener, app).await;

        let res = self.listener.exit().await;
        app.errors_mut().drain_any_result(res).await;
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use tokio::sync::broadcast;

    use crate::prelude::*;

    #[tokio::test]
    async fn weak_listener_exits() -> Result<(), Error> {
        #[resource("test:resource")]
        #[listener(weak)]
        #[create(default)]
        struct TestResource;

        let app = App::builder().with::<TestResource>(()).spawn();

        crate::test::finish(app, false).await
    }

    #[tokio::test]
    async fn listener_derive_init() -> Result<(), Error> {
        #[resource("test:resource")]
        #[listener(weak, options)]
        struct TestResource(broadcast::Sender<()>);

        // Create just passes in the sender.
        impl Create for TestResource {
            type ExternalInput = broadcast::Sender<()>;
            async fn create(external: &Self::ExternalInput, _: &mut App) -> Result<Self, Error> {
                Ok(Self(external.clone()))
            }
        }

        impl ListenerDerive for TestResource {
            async fn init(&mut self) -> anyhow::Result<()> {
                self.0.send(())?;
                Ok(())
            }
        }

        let (sender, mut receiver) = broadcast::channel(1);
        let mut app = App::builder().with::<TestResource>(sender).spawn();

        // Make sure the resource loads at least once.
        app.components_mut()
            .load_lazy(TestResource::component_id())
            .await?;

        // Wait until the resource initializes.
        (receiver.recv().await)
            .map_err(|_| anyhow!("resource should send message"))
            .map_err(|err| app.errors().wrap(err))?;

        // Then exit.
        crate::test::finish(app, false).await
    }
}
