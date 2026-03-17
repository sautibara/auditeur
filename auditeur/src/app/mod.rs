//! The main entry-point for the application: loading and running [resources](mod@crate::resource)
//!
//! The main [`App`] handles loading [resources](crate::resource::Resource),
//! [listeners](crate::resource::listener::Listener), and
//! [watchers](crate::resource::watcher::Watcher) from [bundles](Bundle) given by the user.
//!
//! # The Loading Process
//!
//! Note: This is a very technical explanation, and none of it is needed to be known to effectively
//! use this crate. However, it may help with understanding certain things - especially loaders.
//!
//! - Before being added to an [`App`], resources are each wrapped in a [`Loader`]
//!     - Loaders are trait objects that handle resolving [`Input`]s and building
//!       a resource into a [`LoadedResource`]
//! - When the app begins being built, a [`Registry`] is created to hold each [`LoadedResource`]
//! - Each [`Loader`] is loaded into the registry based on its [`Constraints`]
//!     - [`Constraints`] are essentially a list of resources that must be loaded before loading
//!       another resource
//!     - These are necessary so that resources can ensure that the other resources they access
//!       through [`Get<_>`](crate::input::get::Get) are already loaded
//! - Once loaded, resources are each built into a [`LoadedResource`]
//!     - [`LoadedResource`]s build the [`Input`]s for their resources from the registry
//!     - [`LoadedResource`]s allow for dynamically accessing the contained resource by the
//!       registry, enabling [`input::Get<_>`](crate::input::get::Get) to work
//!     - Then, [`LoadedResource`]s may also provide a [`Runner`] to run in the main loop, which is
//!       how listeners and watchers are run
//! - When the registry is [run](Registry::run), each [`Runner`] is asynchronously run at the same
//!   time until they all finish
//!
//! The basic pipeline: [`impl Resource`](Resource) / [`impl Listener`](Listener) /
//! [`impl Watcher`](Watcher) -> [`dyn Loader`](Loader) -> [`dyn LoadedResource`](LoadedResource)
//! -> (optionally) [`dyn Runner`](Runner)

use std::fmt::Display;

use crate::app::channels::Channels;
use crate::prelude::*;

// TODO: separate NamespacedId into ResourceId and BundleId

pub mod channels;
pub mod event;

#[allow(clippy::module_inception)]
mod app;
pub use app::App;
pub(crate) use app::BuiltBundle;

mod registry;
pub use registry::Registry;

#[cfg(feature = "resource-map")]
pub mod resource_map;
#[cfg(feature = "resource-map")]
pub use resource_map::ResourceMap;

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
    /// Returns a set of channels that this loader "owns".
    ///
    /// These channels will automatically be added to the [registry](Registry), but special care
    /// should be taken to make sure that a specific channel isn't added multiple times.
    #[must_use]
    fn channels(&self) -> Channels {
        Channels::new()
    }
}

/// A [resource](mod@crate::resource) that has already been loaded into a [`Registry`]
#[allow(clippy::module_name_repetitions)] // without Loaded, it would conflict with Resource
pub trait LoadedResource: Identifiable + Send + Sync {
    /// Optionally provide a [`Runner`] to run in the main loop
    ///
    /// This runner is joined alongside every other runner, if it is given
    ///
    /// # Errors
    ///
    /// - Returns `Err(Self)` if this resource doesn't provide a runner
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>>;
    /// Obtain a reference to the original resource within a trait object
    fn resource(&self) -> &dyn Resource;
    /// Obtain a reference to the original resource within a trait object
    fn resource_mut(&mut self) -> &mut dyn Resource;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: Identifiable + Send {
    async fn run(self: Box<Self>);
}

/// An id of a specific part of an [`App`].
///
/// This also shows the different namespaces that exist in `auditeur`. For example, it is okay if a
/// [`Resource`] and a [`Bundle`] have the same [`NamespacedId`], but not for a [`Listen`] message
/// and a [`Take`] message.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComponentId {
    /// A [`Resource`]
    Resource(NamespacedId),
    /// A [`Bundle`]
    Bundle(NamespacedId),
    /// A [`Listen`] or [`Take`] [`Message`](crate::Message).
    Message(NamespacedId),
}

