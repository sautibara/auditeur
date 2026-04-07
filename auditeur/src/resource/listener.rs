#![doc = include_str!("listener_docs.md")]

use std::{marker::PhantomData, pin::Pin, task::Poll};

use crate::{
    Message,
    app::{
        LoadedResource, Loader, Runner,
        channels::{self, EventReceiver, EventSender, Taker},
    },
    prelude::*,
};

use bevy_utils_proc_macros::all_tuples;
use futures::Future;
use tokio::select;

// TODO: remove this once #100013 is stabilized
use send_future::SendFuture;
use tracing::debug;

// pub use super::builtins::Logger;

#[doc = include_str!("listener_docs.md")]
pub trait Listener: Resource + Runnable + Sized {
    /// The list of [`Component`]s, starting with a unit tuple.
    type Components: ComponentList<Self>;
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

    /// Returns a set of channels that this component "owns".
    ///
    /// These channels will automatically be added to the [registry](Registry), but special care
    /// should be taken to make sure that a specific channel isn't added multiple times.
    #[must_use]
    fn channels() -> Channels {
        Channels::new()
    }
}

pub trait Listen<M: ListenMessage>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct ListenComponent<L: Listen<M>, M: ListenMessage> {
    receiver: channels::Listener<M>,
    listener: PhantomData<L>,
}

impl<L: Listen<M>, M: ListenMessage> Input for ListenComponent<L, M> {
    async fn create(registry: &Registry, dependent: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.channels().listener::<M>(dependent).await?,
            listener: PhantomData,
        })
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::id())
    }
}

impl<L: Listen<M>, M: ListenMessage> Component<L> for ListenComponent<L, M> {
    type Value = M;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        listener.on(message).send().await?;
        Ok(ControlFlow::Continue(()))
    }
}

pub trait ListenToRegistry: Listener {
    fn on(&mut self, message: RegistryEvent) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

pub struct ListenToRegistryComponent<L: ListenToRegistry> {
    receiver: EventReceiver,
    listener: PhantomData<L>,
}

impl<L: ListenToRegistry> Input for ListenToRegistryComponent<L> {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        let mut receiver = registry.event_receiver();
        receiver.mark_seen();
        Ok(Self {
            receiver,
            listener: PhantomData,
        })
    }
}

impl<L: ListenToRegistry> Component<L> for ListenToRegistryComponent<L> {
    type Value = RegistryEvent;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        let should_break = listener.on(message).send().await?;
        if should_break {
            Ok(ControlFlow::Break(()))
        } else {
            Ok(ControlFlow::Continue(()))
        }
    }
}

pub trait OnLoad<R: Resource>: Listener {
    fn on_load(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub struct OnLoadComponent<L: OnLoad<R>, R: Resource> {
    receiver: EventReceiver,
    should_init: bool,
    listener: PhantomData<(L, R)>,
}

impl<L: OnLoad<R>, R: Resource> Input for OnLoadComponent<L, R> {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        let mut receiver = registry.event_receiver();
        receiver.mark_seen();

        let should_init = registry.loaded_resources().await.contains(R::id());

        Ok(Self {
            receiver,
            should_init,
            listener: PhantomData,
        })
    }
}

impl<L: OnLoad<R>, R: Resource> Component<L> for OnLoadComponent<L, R> {
    type Value = Option<RegistryEvent>;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        if self.should_init {
            self.should_init = false;
            return Ok(ControlFlow::Continue(None));
        }

        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(Some(message))),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        if message.is_none()
            || matches!(&message, Some(RegistryEvent::Running(id)) if *id == *R::id())
        {
            listener.on_load().send().await?;
        }
        Ok(ControlFlow::Continue(()))
    }
}

pub struct TakeComponent<R: Take<M>, M: Message> {
    receiver: Taker<M>,
    pd: PhantomData<R>,
}

