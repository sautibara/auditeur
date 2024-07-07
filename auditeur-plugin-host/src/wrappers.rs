use auditeur::{
    app::{constraints::Constraints, LoadedResource, Registry},
    prelude::async_trait,
    resource::{NamespacedIdRef, Resource},
    Message,
};

use crate::LibraryHandle;

pub struct Loader<M: Message> {
    wrapped: Box<dyn auditeur::app::Loader<M>>,
    handle: LibraryHandle,
}

impl<M: Message> Loader<M> {
    pub fn new(wrapped: Box<dyn auditeur::app::Loader<M>>, handle: LibraryHandle) -> Self {
        Self { wrapped, handle }
    }
}

impl<M: Message> auditeur::app::Loader<M> for Loader<M> {
    fn load(
        self: Box<Self>,
        registry: &Registry<M>,
    ) -> Result<Box<dyn LoadedResource>, auditeur::Error> {
        let loaded = self.wrapped.load(registry)?;
        Ok(Loaded::new_boxed(loaded, self.handle))
    }

    fn constraints(&self) -> Constraints {
        self.wrapped.constraints()
    }
}

impl<M: Message> auditeur::resource::Resource for Loader<M> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.wrapped.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        unimplemented!("const_ident should never be called on a wrapped loader, because it is always contained within a trait object");
    }
}

struct Loaded {
    wrapped: Box<dyn LoadedResource>,
    handle: LibraryHandle,
}

impl Loaded {
    fn new_boxed(
        wrapped: Box<dyn LoadedResource>,
        handle: LibraryHandle,
    ) -> Box<dyn LoadedResource> {
        Box::new(Self { wrapped, handle })
    }
}

impl LoadedResource for Loaded {
    fn runner(self: Box<Self>) -> Option<Box<dyn auditeur::app::Runner>> {
        self.wrapped
            .runner()
            .map(|wrapped| Runner::new_boxed(wrapped, self.handle.clone()))
    }

    fn resource(&self) -> &dyn std::any::Any {
        self.wrapped.resource()
    }
}

impl Resource for Loaded {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.wrapped.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        unimplemented!("const_ident should never be called on a wrapped loaded, because it is always contained within a trait object");
    }
}

struct Runner {
    wrapped: Box<dyn auditeur::app::Runner>,
    _handle: LibraryHandle,
}

impl Runner {
    pub fn new_boxed(
        wrapped: Box<dyn auditeur::app::Runner>,
        handle: LibraryHandle,
    ) -> Box<dyn auditeur::app::Runner> {
        Box::new(Self {
            wrapped,
            _handle: handle,
        })
    }
}

#[async_trait]
impl auditeur::app::Runner for Runner {
    async fn run(self: Box<Self>) {
        self.wrapped.run().await;
    }
}
