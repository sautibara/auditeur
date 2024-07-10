use crate::prelude::*;

use super::{constraints::Constraint, loaded, loader, Loader};

#[macro_export]
// the macro is re-exported in the root of the crate, so it needs a specific name
#[allow(clippy::module_name_repetitions)]
macro_rules! new_bundle {
    ($str:tt) => {
        $crate::app::bundle::Bundle::new($crate::ident!($str))
    };
}

#[must_use = "A bundle does nothing unless it is applied to an App - try App::with_bundle"]
pub struct Bundle<M: Message> {
    id: NamespacedId,
    loaders: Vec<Box<dyn Loader<M>>>,
    bundles: Vec<Self>,
    constraints: Constraints,
}

impl<M: Message> Bundle<M> {
    pub fn new(id: impl Into<NamespacedId>) -> Self {
        Self {
            id: id.into(),
            loaders: Vec::new(),
            bundles: Vec::new(),
            constraints: Constraints::empty(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> &NamespacedId {
        &self.id
    }

    pub fn with<L: Loader<M> + 'static>(mut self, loader: L) -> Self {
        self.loaders.push(Box::new(loader));
        self
    }

    pub fn with_resource<R: Resource>(self, resource: R) -> Self {
        self.with(loader::Resource::new(resource))
    }

    pub fn with_listener<L: Listener<M>>(self, listener: L) -> Self {
        self.with(loader::Listener::new(listener))
    }

    pub fn with_watcher<W: Watcher<M>>(self, watcher: W) -> Self {
        self.with(loader::Watcher::new(watcher))
    }

    pub fn with_bundle(mut self, bundle: impl Supplier<M>) -> Self {
        let bundle = bundle.build();
        self.bundles.push(bundle);
        self
    }

    pub fn with_resource_with<I: Input<M>, R: Resource>(
        self,
        func: impl FnOnce(I::Output) -> R + Send + 'static,
    ) -> Self {
        self.with(loader::With::<I, M, loaded::Resource<R>, _>::new(
            |output, _| Ok(loaded::Resource::new(func(output))),
        ))
    }

    pub fn with_listener_with<I: Input<M>, L: Listener<M>>(
        self,
        func: impl FnOnce(I::Output) -> L + Send + 'static,
    ) -> Self {
        self.with(loader::With::<I, M, loaded::Listener<M, L>, _>::new(
            |output, registry| loaded::Listener::build(func(output), registry),
        ))
    }

    pub fn with_watcher_with<I: Input<M>, W: Watcher<M>>(
        self,
        func: impl FnOnce(I::Output) -> W + Send + 'static,
    ) -> Self {
        self.with(loader::With::<I, M, loaded::Watcher<M, W>, _>::new(
            |output, registry| loaded::Watcher::build(func(output), registry),
        ))
    }

    fn add_constraint(mut self, constraint: Constraint) -> Self {
        self.constraints = std::mem::take(&mut self.constraints).insert(constraint);
        self
    }

    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.add_constraint(Constraint::AfterResource { id: id.into() })
    }

    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.add_constraint(Constraint::AfterBundle { id: id.into() })
    }

    pub(super) fn apply_to(self, app: &mut App<M>, parent_bundle: Option<&NamespacedId>) {
        let id = self.id;
        for bundle in self.bundles {
            bundle.apply_to(app, Some(&id));
        }
        for loader in self.loaders {
            app.add_loader(loader, id.clone());
        }
        app.add_bundle_inner(id, self.constraints, parent_bundle.cloned());
    }
}

pub trait Supplier<M: Message> {
    fn build(self) -> Bundle<M>;
}

impl<M: Message, F: FnOnce() -> Bundle<M>> Supplier<M> for F {
    fn build(self) -> Bundle<M> {
        self()
    }
}

impl<M: Message> Supplier<M> for Bundle<M> {
    fn build(self) -> Self {
        self
    }
}
