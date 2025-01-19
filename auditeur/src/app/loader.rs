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
pub trait Loader: Send {
    /// Consume the loader and create a [`LoadedResource`], resolving inputs using `registry`
    ///
    /// # Errors
    ///
    /// - If the inputs return any errors
    ///   - If any resources are not found
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error>;
    /// Return a list of [`Constraints`] that must be satisfied before loading
    fn constraints(&self) -> Constraints;
    /// Return a reference to the contained [resource](mod@crate::resource)'s [`NamespacedId`]
    fn ident(&self) -> &NamespacedIdRef;
}

pub struct Listener<M: Message, L: ListenerTrait<M> + Create> {
    external_input: L::ExternalInput,
    phantom_data: PhantomData<M>,
}

impl<M: Message, L: ListenerTrait<M> + Create> Listener<M, L> {
    pub const fn new(external_input: L::ExternalInput) -> Self {
        Self {
            external_input,
            phantom_data: PhantomData,
        }
    }
}

#[async_trait]
impl<M: Message, L: ListenerTrait<M> + Create> Loader for Listener<M, L> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let registry_input = L::RegistryInput::create(registry, L::const_ident()).await?;
        let listener = L::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: L::const_ident().into(),
                error,
            })?;
        let runner = loaded::Listener::build(listener, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        L::RegistryInput::constraints().extend(L::Input::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        L::const_ident()
    }
}

pub struct Resource<R: ResourceTrait + Create> {
    external_input: R::ExternalInput,
}

impl<R: ResourceTrait + Create> Resource<R> {
    pub const fn new(external_input: R::ExternalInput) -> Self {
        Self { external_input }
    }
}

#[async_trait]
impl<R: ResourceTrait + Create> Loader for Resource<R> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let registry_input = R::RegistryInput::create(registry, R::const_ident()).await?;
        let resource = R::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: R::const_ident().into(),
                error,
            })?;
        Ok(Box::new(loaded::Resource::new(resource)))
    }

    fn constraints(&self) -> Constraints {
        R::RegistryInput::constraints()
    }

    fn ident(&self) -> &NamespacedIdRef {
        R::const_ident()
    }
}

pub struct Watcher<W: WatcherTrait + Create> {
    external_input: W::ExternalInput,
}

impl<W: WatcherTrait + Create> Watcher<W> {
    pub const fn new(external_input: W::ExternalInput) -> Self {
        Self { external_input }
    }
}

#[async_trait]
impl<W: WatcherTrait + Create> Loader for Watcher<W> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let registry_input = W::RegistryInput::create(registry, W::const_ident()).await?;
        let watcher = W::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: W::const_ident().into(),
                error,
            })?;
        let runner = loaded::Watcher::build(watcher, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        W::RegistryInput::constraints().extend(W::Input::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        W::const_ident()
    }
}
