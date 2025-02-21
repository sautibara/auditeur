#![doc = include_str!("listener_docs.md")]

use std::marker::PhantomData;

use crate::{
    app::{EventReceiver, EventSender, LoadedResource, Loader, Runner},
    input::subset::Subset,
    prelude::*,
};

use bevy_utils_proc_macros::all_tuples;
use futures::Future;
use tokio::select;

// TODO: remove this once #100013 is stabilized
use send_future::SendFuture;

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

pub trait Runnable {
    type InitInput<'s>: Send;
    type ExitInput<'s>: Send;

    fn init(
        &mut self,
        input: Self::InitInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn exit(
        &mut self,
        input: Self::ExitInput<'_>,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[tokio::test]
    async fn listener_test() {
        use crate::prelude::*;
        // TODO: add to prelude
        use crate::resource::listener::*;
        use crate::Runnable;

        /// A message that tells [`ExitListener`] to exit the program
        #[derive(Clone, Debug)]
        #[message("test:exit")]
        struct Exit;

        /// A [`Listener`] that exits the program once it receives an [`Exit`] message
        #[listener(id = "test:exit_listener", listen(Exit(input = Stopper)))]
        #[derive(Runnable, Default, Create)]
        struct ExitListener;

        #[has_input]
        impl Listen<Exit> for ExitListener {
            async fn on(&mut self, _: Exit, stopper: &mut Stopper) -> Result<()> {
                // [`Stopper`] can be used to stop individual resources or the entire program
                stopper.exit();
                Ok(())
            }
        }

        // Create a new App with only an ExitListener
        let app =
            App::new().with_bundle(new_bundle!("test:exit").with_listener_new::<ExitListener>(()));
        // Build it into a Registry so that we can get the message sender before running it
        // Usually, it's simpler just to call app.run(_)
        let registry = app.build(1024).await.expect("failed to build app");

        let sender = registry.message_sender::<Exit>();
        // Start the registry running in the background now that we got the message sender
        let running_registry = tokio::task::spawn(registry.run());

        // Send the exit message in the registry, telling the ExitListener to exit the program
        // Usually, resources send messages to other resources, but the main function can too
        sender.send(Exit).expect("failed to send message");

        // Make sure the app exits within an acceptable time frame
        tokio::time::timeout(Duration::from_secs(1), running_registry)
            .await
            .expect("ExitListener should exit immediately after the Exit message is sent")
            .expect("App should run to completion");
    }
}
