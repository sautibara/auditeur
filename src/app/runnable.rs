use std::any::Any;

use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use crate::listener::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;

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
    receiver: Receiver<M>,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub fn build(listener: L, registry: &mut Registry<M>) -> Result<Self, Error> {
        let input = <L::Input as Input<M>>::create(registry)?;
        let receiver = registry.subscribe();
        Ok(Self {
            inner: listener,
            input,
            receiver,
        })
    }

    async fn handle(&mut self, message: M) -> ControlFlow<()> {
        match self.inner.handle(message, &mut self.input).await {
            Ok(flow) => flow,
            Err(_err) => todo!(),
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

#[async_trait]
impl<M: Message, L: ListenerTrait<M>> Runnable<M> for Listener<M, L> {
    fn inner(&self) -> &dyn Any {
        &self.inner
    }

    async fn run(&mut self) {
        // send startup message
        if self.handle(M::startup()).await.is_break() {
            return;
        }
        while let Ok(message) = self.receiver.recv().await {
            let message_is_exit = message.is_exit();
            if self.handle(message).await.is_break() || message_is_exit {
                break;
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
