use std::any::Any;

use anyhow::anyhow;
use futures::pin_mut;
use log::{debug, error};
use tokio::select;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use crate::resource::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::EventReceiver;
use super::RegistryEvent;

const fn then_continue(conditional: bool) -> ControlFlow<()> {
    if conditional {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    }
}

fn handle_recv_error<T>(
    err: RecvError,
    handle_err: impl FnOnce(anyhow::Result<T>),
) -> ControlFlow<()> {
    let closed = matches!(err, RecvError::Closed);
    handle_err(Err(
        anyhow::Error::from(err).context("failed to recieve message")
    ));
    then_continue(!closed)
}

/// A [resource](mod@crate::resource) that has already been loaded into a [`Registry`]
#[allow(clippy::module_name_repetitions)] // without Loaded, it would conflict with Resource
pub trait LoadedResource: ResourceTrait + Send {
    /// Optionally provide a [`Runner`] to run in the main loop
    ///
    /// This runner is joined alongside every other runner, if it is given
    fn runner(self: Box<Self>) -> Option<Box<dyn Runner>>;
    /// Obtain a reference to the original resource, wrapped in an [`Any`]
    ///
    /// This allows [`Registry::get_as`] to give a cloned version of the resource
    fn resource(&self) -> &dyn Any;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: ResourceTrait + Send {
    async fn run(self: Box<Self>);
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    inner: L,
    input: <L::Input as Input<M>>::Output,
    receiver: Receiver<M>,
    event_receiver: EventReceiver,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub fn build(listener: L, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <L::Input as Input<M>>::create(registry, listener.ident())?;
        Ok(Self {
            inner: listener,
            input,
            receiver: registry.message_reciever(),
            event_receiver: registry.event_receiver(),
        })
    }

    async fn handle(&mut self, message: Result<M, RecvError>) -> ControlFlow<()> {
        let message = match message {
            Ok(message) => message,
            Err(err) => return handle_recv_error(err, Self::handle_result),
        };
        Self::handle_result(self.inner.handle(message, &mut self.input).await);
        ControlFlow::Continue(())
    }

    async fn handle_meta(&mut self, message: Option<RegistryEvent>) -> ControlFlow<()> {
        let Some(message) = message else {
            Self::handle_result(Err(anyhow!("registry event channel closed")));
            return ControlFlow::Break(());
        };
        let to_exit = matches!(message, RegistryEvent::Stop { ref id } if *id == self.ident())
            || matches!(message, RegistryEvent::Exit);
        Self::handle_result(self.inner.registry_event(message, &mut self.input).await);
        then_continue(!to_exit)
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

impl<M: Message, L: ListenerTrait<M>> ResourceTrait for Listener<M, L> {
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
    fn resource(&self) -> &dyn Any {
        &self.inner
    }

    fn runner(self: Box<Self>) -> Option<Box<dyn Runner>> {
        Some(self)
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
                    self.handle(message).await;
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
    fn runner(self: Box<Self>) -> Option<Box<dyn Runner>> {
        None
    }

    fn resource(&self) -> &dyn Any {
        &self.resource
    }
}

impl<R: ResourceTrait> ResourceTrait for Resource<R> {
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
    input: <W::Input as Input<M>>::Output,
    event_receiver: EventReceiver,
}

impl<M: Message, W: WatcherTrait<M>> Watcher<M, W> {
    pub fn build(watcher: W, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <W::Input as Input<M>>::create(registry, watcher.ident())?;
        Ok(Self {
            inner: watcher,
            input,
            event_receiver: registry.event_receiver(),
        })
    }

    fn handle_meta(message: Option<RegistryEvent>) -> ControlFlow<()> {
        let Some(message) = message else {
            Self::handle_result(Err(anyhow!("registry event channel closed")));
            return ControlFlow::Break(());
        };
        let to_exit = matches!(message, RegistryEvent::Stop { ref id } if *id == W::const_ident())
            || matches!(message, RegistryEvent::Exit);
        then_continue(!to_exit)
    }

    fn handle_result(res: Result<(), impl Into<anyhow::Error>>) {
        if let Err(err) = res {
            error!("{:?}", err.into());
        }
    }
}

impl<M: Message, W: WatcherTrait<M>> LoadedResource for Watcher<M, W> {
    fn resource(&self) -> &dyn Any {
        &self.inner
    }

    fn runner(self: Box<Self>) -> Option<Box<dyn Runner>> {
        Some(self)
    }
}

#[async_trait]
impl<M: Message, W: WatcherTrait<M>> Runner for Watcher<M, W> {
    async fn run(mut self: Box<Self>) {
        let mut meta_receiver = self.event_receiver.resubscribe();
        let meta_loop = async move {
            loop {
                let message = meta_receiver.recv().await;
                let res = Self::handle_meta(message);
                if res.is_break() {
                    break;
                }
            }
        };
        pin_mut!(meta_loop);
        select! {
            biased;
            () = &mut meta_loop => { }
            res = self.inner.run(&mut self.input) => {
                Self::handle_result(res);
            },
        }

        debug!("'{}' exited", self.ident());
    }
}

impl<M: Message, W: WatcherTrait<M>> ResourceTrait for Watcher<M, W> {
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
