use std::marker::PhantomData;

use crate::prelude::*;

use crate::resource::Listener as ListenerTrait;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::runner::Runner;

pub trait Loader<M: Message>: ResourceTrait + Send {
    fn load(&mut self, registry: &mut Registry<M>) -> Result<(), Error>;
    fn is_finished(&self) -> bool;

    fn constraints(&self) -> Constraints;
}

pub struct Listener<M: Message, L: ListenerTrait<M>> {
    listener: Option<L>,
    message: PhantomData<M>,
}

impl<M: Message, L: ListenerTrait<M>> Listener<M, L> {
    pub const fn new(listener: L) -> Self {
        Self {
            listener: Some(listener),
            message: PhantomData,
        }
    }
}

impl<M: Message, L: ListenerTrait<M>> Loader<M> for Listener<M, L> {
    fn load(&mut self, registry: &mut Registry<M>) -> Result<(), Error> {
        if let Some(listener) = self.listener.take() {
            registry.add_listener(listener)?;
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.listener.is_none()
    }

    fn constraints(&self) -> Constraints {
        self.listener
            .as_ref()
            .map(ListenerTrait::constraints)
            .unwrap_or_default()
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
    resource: Option<R>,
}

impl<R: ResourceTrait> Resource<R> {
    pub const fn new(resource: R) -> Self {
        Self {
            resource: Some(resource),
        }
    }
}

impl<M: Message, R: ResourceTrait> Loader<M> for Resource<R> {
    fn load(&mut self, registry: &mut Registry<M>) -> Result<(), Error> {
        if let Some(resource) = self.resource.take() {
            registry.add_resource(resource);
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.resource.is_none()
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
    watcher: Option<W>,
    message: PhantomData<M>,
}

impl<M: Message, W: WatcherTrait<M>> Watcher<M, W> {
    pub const fn new(watcher: W) -> Self {
        Self {
            watcher: Some(watcher),
            message: PhantomData,
        }
    }
}

impl<M: Message, W: WatcherTrait<M>> Loader<M> for Watcher<M, W> {
    fn load(&mut self, registry: &mut Registry<M>) -> Result<(), Error> {
        if let Some(watcher) = self.watcher.take() {
            registry.add_watcher(watcher)?;
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.watcher.is_none()
    }

    fn constraints(&self) -> Constraints {
        self.watcher
            .as_ref()
            .map(WatcherTrait::constraints)
            .unwrap_or_default()
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
    R: Runner<M>,
    F: FnOnce(I::Output, &mut Registry<M>) -> Result<R, Error> + Send + 'static,
{
    func: Option<F>,
    phantom_data: PhantomData<(I, R, M)>,
}

impl<I, R, M, F> With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: Runner<M>,
    F: FnOnce(I::Output, &mut Registry<M>) -> Result<R, Error> + Send + 'static,
{
    pub const fn new(func: F) -> Self {
        Self {
            func: Some(func),
            phantom_data: PhantomData,
        }
    }
}

impl<I, M, R, F> Loader<M> for With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: Runner<M>,
    F: FnOnce(I::Output, &mut Registry<M>) -> Result<R, Error> + Send + 'static,
{
    fn load(&mut self, registry: &mut Registry<M>) -> Result<(), Error> {
        if let Some(func) = self.func.take() {
            let input = I::create(registry)?;
            let resource = func(input, registry)?;
            registry.add(resource);
        }
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.func.is_none()
    }

    fn constraints(&self) -> Constraints {
        I::constraints()
    }
}

impl<I, M, R, F> ResourceTrait for With<I, M, R, F>
where
    I: Input<M>,
    M: Message,
    R: Runner<M>,
    F: FnOnce(I::Output, &mut Registry<M>) -> Result<R, Error> + Send + 'static,
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
