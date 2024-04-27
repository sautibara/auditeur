use std::any::Any;

use futures::pin_mut;
use tokio::select;
use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use crate::listener::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::watcher::Watcher as WatcherTrait;

#[async_trait]
pub trait Runnable<M: Message>: ResourceTrait + Send {
    async fn run(&mut self);

    fn is_runnable(&self) -> bool {
        true
    }

    fn inner(&self) -> &dyn Any;
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    inner: L,
    input: <L::Input as Input<M>>::Output,
    exit_signal: Receiver<Exit>,
    receiver: Receiver<M>,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub fn build(listener: L, registry: &mut Registry<M>) -> Result<Self, Error> {
        let input = <L::Input as Input<M>>::create(registry)?;
        let receiver = registry.subscribe();
        let exit_signal = registry.exit_signal();
        Ok(Self {
            inner: listener,
            input,
            exit_signal,
            receiver,
        })
    }

    async fn handle(&mut self, message: M) -> ControlFlow<()> {
        match self.inner.handle(message, &mut self.input).await {
            Ok(flow) => flow,
            Err(err) => {
                self.handle_err(err);
                ControlFlow::Continue(())
            }
        }
    }

    async fn on_exit(&mut self) {
        if let Err(err) = self.inner.exit(&mut self.input).await {
            self.handle_err(err);
        }
    }

    async fn startup(&mut self) -> ControlFlow<()> {
        match self.inner.startup(&mut self.input).await {
            Ok(flow) => flow,
            Err(err) => {
                self.handle_err(err);
                ControlFlow::Continue(())
            }
        }
    }

    fn handle_err(&self, err: anyhow::Error) {
        drop(err);
        todo!()
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

#[async_trait]
impl<M: Message, L: ListenerTrait<M>> Runnable<M> for Listener<M, L> {
    fn inner(&self) -> &dyn Any {
        &self.inner
    }

    async fn run(&mut self) {
        // send startup message
        if self.startup().await.is_break() {
            return;
        }

        // listen to the exit signal, calling it out here to avoid remaking the future
        let mut exit_signal = self.exit_signal.resubscribe();
        let exit_signal = exit_signal.recv();
        pin_mut!(exit_signal);

        loop {
            tokio::select! {
                message = self.receiver.recv() => {
                    let Ok(message) = message else { break; };
                    if self.handle(message).await.is_break() {
                        break;
                    }
                },
                _ = &mut exit_signal => {
                    self.on_exit().await;
                    break;
                }
            }
        }
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

#[async_trait]
impl<M: Message, R: ResourceTrait> Runnable<M> for Resource<R> {
    fn is_runnable(&self) -> bool {
        false
    }

    async fn run(&mut self) {}

    fn inner(&self) -> &dyn Any {
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
    pub fn build(watcher: W, registry: &mut Registry<M>) -> Result<Self, Error> {
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

#[async_trait]
impl<M: Message, W: WatcherTrait<M>> Runnable<M> for Watcher<M, W> {
    fn inner(&self) -> &dyn Any {
        &self.inner
    }

    async fn run(&mut self) {
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
