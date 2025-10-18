use crate::{
    app::{LoadedResource, Loader, Runner},
    prelude::*,
};

pub struct NonRunnableLoader<R: Resource + Create> {
    external_input: R::ExternalInput,
}

impl<R: Resource + Create> NonRunnableLoader<R> {
    pub const fn new(external_input: R::ExternalInput) -> Self {
        Self { external_input }
    }
}

#[async_trait]
impl<R: Resource + Create> Loader for NonRunnableLoader<R> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let ident = ComponentIdRef::Resource(R::const_ident());
        let registry_input = R::RegistryInput::create(registry, ident).await?;
        let resource = R::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: R::const_ident().into(),
                error,
            })?;
        Ok(Box::new(LoadedNonRunnable::new(resource)))
    }

    fn constraints(&self) -> Constraints {
        R::RegistryInput::constraints()
    }

    fn ident(&self) -> &NamespacedIdRef {
        R::const_ident()
    }
}

struct LoadedNonRunnable<R: Resource> {
    resource: R,
}

impl<R: Resource> LoadedNonRunnable<R> {
    pub const fn new(resource: R) -> Self {
        Self { resource }
    }
}

impl<R: Resource> LoadedResource for LoadedNonRunnable<R> {
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Err(self)
    }

    fn resource(&self) -> &dyn Resource {
        &self.resource
    }

    fn resource_mut(&mut self) -> &mut dyn Resource {
        &mut self.resource
    }
}

impl<R: Resource> Identifiable for LoadedNonRunnable<R> {
    fn ident(&self) -> &'static NamespacedIdRef {
        self.resource.ident()
    }

    fn const_ident() -> &'static NamespacedIdRef
    where
        Self: Sized,
    {
        R::const_ident()
    }
}
