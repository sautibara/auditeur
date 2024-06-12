use std::{collections::HashMap, ops::Deref};

use tokio::sync::broadcast::Receiver;

use crate::{prelude::*, Exiter};

use self::{constraints::Constraint, loader::Loader, runner::Runner};

pub mod constraints;
mod loader;
mod runner;

pub struct App<M> {
    sender: Sender<M>,
    exit_handle: Exiter,
    loaders: HashMap<NamespacedId, Box<dyn Loader<M>>>,
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
            .insert(L::const_ident().into(), Box::new(loader));
        self
    }

    pub fn add_listener<L: Listener<M>>(&mut self, listener: L) -> &mut Self {
        self.add(loader::Listener::new(listener))
    }

    pub fn add_listener_with<I: Input<M>, L: Listener<M>>(
        &mut self,
        func: impl FnOnce(I::Output) -> L + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, runner::Listener<M, L>, _>::new(
            |output, registry| runner::Listener::build(func(output), registry),
        ))
    }

    pub fn add_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.add(loader::Resource::new(resource))
    }

    pub fn add_resource_with<I: Input<M>, R: Resource>(
        &mut self,
        func: impl FnOnce(I::Output) -> R + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, runner::Resource<R>, _>::new(
            |output, _| Ok(runner::Resource::new(func(output))),
        ))
    }

    pub fn add_watcher<W: Watcher<M>>(&mut self, watcher: W) -> &mut Self {
        self.add(loader::Watcher::new(watcher))
    }

    pub fn add_watcher_with<I: Input<M>, W: Watcher<M>>(
        &mut self,
        func: impl FnOnce(I::Output) -> W + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, runner::Watcher<M, W>, _>::new(
            |output, registry| runner::Watcher::build(func(output), registry),
        ))
    }

    pub fn add_plugin<P: Plugin<M> + ?Sized>(&mut self, plugin: &P) -> &mut Self {
        plugin.apply(self);
        self
    }

    #[must_use]
    pub fn loaders(&self) -> Vec<&dyn Loader<M>> {
        self.loaders.values().map(Deref::deref).collect()
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
        if loader.is_finished() {
            return Ok(());
        }

        // if there are no constraints, we can just load it now and return
        let constraints = loader.constraints();
        if constraints.is_empty() {
            return loader.load(registry);
        }

        // if there are constraints, we must load them first
        for Constraint::After { id } in constraints {
            self.resolve(&id, registry)?;
        }

        // then we have to get the loader again because the mutable reference is invalidated by
        // loading the other constraints
        self.loaders
            .get_mut(id)
            .expect("the loader should never be deleted by resolving other loaders")
            .load(registry)
    }
}

pub struct NotFound;

pub struct Registry<M: Message> {
    sender: Sender<M>,
    exit_handle: Exiter,
    resources: HashMap<NamespacedId, Box<dyn Runner<M>>>,
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
        let runner = self.resources.get(&id)?;
        runner.inner().downcast_ref()
    }

    pub fn add<R: Runner<M>>(&mut self, runner: R) -> &mut Self {
        self.resources
            .insert(R::const_ident().into(), Box::new(runner));
        self
    }

    /// # Errors
    ///
    /// - If `listener`'s input failed to build
    pub fn add_listener<L: Listener<M>>(&mut self, listener: L) -> Result<&mut Self, Error> {
        let erased = runner::Listener::build(listener, self)?;
        Ok(self.add(erased))
    }

    pub fn add_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.add(runner::Resource::new(resource))
    }

    /// # Errors
    ///
    /// - If `listener`'s input failed to build
    pub fn add_watcher<W: Watcher<M>>(&mut self, watcher: W) -> Result<&mut Self, Error> {
        let erased = runner::Watcher::build(watcher, self)?;
        Ok(self.add(erased))
    }

    async fn run(mut self) {
        let futures = self
            .resources
            .values_mut()
            .filter(|runner| runner.is_runnable())
            .map(|runner| runner.run());
        futures::future::join_all(futures).await;
    }
}
