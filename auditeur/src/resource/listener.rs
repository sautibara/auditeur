#![doc = include_str!("listener_docs.md")]

use std::marker::PhantomData;

use crate::{
    app::{EventReceiver, EventSender, LoadedResource, Loader, RespondReceiver, Runner},
    input::subset::Subset,
    prelude::*,
};

use auditeur_macros::cfg_supertrait;
use bevy_utils_proc_macros::all_tuples;
use futures::Future;
use tokio::select;

// TODO: remove this once #100013 is stabilized
use send_future::SendFuture;
use tracing::debug;

// pub use super::builtins::Logger;

#[doc = include_str!("listener_docs.md")]
pub trait Listener: Resource + Runnable + Sized
where
    Self::Input: for<'s> Subset<Self::InitCombination, SubsetMut<'s> = <Self as Runnable>::InitInput<'s>>
        + for<'s> Subset<Self::ExitCombination, SubsetMut<'s> = <Self as Runnable>::ExitInput<'s>>,
{
    /// A shared list of inputs between [`Self::Components`] and [`Runnable`], starting with a unit
    /// tuple. In essence, each separate input of the components and runnable methods must be a
    /// [`Subset`] of this type.
    type Input: Input;
    /// The list of [`Component`]s, starting with a unit tuple.
    type Components: ComponentList<Self>;

    /// The [combinations](Subset) for each component in order, stored in a tuple.
    type ComponentCombinations;
    /// The [combination](Subset) for [`Runnable::init`]
    type InitCombination;
    /// The [combination](Subset) for [`Runnable::exit`]
    type ExitCombination;
}

pub trait Component<L: Listener>: Input {
    type UserInput<'s>: Send;
    type Value: Send;

    fn wait(
        &mut self,
        listener: &L,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<(), Self::Value>>> + Send;

    fn handle(
        &mut self,
        value: Self::Value,
        listener: &mut L,
        input: Self::UserInput<'_>,
    ) -> impl Future<Output = anyhow::Result<ControlFlow<()>>> + Send;
}

pub trait Listen<M: Message>: Listener {
    type OnInput<'s>: Send;

    fn on(
        &mut self,
        message: M,
        input: <Self as Listen<M>>::OnInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[component((<$_:ty, $message:ty>) => (implementation = Listen<$message>))]
pub struct ListenComponent<L: Listen<M>, M: Message> {
    receiver: tokio::sync::broadcast::Receiver<M>,
    listener: PhantomData<L>,
}

impl<L: Listen<M>, M: Message> Input for ListenComponent<L, M> {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.message_receiver::<M>(),
            listener: PhantomData,
        })
    }
}

impl<L: Listen<M>, M: Message> Component<L> for ListenComponent<L, M> {
    type Value = M;
    type UserInput<'s> = <L as Listen<M>>::OnInput<'s>;

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Ok(message) => Ok(ControlFlow::Continue(message)),
            Err(_) => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        message: Self::Value,
        listener: &mut L,
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        listener.on(message, input).send().await?;
        Ok(ControlFlow::Continue(()))
    }
}

pub trait Respond<M: Message>: Listener {
    type Response: Response;

    type OnInput<'s>: Send;
    fn on(
        &mut self,
        message: M,
        input: <Self as Respond<M>>::OnInput<'_>,
    ) -> impl Future<Output = Self::Response> + Send;
}

#[cfg_supertrait(feature = "path-any", crate::path_any::PathAnyConst)]
pub trait Response: Send + Sync + 'static {}

#[component((<$_:ty, $message:ty>) => (implementation = Respond<$message>))]
pub struct RespondComponent<R: Respond<M>, M: Message> {
    receiver: RespondReceiver<Respondable<M, R::Response>>,
}

#[cfg_attr(feature = "path-any", derive(PathAny))]
#[doc(hidden)] // NOTE: this is an implementation detail, only public because it's in traits
pub struct Respondable<M: Message, R: Response> {
    message: M,
    response_channel: tokio::sync::oneshot::Sender<R>,
}

impl<R: Respond<M>, M: Message> Input for RespondComponent<R, M> {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.respond_receiver::<R, M>().await,
        })
    }
}

impl<R: Respond<M>, M: Message> Component<R> for RespondComponent<R, M> {
    type Value = Respondable<M, R::Response>;
    type UserInput<'s> = <R as Respond<M>>::OnInput<'s>;

    async fn wait(&mut self, _: &R) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        respondable: Self::Value,
        responder: &mut R,
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        let Respondable {
            message,
            response_channel,
        } = respondable;
        let response = responder.on(message, input).send().await;
        let _ = response_channel.send(response);
        Ok(ControlFlow::Continue(()))
    }
}

pub struct Transceiver<R: Respond<M>, M: Message> {
    sender: tokio::sync::mpsc::Sender<Respondable<M, R::Response>>,
}

impl<R: Respond<M>, M: Message> Input for Transceiver<R, M> {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            sender: registry.respond_sender::<R, M>(),
        })
    }
}

impl<R: Respond<M>, M: Message> Transceiver<R, M> {
    pub async fn send(&self, message: M) -> Option<R::Response> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let respondable = Respondable {
            message,
            response_channel: sender,
        };
        let _ = self.sender.send(respondable).await;
        receiver.await.ok()
    }
}

