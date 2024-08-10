use std::marker::PhantomData;

use crate::prelude::*;

use crate::resource::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::loaded;
use super::loaded::LoadedResource;

/// An object that can be loaded with a [`Registry`] to return a finished [`LoadedResource`]
///
/// This trait and its delayed execution is the thing that allows for the "trait magic" of
/// [inputs](Input). When loading, inputs are taken from the [Registry]. Resources are ensured to
/// already be loaded through the [`Constraints`] of each input, which is why delayed execution is
/// necessary.
#[async_trait]
pub trait Loader<M: Message>: Send {
    /// Consume the loader and create a [`LoadedResource`], resolving inputs using `registry`
    ///
    /// # Errors
    ///
    /// - If the inputs return any errors
    ///   - If any resources are not found
    async fn load(
        self: Box<Self>,
        registry: &Registry<M>,
    ) -> Result<Box<dyn LoadedResource>, Error>;
    /// Return a list of [`Constraints`] that must be satisfied before loading
    fn constraints(&self) -> Constraints;
    /// Return a reference to the contained [resource](mod@crate::resource)'s [`NamespacedId`]
    fn ident(&self) -> NamespacedIdRef<'_>;
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    listener: L,
    message: PhantomData<M>,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub const fn new(listener: L) -> Self {
        Self {
            listener,
            message: PhantomData,
        }
    }
}

#[async_trait]
impl<M: Message, L: ListenerTrait<M>> Loader<M> for Listener<M, L> {
    async fn load(
        self: Box<Self>,
        registry: &Registry<M>,
    ) -> Result<Box<dyn LoadedResource>, Error> {
        let runner = loaded::Listener::build(self.listener, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        self.listener.constraints()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        self.listener.ident()
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
impl<M: Message, R: ResourceTrait> Loader<M> for Resource<R> {
    async fn load(
        self: Box<Self>,
        registry: &Registry<M>,
    ) -> Result<Box<dyn LoadedResource>, Error> {
        Ok(Box::new(loaded::Resource::new(self.resource, registry)))
    }

    fn constraints(&self) -> Constraints {
        Constraints::empty()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        self.resource.ident()
    }
}

pub struct Watcher<M: Message, W: WatcherTrait<M>> {
    watcher: W,
    message: PhantomData<M>,
}

impl<M: Message, W: WatcherTrait<M>> Watcher<M, W> {
    pub const fn new(watcher: W) -> Self {
        Self {
            watcher,
            message: PhantomData,
        }
    }
}

#[async_trait]
impl<M: Message, W: WatcherTrait<M>> Loader<M> for Watcher<M, W> {
    async fn load(
        self: Box<Self>,
        registry: &Registry<M>,
    ) -> Result<Box<dyn LoadedResource>, Error> {
        let runner = loaded::Watcher::build(self.watcher, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        self.watcher.constraints()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        self.watcher.ident()
    }
}
