use std::marker::PhantomData;

use crate::prelude::*;

use crate::resource::Listener as ListenerTrait;
use crate::resource::Watcher as WatcherTrait;

use super::loaded;
use super::loaded::LoadedResource;

/// An object that can be loaded with a [`Registry`] to return a finished [`LoadedResource`]
///
/// This trait and its delayed execution is the thing that allows for the "trait magic" of
/// [inputs](Input). When loading, inputs are taken from the [Registry]. Resources are ensured to
/// already be loaded through the [`Constraints`] of each input, which is why delayed execution is
/// necessary.
pub trait Loader<M: Message>: Send {
    /// Consume the loader and create a [`LoadedResource`], resolving inputs using `registry`
    ///
    /// # Errors
    ///
    /// - If the inputs return any errors
    ///   - If any resources are not found
    fn load(self: Box<Self>, registry: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error>;
    /// Return a list of [`Constraints`] that must be satisfied before loading
    fn constraints(&self) -> Constraints;
    /// Return a reference to the contained [resource](crate::resource)'s [`NamespacedId`]
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

impl<M: Message, L: ListenerTrait<M>> Loader<M> for Listener<M, L> {
    fn load(self: Box<Self>, registry: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error> {
        let runner = loaded::Listener::build(self.listener, registry)?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        self.listener.constraints()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        self.listener.ident()
    }
}

pub struct Resource<R: DynamicResource> {
    resource: R,
}

impl<R: DynamicResource> Resource<R> {
    pub const fn new(resource: R) -> Self {
        Self { resource }
    }
}

impl<M: Message, R: DynamicResource> Loader<M> for Resource<R> {
    fn load(self: Box<Self>, _: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error> {
        Ok(Box::new(loaded::Resource::new(self.resource)))
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

impl<M: Message, W: WatcherTrait<M>> Loader<M> for Watcher<M, W> {
    fn load(self: Box<Self>, registry: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error> {
        let runner = loaded::Watcher::build(self.watcher, registry)?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        self.watcher.constraints()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        self.watcher.ident()
    }
}

pub struct With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: LoadedResource,
    F: FnOnce(I::Output, &Registry<M>) -> Result<R, Error> + Send + 'static,
{
    func: F,
    phantom_data: PhantomData<(I, R, M)>,
}

impl<I, R, M, F> With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: LoadedResource,
    F: FnOnce(I::Output, &Registry<M>) -> Result<R, Error> + Send + 'static,
{
    pub const fn new(func: F) -> Self {
        Self {
            func,
            phantom_data: PhantomData,
        }
    }
}

impl<I, M, R, F> Loader<M> for With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: LoadedResource,
    F: FnOnce(I::Output, &Registry<M>) -> Result<R, Error> + Send + 'static,
{
    fn load(self: Box<Self>, registry: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error> {
        let input = I::create(registry, R::const_ident())?;
        let resource = (self.func)(input, registry)?;
        Ok(Box::new(resource))
    }

    fn constraints(&self) -> Constraints {
        I::constraints()
    }

    fn ident(&self) -> NamespacedIdRef<'_> {
        R::const_ident()
    }
}
