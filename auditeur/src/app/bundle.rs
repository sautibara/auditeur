use crate::prelude::*;

use super::{constraints::Constraint, loaded, loader, Loader};

#[macro_export]
// the macro is re-exported in the root of the crate, so it needs a specific name
#[allow(clippy::module_name_repetitions)]
macro_rules! new_bundle {
    ($str:tt) => {
        $crate::app::Bundle::new($crate::ident!($str))
    };
}

/// A group of [resources](mod@crate::resource) and potential [other bundles](Self) under a common id
///
/// A bundle is the main container for resources to be put into an [`App`]. They're able to be
/// combined - by putting bundles within other bundles - allowing for hierarchies of functionality.
/// In a sense, they're a bit like rust modules, separating functions into multiple levels.
///
/// Each bundle has a unique [identifier](NamespacedId) in order to differentiate them. As such,
/// care should be taken to prevent identifier collisions.
///
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

/// Any way to supply a [`Bundle`] to an [`App`]
///
/// # Examples
///
/// ```
/// use auditeur::prelude::*;
/// #
/// # #[derive(Clone)]
/// # struct Message;
/// # impl auditeur::Message for Message {}
/// # auditeur::bake_message!(Message);
/// #
/// # // In the ident! calls in new_bundle!, proc_macro_crate messes up and thinks that auditeur
/// # // should be referred to as `crate` even though we're in a separate crate for testing. To
/// # // avoid this, we can just import auditeur::resource here so that crate::resource resolves to
/// # // it.
/// # use auditeur::resource;
/// #
/// # fn main() {
///
/// // this is a simple function that creates an empty bundle
/// fn create_bundle() -> Bundle {
///     new_bundle!("from:function")
/// }
///
/// let mut app = App::new(32);
///
/// // bundles can either be given directly as an expression
/// app.add_bundle(new_bundle!("from:expression"));
/// // or from a function
/// app.add_bundle(create_bundle);
/// # }
/// ```
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
