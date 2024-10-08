use anyhow::anyhow;
use futures::pin_mut;
use log::warn;
use log::{debug, error};
use tokio::select;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use crate::resource::Identifiable;
use crate::resource::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::RegistryEvent;
use super::{EventReceiver, EventSender};

const fn then_continue(conditional: bool) -> ControlFlow<()> {
    if conditional {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    }
}

/// A [resource](mod@crate::resource) that has already been loaded into a [`Registry`]
#[allow(clippy::module_name_repetitions)] // without Loaded, it would conflict with Resource
pub trait LoadedResource: Identifiable + Send + Sync {
    /// Optionally provide a [`Runner`] to run in the main loop
    ///
    /// This runner is joined alongside every other runner, if it is given
    ///
    /// # Errors
    ///
    /// - Returns [`Err(Self)`] if this resource doesn't provide a runner
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>>;
    /// Obtain a reference to the original resource within a trait object
    fn resource(&self) -> &dyn ResourceTrait;
    /// Obtain a reference to the original resource within an [`Any`] trait object
    ///
    /// This allows [`Registry::get_as`] to give a cloned version of the resource
    ///
    /// Note: this will be deprecated once trait upcasting coercion stabilizes
    fn resource_any(&self) -> &dyn Any;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: Identifiable + Send {
    async fn run(self: Box<Self>);
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    inner: L,
    input: L::Input,
    receiver: Receiver<M>,
    event_receiver: EventReceiver,
    event_sender: EventSender,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub async fn build(listener: L, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <L::Input as Input<M>>::create(registry, listener.ident()).await?;
        Ok(Self {
            inner: listener,
            input,
            receiver: registry.message_receiver(),
            event_receiver: registry.event_receiver(),
            event_sender: registry.event_sender(),
        })
    }

    fn handle_res(&self, result: anyhow::Result<ControlFlow<()>>) -> ControlFlow<()> {
        match result {
            Ok(flow) => flow,
            Err(error) => {
                self.event_sender.send_error(self.inner.ident(), error);
                ControlFlow::Continue(())
            }
        }
    }

    async fn handle_message(&mut self, message: Result<M, RecvError>) -> ControlFlow<()> {
        let res = self.handle_message_inner(message).await;
        self.handle_res(res)
    }

    async fn handle_message_inner(
        &mut self,
        message: Result<M, RecvError>,
    ) -> anyhow::Result<ControlFlow<()>> {
        match message {
            Ok(message) => {
                self.inner.handle(message, &mut self.input).await?;
                Ok(ControlFlow::Continue(()))
            }
            Err(RecvError::Closed) => Ok(ControlFlow::Break(())),
            Err(RecvError::Lagged(amount)) => {
                Err(anyhow!("Message stream lagged by {amount} messages."))
            }
        }
    }

    async fn handle_meta(&mut self, message: Option<RegistryEvent>) -> ControlFlow<()> {
        let res = self.handle_meta_inner(message).await;
        self.handle_res(res)
    }

    async fn handle_meta_inner(
        &mut self,
        message: Option<RegistryEvent>,
    ) -> anyhow::Result<ControlFlow<()>> {
        match message {
            Some(message) => {
                let to_exit = match message {
                    RegistryEvent::Stop(ref id) if *id == self.ident() => true,
                    RegistryEvent::Exit => true,
                    #[cfg(feature = "runtime-loading")]
                    RegistryEvent::Get(ref get_request) => {
                        get_request.check(&self.inner);
                        false
                    }
                    _ => false,
                };
                self.inner.registry_event(message, &mut self.input).await?;
                Ok(then_continue(!to_exit))
            }
            None => Ok(ControlFlow::Break(())),
        }
    }

    async fn on_exit(&mut self) {
        Self::handle_result(self.inner.exit(&mut self.input).await);
    }

    async fn init(&mut self) {
        Self::handle_result(self.inner.init(&mut self.input).await);
    }

    fn handle_result(result: Result<(), impl Into<anyhow::Error>>) {
        if let Err(err) = result {
            error!("{:?}", err.into());
        }
    }
}

impl<M: Message, L: ListenerTrait<M>> Identifiable for Listener<M, L> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.inner.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        L::const_ident()
    }
}

impl<M: Message, L: ListenerTrait<M>> LoadedResource for Listener<M, L> {
    fn resource(&self) -> &dyn ResourceTrait {
        &self.inner
    }