impl ComponentId {
    /// Returns a [`ComponentIdRef`] referencing this id.
    #[must_use]
    pub fn as_ref(&self) -> ComponentIdRef<'_> {
        match self {
            Self::Resource(id) => ComponentIdRef::Resource(id),
            Self::Bundle(id) => ComponentIdRef::Bundle(id),
            Self::Message(id) => ComponentIdRef::Message(id),
        }
    }

    /// Returns a [`Resource`] id with id `id`.
    pub fn resource(id: impl Into<NamespacedId>) -> Self {
        Self::Resource(id.into())
    }

    /// Returns a [`Bundle`] id with id `id`.
    pub fn bundle(id: impl Into<NamespacedId>) -> Self {
        Self::Bundle(id.into())
    }

    /// Returns a [`Message`](crate::Message) id with id `id`.
    pub fn message(id: impl Into<NamespacedId>) -> Self {
        Self::Message(id.into())
    }

    /// Returns [`Some`] with the resource id if this is a [`Resource`](Self::Resource), or
    /// [`None`] otherwise.
    #[must_use]
    pub fn as_resource(&self) -> Option<&NamespacedIdRef> {
        if let Self::Resource(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns [`Some`] with the bundle id if this is a [`Bundle`](Self::Bundle), or
    /// [`None`] otherwise.
    #[must_use]
    pub fn as_bundle(&self) -> Option<&NamespacedIdRef> {
        if let Self::Bundle(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns [`Some`] with the message id if this is a [`Message`](Self::Message), or
    /// [`None`] otherwise.
    #[must_use]
    pub fn as_message(&self) -> Option<&NamespacedIdRef> {
        if let Self::Message(v) = self {
            Some(v)
        } else {
            None
        }
    }

    /// Returns the inner [`NamespacedIdRef`], no matter what type of component this is.
    #[must_use]
    pub fn inner(&self) -> &NamespacedIdRef {
        match self {
            Self::Resource(id) | Self::Bundle(id) | Self::Message(id) => id,
        }
    }

    /// Returns the inner [`NamespacedId`], no matter what type of component this is.
    #[must_use]
    pub fn into_inner(self) -> NamespacedId {
        match self {
            Self::Resource(id) | Self::Bundle(id) | Self::Message(id) => id,
        }
    }
}

impl Display for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// A reference to a [`ComponentId`].
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub enum ComponentIdRef<'a> {
    Resource(&'a NamespacedIdRef),
    Bundle(&'a NamespacedIdRef),
    Message(&'a NamespacedIdRef),
}

impl<'a> ComponentIdRef<'a> {
    /// Converts this into an owned [`ComponentId`] by cloning.
    #[must_use]
    pub fn into_owned(self) -> ComponentId {
        match self {
            Self::Resource(id) => ComponentId::Resource(id.to_owned()),
            Self::Bundle(id) => ComponentId::Bundle(id.to_owned()),
            Self::Message(id) => ComponentId::Message(id.to_owned()),
        }
    }

    /// Returns the inner [`NamespacedIdRef`], no matter what type of component this is.
    #[must_use]
    pub const fn inner(self) -> &'a NamespacedIdRef {
        match self {
            Self::Resource(id) | Self::Bundle(id) | Self::Message(id) => id,
        }
    }
}

impl hashbrown::Equivalent<ComponentId> for ComponentIdRef<'_> {
    fn equivalent(&self, key: &ComponentId) -> bool {
        key.as_ref() == *self
    }
}

impl<'a> From<&'a ComponentIdRef<'a>> for ComponentId {
    fn from(value: &'a ComponentIdRef) -> Self {
        value.into_owned()
    }
}

impl Display for ComponentIdRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resource(id) => write!(f, "resource '{id}'"),
            Self::Bundle(id) => write!(f, "bundle '{id}'"),
            Self::Message(id) => write!(f, "message '{id}'"),
        }
    }
}

impl From<ComponentIdRef<'_>> for ComponentId {
    fn from(value: ComponentIdRef<'_>) -> Self {
        value.into_owned()
    }
}

impl<'a> From<&'a Self> for ComponentId {
    fn from(value: &'a Self) -> Self {
        value.clone()
    }
}
