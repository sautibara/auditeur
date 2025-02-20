//! Groups of [resources](mod@crate::resource) and potential [other bundles](Bundle) under common
//! [ids](NamespacedId)
//!
//! [`Bundle`]s are the main collections for resources and are the only things that an [`App`] can
//! understand. They allow for module-like hierarchies by putting one in another. See the [struct
//! documentation](Bundle) for more

use std::sync::Arc;

use crate::prelude::*;

use crate::app::{loader, BuiltBundle, Constraint, Loader};

/// Create a new bundle with the given identifier
///
/// Note: This macro expands to `Bundle::new(ident!($str))`
// TODO: example
#[macro_export]
// the macro is re-exported in the root of the crate, so it needs a specific name
#[allow(clippy::module_name_repetitions)]
macro_rules! new_bundle {
    ($str:tt) => {
        $crate::bundle::Bundle::new($crate::ident!($str))
    };
}

#[allow(clippy::module_name_repetitions)]
pub use new_bundle;

/// A group of [resources](mod@crate::resource) and potential [other bundles](Self) under a common
/// [id](NamespacedId)
///
/// A bundle is the main container for resources to be put into an [`App`]. They're able to be
/// combined - by putting bundles within other bundles - allowing for hierarchies of functionality.
/// In a sense, they're a bit like rust modules, separating functions into multiple levels.
///
/// Each bundle has a unique [identifier](NamespacedId) in order to differentiate them. As such,
/// care should be taken to prevent identifier collisions.
#[must_use = "A bundle does nothing unless it is applied to an App - try App::with_bundle"]
pub struct Bundle {
    id: NamespacedId,
    loaders: Vec<Box<dyn Loader>>,
    bundles: Vec<Self>,
    constraints: Constraints,
}

impl Bundle {
    /// Create a new bundle with the id `id`
    ///
    /// See [`new_bundle!`] for a simpler way to do this
    pub fn new(id: impl Into<NamespacedId>) -> Self {
        Self {
            id: id.into(),
            loaders: Vec::new(),
            bundles: Vec::new(),
            constraints: Constraints::empty(),
        }
    }

    /// Apply this bundle to the specified [`App`], adding each bundle, loader, the bundle itself
    pub(super) fn apply_to(self, app: &mut App, parent: Option<&NamespacedIdRef>) -> NamespacedId {
        let this_id = &self.id;
        let bundles: Arc<[NamespacedId]> = self
            .bundles
            .into_iter()
            .map(|bundle| bundle.apply_to(app, Some(this_id)))
            .collect();
        let resources: Arc<[NamespacedId]> = self
            .loaders
            .into_iter()
            .map(|loader| app.add_loader(loader, this_id))
            .collect();
        app.add_bundle_inner(
            self.id.clone(),
            BuiltBundle {
                constraints: self.constraints,
                bundles,
                resources,
                parent: parent.map(NamespacedIdRef::to_owned),
            },
        );
        self.id
    }

    /// Get a reference to the id that represents this bundle
    #[must_use]
    pub fn id(&self) -> &NamespacedIdRef {
        &self.id
    }
}

/// These methods allow adding different [resources](mod@crate::resource) to the [`Bundle`]
///
/// Once added, these resources will be allowed to run in the main loop and be referenced by others
/// using [`input::Get<_>`](crate::input::get::Get). Notice that even though [`Listener`]s and
/// [`Watcher`]s can be added through [`Self::with_resource`], if they are, they won't be run in
/// the main loop: they must be added through [`Self::with_listener`] or [`Self::with_watcher`].
/// The namespaces for each also aren't exclusive: if a resource with the same [`NamespacedId`] is
/// added as both a [`Listener`] and [`Watcher`], one will overwrite the other.
impl Bundle {
    /// Add a [`Resource`] to this bundle
    ///
    /// Resources are never run, they only provide themselves to [listeners](Listener),
    /// [watchers](Watcher), or other resources with [`input::Get<_>`](crate::input::get::Get)
    pub fn with_resource<R: Resource + Create>(self, external_input: R::ExternalInput) -> Self {
        self.with(loader::Resource::<R>::new(external_input))
    }

