use std::any::Any;

use futures::pin_mut;
use log::{debug, error};
use tokio::select;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use crate::resource::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::Exit;
use super::RegistryMessage;

const fn then_continue(conditional: bool) -> ControlFlow<()> {
    if conditional {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    }
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
    /// This allows [`Registry::get`] to give a cloned version of the resource
    fn resource(&self) -> &dyn Any;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: Send {
    async fn run(self: Box<Self>);
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    inner: L,
    input: <L::Input as Input<M>>::Output,
    exit_signal: Receiver<Exit>,
    receiver: Receiver<M>,
    meta_receiver: Receiver<RegistryMessage>,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub fn build(listener: L, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <L::Input as Input<M>>::create(registry)?;
        Ok(Self {
            inner: listener,
            input,
            exit_signal: registry.exit_signal(),
            receiver: registry.subscribe(),
            meta_receiver: registry.meta_receiver(),
        })
    }

    fn handle_recv_error(err: RecvError) -> ControlFlow<()> {
        let closed = matches!(err, RecvError::Closed);
        Self::handle_err(anyhow::Error::from(err).context("failed to recieve message"));
        then_continue(!closed)
    }

    async fn handle(&mut self, message: Result<M, RecvError>) -> ControlFlow<()> {
        let message = match message {
            Ok(message) => message,
            Err(err) => return Self::handle_recv_error(err),
        };
        if let Err(err) = self.inner.handle(message, &mut self.input).await {
            Self::handle_err(err);
        }
        ControlFlow::Continue(())
    }

    async fn handle_meta(
        &mut self,
        message: Result<RegistryMessage, RecvError>,
    ) -> ControlFlow<()> {
        let message = match message {
            Ok(message) => message,
            Err(err) => return Self::handle_recv_error(err),
        };
        let to_exit = matches!(message, RegistryMessage::Stop { ref id } if *id == self.ident());
        if let Err(err) = self.inner.meta(message, &mut self.input).await {
            Self::handle_err(err);
        }
        then_continue(!to_exit)
    }

    async fn on_exit(&mut self) {
        if let Err(err) = self.inner.exit(&mut self.input).await {
            Self::handle_err(err);
        }
    }

    async fn init(&mut self) {
        if let Err(err) = self.inner.init(&mut self.input).await {
            Self::handle_err(err);
        }
    }

    fn handle_err(err: impl Into<anyhow::Error>) {
        error!("{:?}", err.into());
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

        // listen to the exit signal, calling it out here to avoid remaking the future
        let mut exit_signal = self.exit_signal.resubscribe();
        let exit_signal = exit_signal.recv();
        pin_mut!(exit_signal);

        loop {
            tokio::select! {
                message = self.receiver.recv() => {
                    self.handle(message).await;
                },
                meta = self.meta_receiver.recv() => {
                    if self.handle_meta(meta).await.is_break() {
                        break;
                    }
                }
                _ = &mut exit_signal => {
                    break;
                }
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
    exit_signal: Receiver<Exit>,
}

impl<M: Message, W: WatcherTrait<M>> Watcher<M, W> {
    pub fn build(watcher: W, registry: &Registry<M>) -> Result<Self, Error> {
        let input = <W::Input as Input<M>>::create(registry)?;
        let exit_signal = registry.exit_signal();
        Ok(Self {
            inner: watcher,
            exit_signal,
            input,
        })
    }

    fn handle_result(&self, res: anyhow::Result<()>) {
        if let Err(_err) = res {
            todo!();
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
        select! {
            res = self.inner.run(&mut self.input) => {
                self.handle_result(res);
            },
            _ = self.exit_signal.recv() => {
                let res = self.inner.exit(&mut self.input).await;
                self.handle_result(res);
            }
        }
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