impl<R: Respond<M>, M: Message> Clone for Transceiver<R, M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

#[component(() => ())]
pub struct RegistryComponent {
    receiver: EventReceiver,
}

impl Input for RegistryComponent {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.event_receiver(),
        })
    }
}

impl<L: Listener> Component<L> for RegistryComponent {
    type Value = RegistryEvent;
    type UserInput<'s> = ();

    async fn wait(&mut self, _: &L) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        #[allow(clippy::option_if_let_else)] // I feel like this is easier to read
        match self.receiver.recv().await {
            Some(message) => Ok(ControlFlow::Continue(message)),
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn handle(
        &mut self,
        event: Self::Value,
        listener: &mut L,
        (): Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        Ok(match event {
            RegistryEvent::Stop(ref id) if id == L::const_ident() => ControlFlow::Break(()),
            RegistryEvent::Exit => ControlFlow::Break(()),
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::Get(ref get_request) => {
                get_request.check(listener).await;
                ControlFlow::Continue(())
            }
            _ => ControlFlow::Continue(()),
        })
    }
}

#[allow(private_bounds)]
pub trait ComponentList<L: Listener>: Sealed<L> + Input + Send + Sync {
    fn run(
        &mut self,
        listener: &mut L,
        input: &mut L::Input,
        sender: &EventSender,
    ) -> impl Future<Output = ()> + Send;
}

pub(crate) trait Sealed<L> {}

macro_rules! impl_component_list {
    ($(($T:tt, $C:tt, $c:tt)),*) => {
        #[allow(unused_parens)]
        impl<L, $($T,)* $($C,)*> ComponentList<L> for ((), $($T),*)
        where
            Self: Input,
            $($T: Component<L>,)*
            L: Listener<Components = Self, ComponentCombinations = ($($C),*)>,
            L::Input: 'static
                $(+ for<'s> Subset<$C, SubsetMut<'s> = $T::UserInput<'s>>)*
        {
            async fn run(&mut self, listener: &mut L, input: &mut L::Input, sender: &EventSender) {
                let ((), $($c),*) = self;

                loop {
                    select! { $(
                        res = $c.wait(&listener) => {
                            let Some(val) = sender.handle_error(res, listener.ident()) else { continue; };
                            let ControlFlow::Continue(val) = val else { break; };

                            let input = <<L as Listener>::Input as Subset<$C>>::subset_mut(input);
                            let res = $c.handle(val, listener, input).send().await;

                            let Some(val) = sender.handle_error(res, listener.ident()) else { continue; };
                            if val.is_break() { break; }
                        },
                    )*}
                }
            }
        }

        #[allow(unused_parens)]
        impl<L, $($T,)* $($C,)*> Sealed<L> for ((), $($T),*)
        where
            Self: Input,
            $($T: Component<L>,)*
            L: Listener<Components = Self, ComponentCombinations = ($($C),*)>,
            L::Input: 'static
                $(+ for<'s> Subset<$C, SubsetMut<'s> = $T::UserInput<'s>>)*
        {}
    };
}

all_tuples!(impl_component_list, 1, 16, T, C, c);

pub(crate) struct LoadedListener<L: Listener> {
    listener: L,
    input: L::Input,
    components: L::Components,
    event_sender: EventSender,
}

impl<L: Listener> LoadedListener<L> {
    async fn build(listener: L, registry: &Registry) -> Result<Self, Error> {
        Ok(Self {
            input: L::Input::create(registry, listener.ident()).await?,
            components: L::Components::create(registry, listener.ident()).await?,
            listener,
            event_sender: registry.event_sender(),
        })
    }
}

#[async_trait]
impl<L: Listener> Runner for LoadedListener<L> {
    async fn run(mut self: Box<Self>) {
        let init_input = <L::Input as Subset<L::InitCombination>>::subset_mut(&mut self.input);
        let res = self.listener.init(init_input).send().await;
        self.event_sender.handle_error(res, self.listener.ident());

        <L::Components as ComponentList<L>>::run(
            &mut self.components,
            &mut self.listener,
            &mut self.input,
            &self.event_sender,
        )
        .await;

        let exit_input = <L::Input as Subset<L::ExitCombination>>::subset_mut(&mut self.input);
        let res = self.listener.exit(exit_input).send().await;
        self.event_sender.handle_error(res, self.listener.ident());

        debug!("exit");
    }
}

impl<L: Listener> Identifiable for LoadedListener<L> {
    fn const_ident() -> &'static NamespacedIdRef
    where
        Self: Sized,
    {
        L::const_ident()
    }

    fn ident(&self) -> &'static NamespacedIdRef {
        self.listener.ident()
    }
}

impl<L: Listener> LoadedResource for LoadedListener<L> {
    fn resource_any(&self) -> &dyn PathAny {
        &self.listener
    }

    fn resource(&self) -> &dyn Resource {
        &self.listener
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
        let registry_input = L::RegistryInput::create(registry, L::const_ident()).await?;
        let listener = L::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: L::const_ident().into(),
                error,
            })?;
        let runner = LoadedListener::build(listener, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        L::RegistryInput::constraints().extend(L::Input::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        L::const_ident()
    }
}
