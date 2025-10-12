#![doc = include_str!("listener_docs.md")]

use std::marker::PhantomData;

use crate::{
    Message,
    app::{
        LoadedResource, Loader, Runner,
        channels::{self, EventReceiver, EventSender, Taker},
        event::ComponentIdRef,
    },
    input::subset::Subset,
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
    type OnInput<'s>: Send;

    fn on(
        &mut self,
        message: M,
        input: <Self as Listen<M>>::OnInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[component((<$_:ty, $message:ty>) => (implementation = Listen<$message>))]
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
        Constraints::empty().after_message(M::const_ident())
    }
}

impl<L: Listen<M>, M: ListenMessage> Component<L> for ListenComponent<L, M> {
    type Value = M;
    type UserInput<'s> = <L as Listen<M>>::OnInput<'s>;

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
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        listener.on(message, input).send().await?;
        Ok(ControlFlow::Continue(()))
    }
}

pub trait ListenToRegistry: Listener {
    type OnInput<'s>: Send;

    fn on(
        &mut self,
        message: RegistryEvent,
        input: <Self as ListenToRegistry>::OnInput<'_>,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

#[component((<$_:ty>) => (implementation = ListenToRegistry))]
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
    type UserInput<'s> = <L as ListenToRegistry>::OnInput<'s>;

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
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        let should_break = listener.on(message, input).send().await?;
        if should_break {
            Ok(ControlFlow::Break(()))
        } else {
            Ok(ControlFlow::Continue(()))
        }
    }
}

pub trait OnLoad<R: Resource>: Listener {
    type OnLoadInput<'s>: Send;

    fn on_load(
        &mut self,
        input: <Self as OnLoad<R>>::OnLoadInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[component((<$_:ty, $message:ty>) => (implementation = OnLoad<$message>))]
pub struct OnLoadComponent<L: OnLoad<R>, R: Resource> {
    receiver: EventReceiver,
    should_init: bool,
    listener: PhantomData<(L, R)>,
}

impl<L: OnLoad<R>, R: Resource> Input for OnLoadComponent<L, R> {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        let mut receiver = registry.event_receiver();
        receiver.mark_seen();

        let should_init = registry.loaded_resources().await.contains(R::const_ident());

        Ok(Self {
            receiver,
            should_init,
            listener: PhantomData,
        })
    }
}

impl<L: OnLoad<R>, R: Resource> Component<L> for OnLoadComponent<L, R> {
    type Value = Option<RegistryEvent>;
    type UserInput<'s> = <L as OnLoad<R>>::OnLoadInput<'s>;

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
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        if message.is_none()
            || matches!(&message, Some(RegistryEvent::Running(id)) if *id == *R::const_ident())
        {
            listener.on_load(input).send().await?;
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[component((<$_:ty, $message:ty>) => (implementation = Take<$message>))]
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
        Constraints::empty().after_message(M::const_ident())
    }
}

pub trait Take<M: Message>: Listener {
    type OnInput<'s>: Send;
    fn on(
        &mut self,
        message: M,
        input: <Self as Take<M>>::OnInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl<R: Take<M>, M: Message> Component<R> for TakeComponent<R, M> {
    type Value = M;
    type UserInput<'s> = <R as Take<M>>::OnInput<'s>;

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
        input: Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        responder.on(message, input).send().await?;
        Ok(ControlFlow::Continue(()))
    }

    fn channels() -> Channels {
        Channels::new().with_take_channel::<M>()
    }
}

#[component(() => ())]
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
    type UserInput<'s> = ();

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
        (): Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        Ok(match event {
            RegistryEvent::Stop(ref id) if id == L::const_ident() => ControlFlow::Break(()),
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

#[allow(private_bounds)]
pub trait ComponentList<L: Listener>: Sealed<L> + Input + Send + Sync {
    fn run(
        &mut self,
        listener: &mut L,
        input: &mut L::Input,
        sender: &EventSender,
    ) -> impl Future<Output = ()> + Send;

    fn channels() -> Channels;
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

            fn channels() -> Channels {
                Channels::new()
                    $(.combine_with($T::channels()))*
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
    pub(crate) async fn build(listener: L, registry: &Registry) -> Result<Self, Error> {
        let ident = ComponentIdRef::Resource(listener.ident());
        Ok(Self {
            input: L::Input::create(registry, ident).await?,
            components: L::Components::create(registry, ident).await?,
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
        let ident = ComponentIdRef::Resource(L::const_ident());
        let registry_input = L::RegistryInput::create(registry, ident).await?;
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
        L::RegistryInput::constraints()
            .extend(L::Input::constraints())
            .extend(L::Components::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        L::const_ident()
    }

    fn channels(&self) -> Channels {
        L::Components::channels()
    }
}
