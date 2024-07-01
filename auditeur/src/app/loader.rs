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
pub trait Loader<M: Message>: ResourceTrait + Send {
    /// Consume the loader and create a [`LoadedResource`], resolving inputs using `registry`
    ///
    /// # Errors
    ///
    /// - If the inputs return any errors
    ///   - If any resources are not found
    fn load(self: Box<Self>, registry: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error>;
    /// Return a list of [`Constraints`] that must be satisfied before loading
    fn constraints(&self) -> Constraints;
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
}

impl<M: Message, L: ListenerTrait<M>> ResourceTrait for Listener<M, L> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        L::const_ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        L::const_ident()
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

impl<M: Message, R: ResourceTrait> Loader<M> for Resource<R> {
    fn load(self: Box<Self>, _: &Registry<M>) -> Result<Box<dyn LoadedResource>, Error> {
        Ok(Box::new(loaded::Resource::new(self.resource)))
    }

    fn constraints(&self) -> Constraints {
        Constraints::empty()
    }
}

impl<R: ResourceTrait> ResourceTrait for Resource<R> {
    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        R::const_ident()
    }

    fn ident(&self) -> NamespacedIdRef<'static> {
        R::const_ident()
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
}

impl<M: Message, W: WatcherTrait<M>> ResourceTrait for Watcher<M, W> {
    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        W::const_ident()
    }

    fn ident(&self) -> NamespacedIdRef<'static> {
        W::const_ident()
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
        let input = I::create(registry)?;
        let resource = (self.func)(input, registry)?;
        Ok(Box::new(resource))
    }

    fn constraints(&self) -> Constraints {
        I::constraints()
    }
}

impl<I, M, R, F> ResourceTrait for With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: LoadedResource,
    F: FnOnce(I::Output, &Registry<M>) -> Result<R, Error> + Send + 'static,
{
    fn ident(&self) -> NamespacedIdRef<'static> {
        R::const_ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        R::const_ident()
    }
}
