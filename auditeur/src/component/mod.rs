//! Items that can be added, loaded, unloaded, and removed from an [`App`].
//!
//! [`Component`]s are the building blocks of an app. When loaded, they can access and load other
//! components, obtaining [dependencies](crate::app::map::Dependency) on them. When all dependencies
//! on a specific component are released (dropped), that component is then automatically unloaded.
//! The app automatically does all the bookkeeping necessary to figure out when components should
//! be loaded and unloaded.
//!
//! Components are added either before the app is run in [`app::Builder`](crate::app::Builder) or
//! at runtime using [`AppComponentsMut::add`](crate::app::component::AppComponentsMut::add). They
//! may also get added with [`Children`] of their own as defined by the [`Register`] trait. Most
//! components have a way of specifying children that should be added whenever the parent component
//! is added. They will only be removed if their removal is
//! [requested](crate::app::component::AppComponentsMut::request).
//!
//! Each component belongs to a specific [`Registry`], which groups together components by use and
//! determines how they load and unload. Useful registries include:
//! - [`Registries`](mod@registry), as registries are also components in of themselves,
//! - [`Resources`](mod@resource), who mainly provide asynchronous tasks to run for long
//!   periods,
//! - [`ListenMessages`](mod@listen), who provide broadcast-style message channels,
//! - [`TakeMessages`](mod@take), who provide mpmc-style message channels,
//! - [`Bundles`](mod@bundle), who are convenient places to group components together,
//!
//! # Examples
//!
//! ## Adding and Removing
//!
//! ```
//! use auditeur::app::map::RequestSet;
//! use auditeur::component::bundle::Bundles;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! // Bundles are the most simple type of component. They only have children, and loading and
//! // unloading are largely unimportant to them.
//!
//! let mut app = App::builder()
//!     // This is an example of a component being added before the app is run.
//!     // Here we make a bundle with no children and add it to the app builder.
//!     .with::<Bundle>(new_bundle!("test:bundle_1"))
//!     .spawn();
//!
//! // We could use the map to query the current state of the app.
//! let map = app.map().get().await;
//! let bundle_id = ComponentId::inn::<Bundles>(ident!("test:bundle_1"));
//!
//! // Since we added it in the builder, the bundle should be added.
//! assert!(map.is_added(&bundle_id));
//! // But since no other components have dependencies on it, it shouldn't be loaded.
//! assert!(!map.is_loaded(&bundle_id));
//!
//! // Components can only be removed if their removal is requested.
//! let waiter = app
//!     .components_mut()
//!     .request(RequestSet::remove(bundle_id.clone()))
//!     .await;
//! waiter.wait().await;
//! let map = app.map().get().await;
//!
//! // Now that we requested the bundle's removal, it shouldn't be added anymore.
//! assert!(!map.is_added(&bundle_id));
//! assert!(!map.is_loaded(&bundle_id));
//!
//! auditeur::test::finish(app, false).await
//! # }
//! ```
//!
//! ## Children
//!
//! ```
//! use auditeur::app::map::RequestSet;
//! use auditeur::component::bundle::Bundles;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let mut app = App::builder().spawn();
//!
//! // Here's a bundle with a child.
//! // We add its child like we would in [`app::Builder`] or [`Children`].
//! let bundle = new_bundle!("test:bundle_1").with::<Bundle>(new_bundle!("test:bundle_2"));
//!
//! // And in this case, we'll add it to the app using [`AppComponentsMut::add`] instead.
//! // This is not necessary; it's just another option to add components.
//! app.components_mut()
//!     .add(Children::new().with::<Bundle>(bundle))
//!     .await;
//!
//! let map = app.map().get().await;
//! let bundle_id_1 = ComponentId::inn::<Bundles>(ident!("test:bundle_1"));
//! let bundle_id_2 = ComponentId::inn::<Bundles>(ident!("test:bundle_2"));
//!
//! // Now both bundles should be added,
//! assert!(map.is_added(&bundle_id_1));
//! assert!(map.is_added(&bundle_id_2));
//!
//! // with the first bundle as the parent of the second.
//! assert!(
//!     map.parent_of(&bundle_id_2)
//!         .is_some_and(|parent| **parent == bundle_id_1)
//! );
//!
//! // Now, if we request the removal of the parent component,
//! app.components_mut()
//!     .request(RequestSet::remove(bundle_id_1.clone()))
//!     .await
//!     .wait()
//!     .await;
//! let map = app.map().get().await;
//!
//! // the child component will also be removed.
//! assert!(!map.is_added(&bundle_id_1));
//! assert!(!map.is_added(&bundle_id_2));
//!
//! auditeur::test::finish(app, false).await
//! # }
//! ```
//!
//! ## Loading and Unloading
//!
//! ```
//! use auditeur::app::map::RequestSet;
//! use auditeur::component::bundle::Bundles;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! // The loading and unloading of bundles is useless, as their loaded value is empty,
//! // but they can still be used to investigate how loading and unloading work.
//!
//! let mut app = App::builder()
//!     .with::<Bundle>(new_bundle!("test:bundle"))
//!     .spawn();
//!
//! let bundle_id = ComponentId::inn::<Bundles>(ident!("test:bundle"));
//!
//! // All components start out unloaded and only get loaded when necessary. (Although, some
//! // registries immediately load their components when they're added, like resources).
//! let map = app.map().get().await;
//! assert!(map.is_added(&bundle_id));
//! assert!(!map.is_loaded(&bundle_id));
//!
//! // By using [`AppComponentsMut::load`], we can ensure that a component is loaded.
//! let dependency: Dependency<()> = app.components_mut().load(bundle_id.clone()).await?;
//!
//! // [`AppComponentsMut::load`] only returns once the component finishes unloading,
//! // so we can be sure that it's loaded now.
//! let map = app.map().get().await;
//! assert!(map.is_loaded(&bundle_id));
//!
//! // Dependencies keep components loaded and provide access to what they're wrapping.
//! // They also hold extra information that can be accessed through [`Dependency::info_of`].
//! let info = Dependency::info_of(&dependency);
//!
//! // For example, we could check who the dependent of the dependency is - who is keeping it alive.
//! let dependent = info.id.dependent();
//! // By default, the dependent of the app handle given through [`app::Builder::spawn`] is the app
//! // component, so that should also be the dependent of our dependency.
//! assert_eq!(**dependent, App::component_id());
//!
//! // Once all dependencies of a component are dropped, it is automatically unloaded.
//! drop(dependency);
//! let waiter = app
//!     .components_mut()
//!     .wait_for(RequestSet::unload(bundle_id.clone()))
//!     .await;
//! waiter.wait().await;
//!
//! let map = app.map().get().await;
//! assert!(!map.is_loaded(&bundle_id));
//!
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::ops::Deref;

