use std::collections::HashMap;

use tokio::sync::broadcast::Receiver;

use crate::{prelude::*, Exiter};

use self::constraints::Constraint;

pub mod constraints;
mod loaded;
mod loader;

pub use loaded::LoadedResource;
pub use loaded::Runner;
pub use loader::Loader;

pub struct App<M> {
    sender: Sender<M>,
    exit_handle: Exiter,
    loaders: HashMap<NamespacedId, Option<Box<dyn Loader<M>>>>,
}

impl<M: Message> App<M> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let sender = Sender::new(capacity);
        let exit_handle = Exiter::new();
        Self {
            sender,
            exit_handle,
            loaders: HashMap::new(),
        }
    }

    #[must_use]
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> Receiver<M> {
        self.sender.subscribe()
    }

    #[must_use]
    pub fn exiter(&self) -> Exiter {
        self.exit_handle.clone()
    }

    #[must_use]
    pub fn exiter_subscribe(&self) -> Receiver<Exit> {
        self.exit_handle.subscribe()
    }

    pub fn add<L: Loader<M> + 'static>(&mut self, loader: L) -> &mut Self {
        self.loaders
            .insert(L::const_ident().into(), Some(Box::new(loader)));
        self
    }

    pub fn add_listener<L: Listener<M>>(&mut self, listener: L) -> &mut Self {
        self.add(loader::Listener::new(listener))
    }

    pub fn add_listener_with<I: Input<M>, L: Listener<M>>(
        &mut self,
        func: impl FnOnce(I::Output) -> L + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, loaded::Listener<M, L>, _>::new(
            |output, registry| loaded::Listener::build(func(output), registry),
        ))
    }

    pub fn add_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.add(loader::Resource::new(resource))
    }

    pub fn add_resource_with<I: Input<M>, R: Resource>(
        &mut self,
        func: impl FnOnce(I::Output) -> R + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, loaded::Resource<R>, _>::new(
            |output, _| Ok(loaded::Resource::new(func(output))),
        ))
    }

    pub fn add_watcher<W: Watcher<M>>(&mut self, watcher: W) -> &mut Self {
        self.add(loader::Watcher::new(watcher))
    }

    pub fn add_watcher_with<I: Input<M>, W: Watcher<M>>(
        &mut self,
        func: impl FnOnce(I::Output) -> W + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, loaded::Watcher<M, W>, _>::new(
            |output, registry| loaded::Watcher::build(func(output), registry),
        ))
    }

    pub fn add_plugin<P: Plugin<M> + ?Sized>(&mut self, plugin: &P) -> &mut Self {
        plugin.apply(self);
        self
    }

    /// Listen to all messages until [an exit message](Exiter) is sent
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency was not found
    pub async fn run(mut self) -> Result<(), Error> {
        let mut registry = Registry::new(self.sender.clone(), self.exit_handle.clone());
        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for id in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve(&id, &mut registry)?;
        }
        registry.run().await;
        Ok(())
    }

    fn resolve(&mut self, id: &NamespacedId, registry: &mut Registry<M>) -> Result<(), Error> {
        let loader = self
            .loaders
            .get_mut(id)
            .ok_or_else(|| Error::NotFound { id: id.clone() })?;

        // if it's already finished, then early return
        let Some(loader) = loader.take() else {
            return Ok(());
        };

        // if there are constraints, we must load them first
        for Constraint::After { id } in loader.constraints() {
            self.resolve(&id, registry)?;
        }

        // then we can finally load the loader
        let runnable = loader.load(registry)?;
        registry.add_boxed(runnable);
        Ok(())
    }

    /// Clone the app with the same message channel and exit handle, but with a new set of
    /// resources
    #[must_use]
    pub fn remake(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            exit_handle: self.exit_handle.clone(),
            loaders: HashMap::new(),
        }
    }

    pub fn into_loaders(self) -> impl Iterator<Item = Box<dyn Loader<M>>> {
        self.loaders.into_values().flatten()
    }
}

impl<M: Message> Extend<Box<dyn Loader<M>>> for App<M> {
    fn extend<T: IntoIterator<Item = Box<dyn Loader<M>>>>(&mut self, iter: T) {
        self.loaders.extend(
            iter.into_iter()
                .map(|loader| (loader.ident().into_owned(), Some(loader))),
        );
    }
}

pub struct NotFound;

pub struct Registry<M: Message> {
    sender: Sender<M>,
    exit_handle: Exiter,
    resources: HashMap<NamespacedId, Box<dyn LoadedResource>>,
}

impl<M: Message> Registry<M> {
    fn new(sender: Sender<M>, exit_handle: Exiter) -> Self {
        Self {
            sender,
            exit_handle,
            resources: HashMap::new(),
        }
    }

    #[must_use]
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> Receiver<M> {
        self.sender.subscribe()
    }

    #[must_use]
    pub fn exiter(&self) -> Exiter {
        self.exit_handle.clone()
    }

    #[must_use]
    pub fn exit_signal(&self) -> Receiver<Exit> {
        self.exit_handle.subscribe()
    }

    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<&R> {
        let id = R::const_ident().into();
        let loaded = self.resources.get(&id)?;
        loaded.resource().downcast_ref()
    }

    pub fn add_boxed(&mut self, runner: Box<dyn LoadedResource>) -> &mut Self {
        self.resources.insert(runner.ident().into(), runner);
        self
    }

    async fn run(self) {
        let futures = self
            .resources
            .into_values()
            .filter_map(LoadedResource::runner)
            .map(Runner::run);
        futures::future::join_all(futures).await;
    }
}
