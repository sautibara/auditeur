#![doc = include_str!("listener_docs.md")]

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
use tracing::debug;

#[doc = include_str!("listener_docs.md")]
pub trait Listener: Resource {
    /// The list of [`Component`]s, starting with either [`Unbiased`] or [`Biased`].
    type Components: ComponentList<Self>;

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

pub trait Component<L: Listener>: Input {
    type Value: Send;

    fn wait(
        &mut self,
        listener: &L,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<(), Self::Value>>> + Send;

    fn handle(
        &mut self,
        value: Self::Value,
        listener: &mut L,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<()>>> + Send;

    #[must_use]
    fn children(children: Children) -> Children {
        children
    }
}

pub trait Listen<M: ListenMessage>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct ListenComponent<M: ListenMessage> {
    receiver: listen::Listener<M>,
}

impl<M: ListenMessage> Input for ListenComponent<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        Ok(Self {
            receiver: app.listen_mut().listener().await?,
        })
    }
}

impl<L: Listen<M>, M: ListenMessage> Component<L> for ListenComponent<M> {
    type Value = M;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.listen().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
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

pub struct TakeComponent<M: TakeMessage> {
    taker: Taker<M>,
}

impl<M: TakeMessage> Input for TakeComponent<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        Ok(Self {
            taker: app.take_mut().taker().await?,
        })
    }
}

pub trait Take<M: TakeMessage>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl<R: Take<M>, M: TakeMessage> Component<R> for TakeComponent<M> {
    type Value = M;

    async fn wait(&mut self, _: &R) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.taker.take().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
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

#[derive(Input)]
pub struct StopComponent {
    stop: StopListener,
}

impl<L: Listener> Component<L> for StopComponent {
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

pub trait OnAddIn<R: Registry>: Listener {
    fn on_add(&mut self, id: ComponentIdPath) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct OnAddInComponent<R: Registry> {
    listener: listen::Listener<Event>,
    queue: Vec<ComponentIdPath>,
    pd: PhantomData<fn() -> R>,
}

impl<R: Registry> Input for OnAddInComponent<R> {
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

impl<L: OnAddIn<R>, R: Registry> Component<L> for OnAddInComponent<R> {
    type Value = ComponentIdPath;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        if let Some(id) = self.queue.pop() {
            return Ok(ControlFlow::Continue(id));
        }

        loop {
            match self.listener.listen().await {
                Some(Event::Component { id, lifecycle })
                    if id.registry == R::id() && lifecycle == ComponentLifecycle::Add =>
                {
                    return Ok(ControlFlow::Continue(id.component));
                }
                Some(_) => (),
                None => {
                    return Ok(ControlFlow::Break(()));
                }
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

pub trait Respond: Listener {
    type Query: TakeMessage;
}

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

#[allow(clippy::type_complexity)] // wi wi wi
pub struct BoxedTransactionFnOnce<R: Respond>(
    Box<dyn for<'a> FnOnce(&'a mut R) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Send>,
);

impl<R: Respond> BoxedTransactionFnOnce<R> {
    pub fn new(func: impl for<'a> SendAsyncFnOnce<&'a mut R, ()>) -> Self {
        Self(Box::new(|resource| Box::pin((func)(resource))))
    }

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

    pub async fn run(self, resource: &mut R) {
        (self.0)(resource).await;
    }
}

pub type RespondComponent<L> = TakeComponent<<L as Respond>::Query>;

#[derive(Input)]
pub struct Unbiased;
#[derive(Input)]
pub struct Biased;

#[allow(private_bounds)]
pub trait ComponentList<L: Listener>: Sealed<L> + Input + Send {
    fn run(&mut self, listener: &mut L, app: &mut App) -> impl Future<Output = ()> + Send;

    fn children(children: Children) -> Children;
}

pub(crate) trait Sealed<L> {}

macro_rules! impl_component_list {
    ($(($T:tt, $c:tt)),*) => {
        impl_component_list!((Unbiased) () $(($T, $c)),*);
        impl_component_list!((Biased) (biased;) $(($T, $c)),*);
    };
    (($bias_marker:ident) ($($bias:tt)*) $(($T:tt, $c:tt)),*) => {
        #[allow(unused_parens)]
        impl<L, $($T,)*> ComponentList<L> for ($bias_marker, $($T),*)
        where
            Self: Input,
            $($T: Component<L>,)*
            L: Listener<Components = Self>,
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
            $($T: Component<L>,)*
            L: Listener<Components = Self>,
        {}
    };
}

all_tuples!(impl_component_list, 1, 16, T, c);

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

pub struct ListenerMethod;
impl<L: Listener> ResourceParts<L> for ListenerMethod {
    type Runner = ListenerRunner<L>;

    async fn runner(listener: L, app: &mut App) -> Result<Self::Runner, Error> {
        Ok(ListenerRunner {
            listener,
            components: L::Components::create(app).await?,
        })
    }

    fn children() -> Children {
        let children = Children::new();
        let children = <L as Listener>::children(children);
        L::Components::children(children)
    }

    const OPTIONS: Options = <L as Listener>::OPTIONS;
}

pub struct ListenerRunner<L: Listener> {
    listener: L,
    components: L::Components,
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