use namespaced_id::ArcNamespacedId;

use crate::prelude::*;

use crate::component::registry::{Registries, Registry};

pub mod bundle;
pub mod listen;
pub mod registry;
pub mod resource;
pub mod take;

/// A trait the defines how to load a component (`C`) into a registry.
///
/// See the documentation of [`Registry`] for more information about components.
pub trait Register<C>: Registry {
    type ComponentInput;

    fn register(input: Self::ComponentInput) -> (Self::Loader, Children);
}

/// A type that can be added to a specific [`Registry`].
///
/// The types that can be added to a registry is determined by the registry itself with the
/// [`Register`] trait. As such, this cannot be implemented manually.
pub trait Component<R: Registry> {}
impl<R: Register<C>, C> Component<R> for C {}

/// A type that belongs to a single well-defined [`Registry`].
///
/// Types can be added to multiple registries as long as they implement [`Component`] for each of
/// them. This trait, however, is meant to be implemented only for types that only make sense to be
/// added to a single registry.
pub trait RegistryOf: Sized {
    type Registry: Register<Self>;
}

/// An identifier for a [`Component`] added to a specific [`Registry`].
///
/// This type is cheaply cloneable.
#[derive(Hash, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComponentId {
    /// The [`Registry`] that the component was added to.
    pub registry: &'static NamespacedIdRef,
    /// The component's own specific identifier.
    pub component: ComponentIdPath,
}

impl ComponentId {
    /// Returns the [`ComponentId`] of component `C` if it was added to the registry `R`.
    #[must_use]
    pub fn within<R, C>() -> Self
    where
        R: Register<C> + IdentifiedType,
        C: IdentifiedType,
    {
        Self {
            registry: R::id(),
            component: ComponentIdPath::Static(C::id()),
        }
    }