    fn resource_any(&self) -> &dyn Any {
        &self.inner
    }

    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Ok(self)
    }
}

#[async_trait]
impl<M: Message, L: ListenerTrait<M>> Runner for Listener<M, L> {
    async fn run(mut self: Box<Self>) {
        // send startup message
        self.init().await;

        loop {
            tokio::select! {
                biased;
                meta = self.event_receiver.recv() => {
                    if self.handle_meta(meta).await.is_break() {
                        break;
                    }
                }
                message = self.receiver.recv() => {
                    self.handle_message(message).await;
                },
            }
        }

        self.on_exit().await;

        debug!("'{}' exited", self.ident());
    }
}

pub struct Resource<R: ResourceTrait> {
    resource: R,
}

impl<R: ResourceTrait> Resource<R> {
    pub const fn new(resource: R) -> Self {
        Self { resource }
    }
}

impl<R: ResourceTrait> LoadedResource for Resource<R> {
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Err(self)
    }

    fn resource(&self) -> &dyn ResourceTrait {
        &self.resource
    }

    fn resource_any(&self) -> &dyn Any {
        &self.resource
    }
}

impl<R: ResourceTrait> Identifiable for Resource<R> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.resource.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        R::const_ident()
    }
}

pub struct Watcher<M: Message, W: WatcherTrait<M>> {
    inner: W,
    input: W::Input,
    event_receiver: EventReceiver,
    event_sender: EventSender,
}

impl<M: Message, W: WatcherTrait<M>> Watcher<M, W> {
    pub async fn build(watcher: W, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <W::Input as Input<M>>::create(registry, watcher.ident()).await?;
        Ok(Self {
            inner: watcher,
            input,
            event_receiver: registry.event_receiver(),
            event_sender: registry.event_sender(),
        })
    }

    fn handle_meta(
        message: Option<RegistryEvent>,
        #[cfg(feature = "runtime-loading")] reference: Option<&dyn crate::input::CloneAny>,
    ) -> ControlFlow<()> {
        message.map_or(ControlFlow::Break(()), |message| {
            let to_exit = match message {
                RegistryEvent::Stop(ref id) if *id == W::const_ident() => true,
                RegistryEvent::Exit => true,
                #[cfg(feature = "runtime-loading")]
                RegistryEvent::Get(ref get_request) => {
                    if get_request.id() == W::const_ident() {
                        #[allow(clippy::option_if_let_else)] // doesn't return anything
                        if let Some(reference) = reference {
                            get_request.send(reference.clone_any());
                        } else {
                            warn!("id matched GetRequest, but MaybeGetRef returned None");
                            get_request.cancel();
                        }
                    }
                    false
                }
                _ => false,
            };
            then_continue(!to_exit)
        })
    }
}

impl<M: Message, W: WatcherTrait<M>> LoadedResource for Watcher<M, W> {
    fn resource(&self) -> &dyn ResourceTrait {
        &self.inner
    }

    fn resource_any(&self) -> &dyn Any {
        &self.inner
    }

    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Ok(self)
    }
}

#[async_trait]
impl<M: Message, W: WatcherTrait<M>> Runner for Watcher<M, W> {
    async fn run(mut self: Box<Self>) {
        // precompute get requests before running the watcher
        #[cfg(feature = "runtime-loading")]
        let reference = self.inner.maybe_get_ref();

        {
            let mut meta_receiver = self.event_receiver.resubscribe();
            let run = self.inner.run(&mut self.input);
            pin_mut!(run);

            loop {
                select! {
                    biased;
                    message = meta_receiver.recv() => {
                        let res = Self::handle_meta(
                            message,
                            #[cfg(feature = "runtime-loading")] reference.as_deref(),
                        );
                        if res.is_break() {
                            break;
                        }
                    }
                    res = &mut run => {
                        if let Err(err) = res {
                            self.event_sender.send_error(W::const_ident(), err);
                        }
                        break;
                    },
                }
            }
        }

        let res = self.inner.exit(&mut self.input).await;
        if let Err(err) = res {
            self.event_sender.send_error(W::const_ident(), err);
        }

        debug!("'{}' exited", W::const_ident());
    }
}

impl<M: Message, W: WatcherTrait<M>> Identifiable for Watcher<M, W> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.inner.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        W::const_ident()
    }
}