    /// Add a [`Listener`] to this bundle
    ///
    /// Listeners run in the main loop, listening to the main message channel. Like watchers, they
    /// can use [inputs](Input) to reference and talk to other [resources](mod@crate::resource).
    pub fn with_listener<M: Message, L: Listener<M> + Create>(
        self,
        external_input: L::ExternalInput,
    ) -> Self {
        self.with(loader::Listener::<M, L>::new(external_input))
    }

    pub fn with_listener_new<L: crate::resource::listener::Listener + Create>(
        self,
        external_input: L::ExternalInput,
    ) -> Self {
        self.with(crate::resource::listener::ListenerLoader::<L>::new(
            external_input,
        ))
    }

    /// Add a [`Watcher`] to this bundle
    ///
    /// Watchers run in the main loop but never receive from the main message channel (although
    /// they can send to it). Like listeners, they can use [inputs](Input) to reference and talk to
    /// other [resources](mod@crate::resource).
    pub fn with_watcher<W: Watcher + Create>(self, external_input: W::ExternalInput) -> Self {
        self.with(loader::Watcher::<W>::new(external_input))
    }

    /// Add another bundle to this bundle
    ///
    /// When this bundle is loaded in an [`App`], the child bundle will also be added - just like a
    /// resource. The child bundle will also respect this bundle's
    /// [constraints](Self::add_constraint)
    pub fn with_bundle(mut self, bundle: impl Supplier) -> Self {
        let bundle = bundle.build();
        self.bundles.push(bundle);
        self
    }

    /// Add a generic [`Loader`] to the bundle's list of loaders
    pub fn with<L: Loader + 'static>(mut self, loader: L) -> Self {
        self.loaders.push(Box::new(loader));
        self
    }
}

/// These methods allow providing [`Constraints`] that must be satisfied before any contained
/// [resource](mod@crate::resource) or [`Bundle`] is loaded
impl Bundle {
    /// Only load the items within this bundle once the resource specified by `id` has been loaded
    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.add_constraint(Constraint::AfterResource { id: id.into() })
    }

    /// Only load the items within this bundle once the bundle specified by `id` has been loaded
    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.add_constraint(Constraint::AfterBundle { id: id.into() })
    }

    /// Only load the items within this bundle once the specified `constraint` has been satisfied
    pub fn add_constraint(mut self, constraint: Constraint) -> Self {
        self.constraints = std::mem::take(&mut self.constraints).insert(constraint);
        self
    }
}

/// Any way to supply a [`Bundle`] to an [`App`]
///
/// # Examples
///
/// ```
/// use auditeur::prelude::*;
/// #
/// # // In the ident! calls in new_bundle!, proc_macro_crate messes up and thinks that auditeur
/// # // should be referred to as `crate` even though we're in a separate crate for testing. To
/// # // avoid this, we can just import auditeur::resource here so that crate::resource resolves to
/// # // it.
/// # use auditeur::{namespaced_id, path_any};
/// #
/// # fn main() {
///
/// // this is a simple function that creates an empty bundle
/// fn create_bundle() -> Bundle {
///     new_bundle!("from:function")
/// }
///
/// let mut app = App::new();
///
/// // bundles can either be given directly as an expression
/// app.add_bundle(new_bundle!("from:expression"));
/// // or from a function
/// app.add_bundle(create_bundle);
/// # }
/// ```
pub trait Supplier {
    fn build(self) -> Bundle;
}

impl<F: FnOnce() -> Bundle> Supplier for F {
    fn build(self) -> Bundle {
        self()
    }
}

impl Supplier for Bundle {
    fn build(self) -> Self {
        self
    }
}