    /// Returns the [`ComponentId`] of a component with the id `id` if it was added to the registry
    /// `R`.
    #[must_use]
    pub fn inn<R: Registry>(id: impl Into<ComponentIdPath>) -> Self {
        Self {
            registry: R::id(),
            component: id.into(),
        }
    }

    /// Returns the [`ComponentId`] of component `C` if it was added to its registry as defined by
    /// [`RegistryOf`].
    #[must_use]
    pub fn off<C: RegistryOf + IdentifiedType>() -> Self {
        C::component_id()
    }

    #[must_use]
    pub(crate) fn registry_id(&self) -> Self {
        Self {
            registry: Registries::id(),
            component: ComponentIdPath::Static(self.registry),
        }
    }
}

impl Display for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.registry, &*self.component)
    }
}

impl Debug for ComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ComponentId(\"{self}\")")
    }
}

/// A cheaply cloneable [`NamespacedId`], as used in a [`ComponentId`].
#[derive(Debug, Clone)]
pub enum ComponentIdPath {
    /// An id backed by an [`Arc`](std::sync::Arc).
    Shared(ArcNamespacedId),
    /// An id backed by a `'static` reference.
    Static(&'static NamespacedIdRef),
}

impl From<ArcNamespacedId> for ComponentIdPath {
    fn from(v: ArcNamespacedId) -> Self {
        Self::Shared(v)
    }
}

impl From<&'static NamespacedIdRef> for ComponentIdPath {
    fn from(v: &'static NamespacedIdRef) -> Self {
        Self::Static(v)
    }
}

impl From<NamespacedId> for ComponentIdPath {
    fn from(v: NamespacedId) -> Self {
        Self::Shared(v.into())
    }
}

impl Deref for ComponentIdPath {
    type Target = NamespacedIdRef;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(shared) => shared,
            Self::Static(stat) => stat,
        }
    }
}

impl From<ComponentIdPath> for NamespacedId {
    fn from(value: ComponentIdPath) -> Self {
        value.deref().to_owned()
    }
}

impl Hash for ComponentIdPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state);
    }
}

impl PartialEq for ComponentIdPath {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for ComponentIdPath {}

impl PartialOrd for ComponentIdPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentIdPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(&**other)
    }
}

/// A [`ComponentId`] that is fully stored in the heap.
///
/// See [`Event::Flushed`](crate::app::Event::Flushed) for more information.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeapComponentId {
    pub registry: NamespacedId,
    pub component: NamespacedId,
}

impl From<ComponentId> for HeapComponentId {
    fn from(id: ComponentId) -> Self {
        Self {
            registry: id.registry.to_owned(),
            component: id.component.deref().to_owned(),
        }
    }
}

/// A type whose values have associated [`NamespacedId`]s that uniquely represent them.
pub trait Identified {
    fn id_dyn(&self) -> &NamespacedIdRef;
}

/// A type that has an associated [`NamespacedId`] that uniquely represents it.
///
/// If a type implements this trait, then they must also guarantee that [`Self::id`] equals
/// [`Self::id_dyn`](Identified::id_dyn) for all values.
pub trait IdentifiedType: Identified {
    fn id() -> &'static NamespacedIdRef;
}

/// A type that has a constant [`NamespacedId`] that uniquely represents it.
///
/// This is a optional trait to allow using types' identifiers in const context.
///
/// If a type implements this trait, then they must also guarantee that [`Self::ID`] equals
/// [`Self::id`](IdentifiedType::id) equals [`Self::id_dyn`](Identified::id_dyn) for all values.
pub trait IdentifiedConst: IdentifiedType {
    const ID: &'static NamespacedIdRef;
}

/// A type whose values have a well-defined [`ComponentId`].
///
/// These values also must belong to a specific registry by definition.
pub trait IdentifiedComponent {
    fn component_id_dyn(&self) -> ComponentId;
}

/// Types that have a single well-defined [`ComponentId`].
///
/// This is only implemented for components that implement [`RegistryOf`].
pub trait IdentifiedComponentType {
    fn component_id() -> ComponentId;
}

impl<C> IdentifiedComponentType for C
where
    C: RegistryOf + IdentifiedType,
{
    fn component_id() -> ComponentId {
        ComponentId::within::<C::Registry, C>()
    }
}
