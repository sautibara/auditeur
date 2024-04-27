use std::collections::HashMap;

use tokio::sync::broadcast::Receiver;

use crate::prelude::*;

use self::{constraints::Constraint, loader::Loader, runnable::Runnable};

pub mod constraints;
mod loader;
mod runnable;

pub struct App<M> {
    sender: Sender<M>,
    loaders: HashMap<NamespacedId, Box<dyn Loader<M>>>,
}

impl<M: Message> App<M> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let sender = Sender::new(capacity);
        Self {
            sender,
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
        self.add(loader::With::<I, M, runnable::Listener<M, L>, _>::new(
            |output, registry| runnable::Listener::build(func(output), registry),
        ))
    }

    pub fn add_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.add(loader::Resource::new(resource))
    }

    pub fn add_resource_with<I: Input<M>, R: Resource>(
        &mut self,
        func: impl FnOnce(I::Output) -> R + Send + 'static,
    ) -> &mut Self {
        self.add(loader::With::<I, M, runnable::Resource<R>, _>::new(
            |output, _| Ok(runnable::Resource::new(func(output))),
        ))
    }

    /// Listen to all messages until [`M::Exit`] is sent
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency was not found
    pub async fn run(mut self) -> Result<(), Error> {
        let mut registry = Registry::new(self.sender.clone());
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
    resources: HashMap<NamespacedId, Box<dyn Runnable<M>>>,
}

impl<M: Message> Registry<M> {
    fn new(sender: Sender<M>) -> Self {
        Self {
            sender,
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
    pub fn get<R: Resource>(&self) -> Option<&R> {
        let id = R::const_ident().into();
        let runnable = self.resources.get(&id)?;
        runnable.inner().downcast_ref()
    }

    pub fn add<R: Runnable<M>>(&mut self, runnable: R) -> &mut Self {
        self.resources
            .insert(R::const_ident().into(), Box::new(runnable));
        self
    }

    /// # Errors
    ///
    /// - If `listener`'s input failed to build
    pub fn add_listener<L: Listener<M>>(&mut self, listener: L) -> Result<&mut Self, Error> {
        let erased = runnable::Listener::build(listener, self)?;
        Ok(self.add(erased))
    }

    pub fn add_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.add(runnable::Resource::new(resource))
    }

    /// Listen to all messages until [`M::exit`] is sent
    pub async fn run(mut self) {
        let futures = self
            .resources
            .values_mut()
            .filter(|runnable| runnable.is_runnable())
            .map(|runnable| runnable.run());
        futures::future::join_all(futures).await;
    }
}