impl<R: Take<M>, M: Message> Input for TakeComponent<R, M> {
    async fn create(registry: &Registry, dependent: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.channels().taker::<M>(dependent).await?,
            pd: PhantomData,
        })
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::id())
    }
}

pub trait Take<M: Message>: Listener {
    fn on(&mut self, message: M) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl<R: Take<M>, M: Message> Component<R> for TakeComponent<R, M> {
    type Value = M;

    async fn wait(&mut self, _: &R) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        responder: &mut R,
    ) -> anyhow::Result<ControlFlow<()>> {
        responder.on(message).send().await?;
        Ok(ControlFlow::Continue(()))
    }

    fn channels() -> Channels {
        Channels::new().with_take_channel::<M>()
    }
}

pub struct RegistryComponent {
    receiver: EventReceiver,
    #[cfg(feature = "runtime-loading")]
    registry: Registry,
}

impl Input for RegistryComponent {
    #[cfg_attr(not(feature = "runtime-loading"), expect(unused_variables))]
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        let mut receiver = registry.event_receiver();
        receiver.mark_seen();
        Ok(Self {
            receiver,
            #[cfg(feature = "runtime-loading")]
            registry: registry.handle(),
        })
    }

    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

impl<L: Listener> Component<L> for RegistryComponent {
    type Value = RegistryEvent;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    #[cfg_attr(not(feature = "runtime-loading"), expect(unused_variables))]
    async fn handle(
        &mut self,
        event: Self::Value,
        listener: &mut L,
    ) -> anyhow::Result<ControlFlow<()>> {
        Ok(match event {
            RegistryEvent::Stop(ref id) if id == L::id() => ControlFlow::Break(()),
            RegistryEvent::Exit => ControlFlow::Break(()),
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::Get(ref get_request) => {
                get_request.check(listener, &self.registry).await;
                ControlFlow::Continue(())
            }
            _ => ControlFlow::Continue(()),
        })
    }
}

pub trait Respond: Listener {
    type Query: crate::Message;
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
                let res = (func)(resource).send().await;
                let _ = sender.send(res);
            })
        }))
    }

    pub async fn run(self, resource: &mut R) {
        (self.0)(resource).await;
    }
}

#[pin_project::pin_project]
pub struct RespondFuture<RT, Fut1, Fut2>
where
    Fut1: Future<Output = ()> + Send,
    Fut2: Future<Output = RT> + Send + 'static,
{
    #[pin]
    fut1: Fut1,
    #[pin]
    fut2: Fut2,
    state: RespondFutureState,
}

enum RespondFutureState {
    Fut1,
    Fut2,
    End,
}

impl<RT, Fut1, Fut2> RespondFuture<RT, Fut1, Fut2>
where
    Fut1: Future<Output = ()> + Send,
    Fut2: Future<Output = RT> + Send + 'static,
{
    #[doc(hidden)]
    pub const fn new(fut1: Fut1, fut2: Fut2) -> Self {
        Self {
            fut1,
            fut2,
            state: RespondFutureState::Fut1,
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn send_blocking(self) -> impl Future<Output = RT> + Send + 'static {
        match self.state {
            RespondFutureState::Fut1 => {
                tokio::runtime::Handle::current().block_on(self.fut1);
                self.fut2
            }
            RespondFutureState::Fut2 => self.fut2,
            RespondFutureState::End => {
                panic!("attemped to call `send_blocking` when the future was already finished");
            }
        }
    }
}

impl<RT, Fut1, Fut2> Future for RespondFuture<RT, Fut1, Fut2>
where
    Fut1: Future<Output = ()> + Send,
    Fut2: Future<Output = RT> + Send + 'static,
{
    type Output = RT;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.state {
            RespondFutureState::Fut1 => match this.fut1.poll(cx) {
                Poll::Ready(()) => match this.fut2.poll(cx) {
                    ready @ Poll::Ready(_) => {
                        *this.state = RespondFutureState::End;
                        ready
                    }
                    pending => {
                        *this.state = RespondFutureState::Fut2;
                        pending
                    }
                },
                Poll::Pending => Poll::Pending,
            },
            RespondFutureState::Fut2 => match this.fut2.poll(cx) {
                ready @ Poll::Ready(_) => {
                    *this.state = RespondFutureState::End;
                    ready
                }
                pending => pending,
            },
            RespondFutureState::End => {
                panic!("called poll on a finished future")
            }
        }
    }
}

pub type RespondComponent<L> = TakeComponent<L, <L as Respond>::Query>;

pub struct Unbiased;
pub struct Biased;

impl Input for Unbiased {
    async fn create(_: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self)
    }
}

impl Input for Biased {
    async fn create(_: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self)
    }
}

#[allow(private_bounds)]
pub trait ComponentList<L: Listener>: Sealed<L> + Input + Send + Sync {
    fn run(&mut self, listener: &mut L, sender: &EventSender) -> impl Future<Output = ()> + Send;

    fn channels() -> Channels;
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
            async fn run(&mut self, listener: &mut L, sender: &EventSender) {
                let (_, $($c),*) = self;

                loop {
                    select! { $($bias)* $(
                        res = $c.wait(&listener) => {
                            let Some(val) = sender.handle_error(res, listener.dyn_id()) else { continue; };
                            let ControlFlow::Continue(val) = val else { break; };

                            let res = $c.handle(val, listener).send().await;

                            let Some(val) = sender.handle_error(res, listener.dyn_id()) else { continue; };
                            if val.is_break() { break; }
                        },
                    )*}
                }
            }

            fn channels() -> Channels {
                Channels::new()
                    $(.combine_with($T::channels()))*
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

pub(crate) struct LoadedListener<L: Listener> {
    listener: L,
    components: L::Components,
    event_sender: EventSender,
}

impl<L: Listener> LoadedListener<L> {
    pub(crate) async fn build(listener: L, registry: &Registry) -> Result<Self, Error> {
        let ident = ComponentIdRef::Resource(listener.dyn_id());
        Ok(Self {
            listener,
            components: L::Components::create(registry, ident).await?,
            event_sender: registry.event_sender(),
        })
    }
}

#[async_trait]
impl<L: Listener> Runner for LoadedListener<L> {
    async fn run(mut self: Box<Self>) {
        let res = self.listener.init().send().await;
        self.event_sender.handle_error(res, self.listener.dyn_id());

        <L::Components as ComponentList<L>>::run(
            &mut self.components,
            &mut self.listener,
            &self.event_sender,
        )
        .await;

        let res = self.listener.exit().send().await;
        self.event_sender.handle_error(res, self.listener.dyn_id());

        debug!("exit");
    }
}

impl<L: Listener> Identifiable for LoadedListener<L> {
    fn id() -> &'static NamespacedIdRef
    where
        Self: Sized,
    {
        L::id()
    }

    fn dyn_id(&self) -> &'static NamespacedIdRef {
        self.listener.dyn_id()
    }
}

impl<L: Listener> LoadedResource for LoadedListener<L> {
    fn resource(&self) -> &dyn Resource {
        &self.listener
    }

    fn resource_mut(&mut self) -> &mut dyn Resource {
        &mut self.listener
    }

    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Ok(self)
    }
}

pub(crate) struct ListenerLoader<L: Listener + Create> {
    external_input: L::ExternalInput,
}

impl<L: Listener + Create> ListenerLoader<L> {
    pub const fn new(external_input: L::ExternalInput) -> Self {
        Self { external_input }
    }
}

#[async_trait]
impl<L: Listener + Create> Loader for ListenerLoader<L> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let ident = ComponentIdRef::Resource(L::id());
        let registry_input = L::RegistryInput::create(registry, ident).await?;
        let listener = L::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: L::id().into(),
                error,
            })?;
        let runner = LoadedListener::build(listener, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        L::RegistryInput::constraints().extend(L::Components::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        L::id()
    }

    fn channels(&self) -> Channels {
        L::Components::channels()
    }
}
