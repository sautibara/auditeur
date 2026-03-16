//! A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
//! currently loaded

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    ops::{Add, AddAssign},
};

use anyhow::{anyhow, bail};
use tracing::{instrument, trace};

use crate::{
    Either,
    app::{
        channels::{
            AnyListenChannel, AnyTakeChannel, EventReceiver, EventSender, Giver, TypedAnyChannel,
        },
        event::BundleInit,
    },
    bundle,
    input::dependency::{self, Removable},
    prelude::*,
    resource::listener,
};

#[cfg(feature = "runtime-loading")]
use crate::app::{channels::Channels, event::GetRequest};

mod query;
pub use query::*;

pub(crate) fn init() -> Bundle {
    new_bundle!("auditeur:resource_map").with_listener::<ResourceMap>(())
}

const RESERVED_IDENTS: &[&NamespacedIdRef] = &[ident!("auditeur:registry")];

/// Holds the amount of dependencies a component has from a specific other component, separated by
/// the type of the dependency.
///
/// For example, a component "foo" could use another component "bar" as a [`Get`] input. Suppose
/// that "bar" also implements [`GetRef`] through [`respond`], and lastly that "foo" additionally holds
/// a removable [`dependency::Handle`] for "bar". In this case, the `inherit` count would be `1`
/// from the [`respond`] handle, the `set_true` count would be `1` from the [`dependency::Handle`],
/// and `set_false` would also be `1` from the [`GetRef`] handle.
// TODO: make a test for the above situation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RemovableCounts {
    inherit: usize,
    set_true: usize,
    set_false: usize,
}

impl RemovableCounts {
    /// Returns `true` if this dependency, as a whole, is removable.
    ///
    /// A dependency is deemed removable if it has at least one `set_true` dependency and exactly
    /// zero `set_false` dependencies (that is, `set_false` overrides `set_true`).
    const fn removable(&self) -> bool {
        // default is unremovable, and it also has priority
        self.set_true > 0 && self.set_false == 0
    }

    /// Returns `true` if this dependency, as a whole, is empty.
    ///
    /// That is, that there are no dependencies in this direction - every count is zero.
    const fn is_empty(&self) -> bool {
        self.inherit == 0 && self.set_true == 0 && self.set_false == 0
    }

    /// Returns the entrywise subtraction of `other` from `self`, or `None` if any of the
    /// subtractions would overflow below `0`.
    fn checked_sub(self, other: Self) -> Option<Self> {
        Some(Self {
            inherit: self.inherit.checked_sub(other.inherit)?,
            set_true: self.set_true.checked_sub(other.set_true)?,
            set_false: self.set_false.checked_sub(other.set_false)?,
        })
    }
}

impl From<Removable> for RemovableCounts {
    fn from(value: Removable) -> Self {
        match value {
            Removable::Inherit => Self {
                inherit: 1,
                ..Default::default()
            },
            Removable::Set(false) => Self {
                set_false: 1,
                ..Default::default()
            },
            Removable::Set(true) => Self {
                set_true: 1,
                ..Default::default()
            },
        }
    }
}

impl Add for RemovableCounts {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            inherit: self.inherit + rhs.inherit,
            set_true: self.set_true + rhs.set_true,
            set_false: self.set_false + rhs.set_false,
        }
    }
}

impl AddAssign for RemovableCounts {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

// Lists all dependencies and dependents for a specific component as well as the count of how many
// things are keeping this component alive.
#[derive(Default, Debug)]
struct ComponentDependencies {
    // The number of strong references keeping this resource alive.
    //
    // Strong references could come from dependents, child components for a bundle, or if this is a
    // running resource.
    strong_count: usize,

    // STRONG: want to keep dependencies alive - we're still using them
    dependencies: hashbrown::HashMap<ComponentId, RemovableCounts>,
    // WEAK: don't want to keep dependents alive - we don't use them
    dependents: hashbrown::HashMap<ComponentId, RemovableCounts>,
}

/// Holds mutable references to the bookkeeping of the two sides of a dependency link in a
/// [`ResourceMap`]. This is included mainly to reduce repeated code.
struct DependencyInfoRes<'a> {
    dependency_counts: &'a mut RemovableCounts,
    dependent_counts: &'a mut RemovableCounts,
    dependency_strong_count: &'a mut usize,
}

impl ComponentDependencies {
    /// Reads the [`DependencyInfoRes`] of a [`dependency::Link`] from a [`ResourceMap`].
    ///
    /// Returns [`None`] if the dependency and dependent are the same component, or if the
    /// dependent is not in the map. Note that the dependent not being in the map is not an error
    /// since dependents can unload at any time - components do not hold strong references to their
    /// dependents.
    ///
    /// # Errors
    ///
    /// - If the dependency is not in the map
    fn dependency_info<'a>(
        map: &'a mut ResourceMap,
        link: &dependency::Link,
    ) -> anyhow::Result<Option<DependencyInfoRes<'a>>> {
        let dependency::Link {
            dependency,
            dependent,
        } = link;

        if dependency == dependent {
            return Ok(None);
        }

        let [dependency_opt, dependent_opt] =
            map.dependencies.get_many_mut([dependency, dependent]);

        let Some(dependency_info) = dependency_opt else {
            bail!("could not find dependency {dependency}");
        };

        let Some(dependent_info) = dependent_opt else {
            return Ok(None);
        };

        let dependency_counts = dependent_info
            .dependencies
            .entry_ref(dependency)
            .or_default();
        let dependent_counts = dependency_info.dependents.entry_ref(dependent).or_default();

        Ok(Some(DependencyInfoRes {
            dependency_counts,
            dependent_counts,
            dependency_strong_count: &mut dependency_info.strong_count,
        }))
    }

    /// Adds `dependency` to `map`.
    ///
    /// Note that no error is returned if the dependency and dependent are the same component, or
    /// if the dependent is not in the map (but the map will not be changed).
    ///
    /// # Errors
    ///
    /// - If the dependency is not in the map.
    fn add_dependency(map: &mut ResourceMap, dependency: dependency::Full) -> anyhow::Result<()> {
        let Some(DependencyInfoRes {
            dependency_counts,
            dependent_counts,
            dependency_strong_count,
        }) = Self::dependency_info(map, &dependency.link)?
        else {
            // dependency_id == dependent_id
            return Ok(());
        };

        let new_dependency = dependency_counts.is_empty();

        let counts = RemovableCounts::from(dependency.removable);

        *dependency_counts += counts;
        *dependent_counts = *dependency_counts;

        if new_dependency {
            *dependency_strong_count += 1;

            map.sender
                .send(RegistryEvent::NewDependency(dependency.link));
        }

        Ok(())
    }

    /// Removes `dependency` from `map`.
    ///
    /// Note that no error is returned if the dependency and dependent are the same component, or
    /// if the dependent is not in the map (but the map will not be changed).
    ///
    /// # Errors
    ///
    /// - If the dependency is not in the map.
    fn remove_dependency(
        map: &mut ResourceMap,
        dependency: dependency::Full,
    ) -> anyhow::Result<()> {
        let Some(DependencyInfoRes {
            dependency_counts,
            dependent_counts,
            ..
        }) = Self::dependency_info(map, &dependency.link)?
        else {
            // dependency_id == dependent_id
            return Ok(());
        };

        let counts = RemovableCounts::from(dependency.removable);

        let Some(new_counts) = dependency_counts.checked_sub(counts) else {
            bail!("DecrementDependency called more than IncrementDependency")
        };

        *dependency_counts = new_counts;
        *dependent_counts = new_counts;

        if new_counts.is_empty() {
            map.drop_reference(dependency.link.dependency.as_ref());

            map.sender
                .send(RegistryEvent::RemovedDependency(dependency.link));
        }

        Ok(())
    }

    fn dependents(&self) -> impl Iterator<Item = (ComponentIdRef<'_>, &RemovableCounts)> {
        self.dependents
            .iter()
            .filter(|(_, counts)| !counts.is_empty())
            .map(|(id, counts)| (id.as_ref(), counts))
    }
}

/// A trait for structs that hold bookkeeping data for components in the map.
///
/// Included to reduce repeated code.
trait LoadedComponent: Sized {
    /// Initializes any references that this component has by default.
    ///
    /// For example, a resource always holds a reference to its parent bundle.
    ///
    /// # Errors
    ///
    /// - If an required reference is not in the map.
    fn init_references(&self, map: &mut ResourceMap) -> anyhow::Result<()>;
    /// Drops any references that this component has by default.
    ///
    /// Note that this doesn't drop any dependencies - those are removed automatically through
    /// [`dependency::Handle`].
    fn drop_references(&self, map: &mut ResourceMap);

    /// Returns the unique id for this component in its namespace.
    fn id(&self) -> &NamespacedIdRef;
    /// Wraps any namespaced id into the correct variant of [`ComponentId`] for this component.
    fn wrap_id(id: &NamespacedIdRef) -> ComponentIdRef<'_>;
    /// Returns the [`ComponentId`] of this component.
    fn component_id(&self) -> ComponentIdRef<'_> {
        Self::wrap_id(self.id())
    }

    /// Returns the dependency info of this component from `map`.
    fn dependencies<'a>(&'a self, map: &'a ResourceMap) -> &'a ComponentDependencies {
        // debug_assert!(map.get_resource(&self.id).is_some());

        // we can index here because the resource existing ensures that its dependencies are also in
        // the map (it is a precondition)
        &map.dependencies[&self.component_id()]
    }

    /// Accesses both the map that holds instances of this component's bookkeeping and the
    /// one that holds all generic dependency info from `map`.
    fn maps_mut(
        map: &mut ResourceMap,
    ) -> (
        &mut hashbrown::HashMap<NamespacedId, Self>,
        &mut hashbrown::HashMap<ComponentId, ComponentDependencies>,
    );
}

/// The bookkeeping for a resource component.
struct LoadedResource {
    id: NamespacedId,

    // STRONG: keep the parent bundle alive
    bundle: NamespacedId,

    #[cfg(feature = "runtime-loading")]
    resource: Option<Box<dyn super::LoadedResource>>,
}

impl LoadedComponent for LoadedResource {
    fn init_references(&self, map: &mut ResourceMap) -> anyhow::Result<()> {
        map.reference_bundle(&self.bundle)?;
        Ok(())
    }

    fn drop_references(&self, map: &mut ResourceMap) {
        map.drop_bundle_reference(&self.bundle);
    }

    fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    fn wrap_id(id: &NamespacedIdRef) -> ComponentIdRef<'_> {
        ComponentIdRef::Resource(id)
    }

    fn maps_mut(
        map: &mut ResourceMap,
    ) -> (
        &mut hashbrown::HashMap<NamespacedId, Self>,
        &mut hashbrown::HashMap<ComponentId, ComponentDependencies>,
    ) {
        (&mut map.resources, &mut map.dependencies)
    }
}

#[allow(clippy::used_underscore_binding)]
impl Debug for LoadedResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("LoadedResource");
        debug_struct
            .field("id", &self.id)
            .field("bundle", &self.bundle);

        #[cfg(feature = "runtime-loading")]
        debug_struct.field(
            "resource",
            &if self.resource.is_some() {
                "<present>"
            } else {
                "<empty>"
            },
        );

        debug_struct.finish()
    }
}

#[cfg(feature = "runtime-loading")]
impl LoadedResource {
    /// Supply an owned resource to us so we could handle its get requests.
    fn supply_owned(&mut self, resource: Box<dyn super::LoadedResource>) {
        tracing::trace!(
            "adding {} to the ResourceMap as a NonRunnable",
            resource.ident()
        );
        self.resource = Some(resource);
    }

    /// Checks a [`GetRequest`] on our owned resource, if it exists.
    async fn check_get_request(&self, get_request: &GetRequest, registry: &Registry) {
        let resource_option = &self.resource;
        if let Some(resource) = resource_option.as_deref() {
            get_request.check_ref(resource.resource(), registry).await;
        }
    }
}

/// The bookkeeping for a bundle component.
#[derive(Debug)]
struct LoadedBundle {
    id: NamespacedId,

    // STRONG: we want to keep the parent bundle alive
    parent: Option<NamespacedId>,
    // WEAK: children can come and go
    resources: HashSet<NamespacedId>,
    bundles: HashSet<NamespacedId>,
    messages: HashSet<NamespacedId>,

    #[cfg(feature = "runtime-loading")]
    init: Option<bundle::SharedInitFn>,
}

impl LoadedComponent for LoadedBundle {
    fn init_references(&self, map: &mut ResourceMap) -> anyhow::Result<()> {
        if let Some(parent) = self.parent.as_deref() {
            map.reference_bundle(parent)?;
        }

        Ok(())
    }

    fn drop_references(&self, map: &mut ResourceMap) {
        if let Some(parent) = self.parent.as_deref() {
            map.drop_bundle_reference(parent);
        }
    }

    fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    fn wrap_id(id: &NamespacedIdRef) -> ComponentIdRef<'_> {
        ComponentIdRef::Bundle(id)
    }

    fn maps_mut(
        map: &mut ResourceMap,
    ) -> (
        &mut hashbrown::HashMap<NamespacedId, Self>,
        &mut hashbrown::HashMap<ComponentId, ComponentDependencies>,
    ) {
        (&mut map.bundles, &mut map.dependencies)
    }
}

impl LoadedBundle {
    fn new(id: NamespacedId) -> Self {
        Self {
            id,
            parent: None,
            resources: HashSet::new(),
            bundles: HashSet::new(),
            messages: HashSet::new(),
            #[cfg(feature = "runtime-loading")]
            init: None,
        }
    }

    /// Replace this bundle's parent bundle with another, adding a strong reference to the new
    /// bundle, and removing our previous strong reference to the old bundle.
    fn replace_parent(
        &mut self,
        map: &mut ResourceMap,
        parent: NamespacedId,
    ) -> anyhow::Result<()> {
        map.reference_bundle(&parent)?;
        if (self.parent.as_ref()).is_some_and(|old_parent| parent != *old_parent)
            && let Some(old_parent) = self.parent.replace(parent)
        {
            map.drop_bundle_reference(&old_parent);
        }
        Ok(())
    }

    /// Update the info about this bundle, including its child components
    fn add_properties(
        &mut self,
        resources: impl IntoIterator<Item = NamespacedId>,
        messages: impl IntoIterator<Item = NamespacedId>,
        bundles: impl IntoIterator<Item = NamespacedId>,
        #[cfg(feature = "runtime-loading")] init: Option<bundle::SharedInitFn>,
    ) {
        self.resources.extend(resources);
        self.messages.extend(messages);
        self.bundles.extend(bundles);
        if cfg!(feature = "runtime-loading") {
            self.init = init.or_else(|| Option::take(&mut self.init));
        }
    }

    /// Returns the info of the parent bundle, or [`None`] if it doesn't exist.
    fn parent<'a>(&self, map: &'a ResourceMap) -> Option<&'a Self> {
        self.parent.as_deref().and_then(|id| map.get_bundle(id))
    }

    /// Returns the ids of all loaded child resources.
    fn resources(&self, map: &ResourceMap) -> impl Iterator<Item = &NamespacedIdRef> {
        self.resources.iter().map(|id| &**id).filter(|id| {
            map.get_resource(id)
                .is_some_and(|resource| resource.bundle == self.id)
        })
    }

    /// Returns the ids of all loaded child bundles.
    fn bundles(&self, map: &ResourceMap) -> impl Iterator<Item = &NamespacedIdRef> {
        self.bundles.iter().map(|id| &**id).filter(|id| {
            map.get_bundle(id).is_some_and(|bundle| {
                bundle
                    .parent
                    .as_ref()
                    .is_some_and(|bundle| *bundle == self.id)
            })
        })
    }

    /// Returns the ids of all loaded child messages.
    fn messages(&self, map: &ResourceMap) -> impl Iterator<Item = &NamespacedIdRef> {
        self.messages.iter().map(|id| &**id).filter(|id| {
            map.get_message(id).is_some_and(|message| {
                message
                    .bundle
                    .as_ref()
                    .is_some_and(|bundle| *bundle == self.id)
            })
        })
    }
}

/// The bookkeeping for a message component.
#[derive(Debug)]
struct LoadedMessage {
    id: NamespacedId,
    bundle: Option<NamespacedId>,
    #[cfg(feature = "runtime-loading")]
    channel: Option<TypedAnyChannel>,
}

impl LoadedComponent for LoadedMessage {
    fn init_references(&self, map: &mut ResourceMap) -> anyhow::Result<()> {
        if let Some(bundle) = self.bundle.as_deref() {
            map.reference_bundle(bundle)?;
        }
        Ok(())
    }

    fn drop_references(&self, map: &mut ResourceMap) {
        if let Some(bundle) = self.bundle.as_deref() {
            map.drop_bundle_reference(bundle);
        }
    }

    fn id(&self) -> &NamespacedIdRef {
        &self.id
    }

    fn wrap_id(id: &NamespacedIdRef) -> ComponentIdRef<'_> {
        ComponentIdRef::Message(id)
    }

    fn maps_mut(
        map: &mut ResourceMap,
    ) -> (
        &mut hashbrown::HashMap<NamespacedId, Self>,
        &mut hashbrown::HashMap<ComponentId, ComponentDependencies>,
    ) {
        (&mut map.messages, &mut map.dependencies)
    }
}

impl LoadedMessage {
    const fn new(id: NamespacedId) -> Self {
        Self {
            id,
            bundle: None,
            #[cfg(feature = "runtime-loading")]
            channel: None,
        }
    }

    /// Replace this message's parent bundle with another, adding a strong reference to the new
    /// bundle, and removing our previous strong reference to the old bundle.
    fn replace_bundle(
        &mut self,
        map: &mut ResourceMap,
        bundle: NamespacedId,
    ) -> anyhow::Result<()> {
        if self.bundle.as_ref().is_some_and(|old| *old == bundle) {
            return Ok(());
        }

        map.reference_bundle(&bundle)?;
        if let Some(bundle) = map.bundles.get_mut(&bundle) {
            bundle.messages.insert(self.id.clone());
        }

        if let Some(old) = self.bundle.take() {
            map.drop_bundle_reference(&old);
        }

        self.bundle = Some(bundle);

        Ok(())
    }
}

/// A dependency tree of the [resources](mod@crate::resource), [messages](crate::app::channels) and
/// [bundles](crate::Bundle) that are currently loaded
///
/// This tree is necessary for telling when resources, bundles, and messages are unloaded. While
/// running resources can be [stopped](RegistryEvent::Stopped), they only unload when every
/// dependency to them also unloads. As such, the map keeps track of these
/// [dependencies](crate::input::dependency).
///
/// Similarly, bundles only unload when all of its child resources and bundles also unload
///
/// This listener is automatically loaded to every [`App`] that is constructed
/// when the `resource-map` feature is specified.
#[derive(Runnable, Debug)]
#[create]
#[resource("auditeur:resource_map")]
#[listener(
    registry_component = false,
    respond,
    listen(BundleInit),
    components(MainComponent)
)]
pub struct ResourceMap {
    #[create(input)]
    sender: EventSender,
    /// A map of all of the currently running resources
    ///
    /// These resources are the roots of what stays loaded, and as such, are stored as strong
    /// references
    // STRONG: the runners are the roots of everything loaded
    runners: HashSet<NamespacedId>,
    /// A map of all of the resources that have ever been loaded
    // WEAK: this simply just lists all resources
    resources: hashbrown::HashMap<NamespacedId, LoadedResource>,
    /// A map of all of the bundles that have ever been loaded
    // WEAK: this simply just lists all bundles
    bundles: hashbrown::HashMap<NamespacedId, LoadedBundle>,
    /// A map of all of the messages that have ever been loaded
    // WEAK: this simply just lists all messages
    messages: hashbrown::HashMap<NamespacedId, LoadedMessage>,
    /// A map of all of the dependencies for any resources, bundles, or messages
    // This is a [`hashbrown::HashMap`] because hashbrown supports the [`Equivalent`] trait, which
    // allows for getting entries without implementing the [`Borrow`] trait. More specifically,
    // the std [`HashMap`] requires the map's key (in this case [`ComponentId`]) to implement
    // [`Borrow`] for a querying key (in this case [`ComponentIdRef<'_>`]). But `Borrow` must
    // return a reference, and we can't construct a ComponentIdRef on the stack, return a reference
    // to it, and exit the function, as it would be a use after free (and thus is rejected by the
    // borrow checker)!
    dependencies: hashbrown::HashMap<ComponentId, ComponentDependencies>,
}

impl CreateDerive for ResourceMap {
    async fn create((): Self::External, _: &Self::Input) -> anyhow::Result<Self::Plain> {
        Ok(Self::Plain {
            runners: HashSet::new(),
            resources: hashbrown::HashMap::new(),
            bundles: hashbrown::HashMap::new(),
            messages: hashbrown::HashMap::new(),
            dependencies: hashbrown::HashMap::new(),
        })
    }
}

impl ResourceMap {
    /// Add a component to the map, fully initializing its references.
    fn add_component<C: LoadedComponent>(&mut self, component: C) -> Result<()> {
        component.init_references(self)?;

        let id = component.component_id();
        let (map, dependencies) = C::maps_mut(self);
        dependencies.insert(id.into_owned(), ComponentDependencies::default());
        map.insert(id.inner().to_owned(), component);

        Ok(())
    }

    /// Take a component out of the map if it exists, or initialize it with a callback function.
    /// If the component was taken, it doesn't remove any of its references. If it was initialized,
    /// the new components' references are added to the map. As such, this should likely be
    /// followed by [`Self::replace_component`].
    fn take_or_init_component<C: LoadedComponent>(
        &mut self,
        id: &NamespacedIdRef,
        init: impl FnOnce() -> C,
    ) -> Result<C> {
        let (map, _) = C::maps_mut(self);

        match map.entry_ref(id) {
            hashbrown::hash_map::EntryRef::Occupied(occupied) => Ok(occupied.remove()),
            hashbrown::hash_map::EntryRef::Vacant(_) => {
                let component = init();
                component.init_references(self)?;
                Ok(component)
            }
        }
    }

    /// Put back a component that was already in the map.
    ///
    /// This adds the component to the map without initializing its references. As such, it can be
    /// thought of as the inverse of [`Self::take_or_init_component`].
    fn replace_component<C: LoadedComponent>(&mut self, component: C) {
        let (map, dependencies) = C::maps_mut(self);
        dependencies
            .entry_ref(&C::wrap_id(component.id()))
            .or_default();
        map.insert(component.id().to_owned(), component);
    }

    /// Add a resource with id `id` to the map.
    fn add_resource(&mut self, id: NamespacedId, parent: NamespacedId) -> Result<()> {
        self.add_component(LoadedResource {
            id,
            bundle: parent,
            #[cfg(feature = "runtime-loading")]
            resource: None,
        })
    }

    /// Add a bundle with id `id` to the map, or if it's already in the map, update its data.
    fn add_bundle(
        &mut self,
        id: &NamespacedIdRef,
        parent: Option<NamespacedId>,
        resources: &[NamespacedId],
        messages: &[NamespacedId],
        bundles: &[NamespacedId],
        #[cfg(feature = "runtime-loading")] init: Option<bundle::SharedInitFn>,
    ) -> Result<()> {
        let mut bundle = self.take_or_init_component(id, || LoadedBundle::new(id.to_owned()))?;

        if let Some(parent) = parent {
            bundle.replace_parent(self, parent)?;
        }

        bundle.add_properties(
            resources.iter().cloned(),
            bundles.iter().cloned(),
            messages.iter().cloned(),
            init,
        );

        self.replace_component(bundle);

        Ok(())
    }

    fn adopt(&mut self, bundle: &NamespacedIdRef, component: ComponentId) {
        if let Some(bundle) = self.bundles.get_mut(bundle) {
            match component {
                ComponentId::Resource(id) => bundle.resources.insert(id),
                ComponentId::Bundle(id) => bundle.bundles.insert(id),
                ComponentId::Message(id) => bundle.messages.insert(id),
            };
        }
    }

    /// Add a message with id `id` to the map, or if it's already in the map, update its data.
    fn add_message(
        &mut self,
        id: &NamespacedIdRef,
        bundle: Option<NamespacedId>,
        #[cfg(feature = "runtime-loading")] channel: Option<TypedAnyChannel>,
    ) -> anyhow::Result<()> {
        let mut message = self.take_or_init_component(id, || LoadedMessage::new(id.to_owned()))?;

        if let Some(bundle) = bundle {
            message.replace_bundle(self, bundle)?;
        }

        #[cfg(feature = "runtime-loading")]
        if let Some(channel) = channel {
            message.channel = Some(channel);
        }

        self.replace_component(message);

        Ok(())
    }

    /// Drop a strong reference to the component with type `C` and id `id`. If this is the last
    /// strong reference to this component, the component may be unloaded. And, if so, all of its
    /// strong references to other components may also be unloaded.
    fn drop_component_reference<C: LoadedComponent>(&mut self, id: &NamespacedIdRef) {
        let (components, dependencies) = C::maps_mut(self);
        if let hashbrown::hash_map::EntryRef::Occupied(mut entry) = components.entry_ref(id) {
            let component = entry.get_mut();
            let mut dependencies = dependencies
                .entry_ref(&component.component_id())
                .or_default_entry();
            dependencies.get_mut().strong_count = dependencies.get().strong_count.saturating_sub(1);

            if dependencies.get().strong_count == 0 {
                // get the ComponentId from the dependencies map
                let (component_id, _) = dependencies.remove_entry();
                // and the component from the component map
                let (_, component) = entry.remove_entry();
                // unload our references before we unload
                component.drop_references(self);

                trace!("unloaded: {component_id}");
                self.sender.send(RegistryEvent::Unloaded(component_id));
            }
        }
    }

    /// Drop a strong reference to the resource with the id `id`.
    fn drop_resource_reference(&mut self, id: &NamespacedIdRef) {
        self.drop_component_reference::<LoadedResource>(id);
    }

    /// Drop a strong reference to the bundle with the id `id`.
    fn drop_bundle_reference(&mut self, id: &NamespacedIdRef) {
        self.drop_component_reference::<LoadedBundle>(id);
    }

    /// Drop a strong reference to the message with the id `id`.
    fn drop_message_reference(&mut self, id: &NamespacedIdRef) {
        self.drop_component_reference::<LoadedMessage>(id);
    }

    /// Drop a strong reference to the component with the id `id`.
    fn drop_reference(&mut self, id: ComponentIdRef<'_>) {
        match id {
            ComponentIdRef::Resource(id) => self.drop_resource_reference(id),
            ComponentIdRef::Bundle(id) => self.drop_bundle_reference(id),
            ComponentIdRef::Message(id) => self.drop_message_reference(id),
        }
    }

    /// Add a strong reference to the resource with the id `id`.
    fn reference_resource(&mut self, id: &NamespacedIdRef) -> anyhow::Result<()> {
        self.reference(ComponentIdRef::Resource(id))
    }

    /// Add a strong reference to the bundle with the id `id`.
    fn reference_bundle(&mut self, id: &NamespacedIdRef) -> anyhow::Result<()> {
        self.reference(ComponentIdRef::Bundle(id))
    }

    /// Add a strong reference to the component with the id `id`.
    fn reference(&mut self, id: ComponentIdRef<'_>) -> anyhow::Result<()> {
        self.dependencies
            .get_mut(&id)
            .map(|component| component.strong_count += 1)
            .ok_or_else(|| anyhow!("failed to get {id}"))
    }

    fn add_dependency(&mut self, dependency: dependency::Full) -> Result<()> {
        // implementation is in [`ComponentDependencies`] to make determining references its issue
        ComponentDependencies::add_dependency(self, dependency)
    }

    fn remove_dependency(&mut self, dependency: dependency::Full) -> Result<()> {
        // same thing as above
        ComponentDependencies::remove_dependency(self, dependency)
    }

    fn get_resource(&self, id: &NamespacedIdRef) -> Option<&LoadedResource> {
        self.resources.get(id)
    }

    fn get_bundle(&self, id: &NamespacedIdRef) -> Option<&LoadedBundle> {
        self.bundles.get(id)
    }

    fn get_message(&self, id: &NamespacedIdRef) -> Option<&LoadedMessage> {
        self.messages.get(id)
    }
}

#[doc(hidden)]
pub struct MainComponent {
    receiver: EventReceiver,
    #[cfg(feature = "runtime-loading")]
    registry: Registry,
    should_exit: bool,
}

impl Input for MainComponent {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.event_receiver(),
            #[cfg(feature = "runtime-loading")]
            registry: registry.handle(),
            should_exit: false,
        })
    }
}

impl listener::Component<ResourceMap> for MainComponent {
    type Value = RegistryEvent;

    async fn wait(&mut self, _: &ResourceMap) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        let res = self.receiver.recv().await;
        Ok(res.map_or(ControlFlow::Break(()), ControlFlow::Continue))
    }

    #[instrument(skip(self, map))]
    async fn handle(
        &mut self,
        event: RegistryEvent,
        map: &mut ResourceMap,
    ) -> anyhow::Result<ControlFlow<()>> {
        trace!("handling registry event");
        match event {
            RegistryEvent::BundleResolving {
                id,
                parent,
                resources,
                messages,
                bundles,
            }
            | RegistryEvent::BundleLoading {
                id,
                parent,
                resources,
                messages,
                bundles,
            } => {
                map.add_bundle(
                    &id,
                    parent,
                    &resources,
                    &messages,
                    &bundles,
                    #[cfg(feature = "runtime-loading")]
                    None,
                )?;
            }
            RegistryEvent::Resolving { id, bundle } | RegistryEvent::Loading { id, bundle }
                if map.get_resource(&id).is_none() =>
            {
                map.add_resource(id, bundle)?;
            }
            RegistryEvent::ListenMessageLoaded { id, bundle } => {
                map.add_message(&id, Some(bundle), None)?;
            }
            RegistryEvent::TakeMessageLoaded { id, bundle } => {
                map.add_message(&id, Some(bundle), None)?;
            }
            RegistryEvent::IncrementDependency(dependency)
                if (dependency.link.dependent)
                    .as_resource()
                    .is_none_or(|resource_id| !RESERVED_IDENTS.contains(&resource_id)) =>
            {
                map.add_dependency(dependency)?;
            }
            RegistryEvent::DecrementDependency(dependency)
                if (dependency.link.dependent)
                    .as_resource()
                    .is_none_or(|resource_id| !RESERVED_IDENTS.contains(&resource_id)) =>
            {
                map.remove_dependency(dependency)?;
            }
            RegistryEvent::Running(id) => {
                map.reference_resource(&id)?;
                map.runners.insert(id);
            }
            RegistryEvent::Stopped(id) => {
                map.runners.remove(&id);
                map.drop_resource_reference(&id);
                // this should be the last runner
                if self.should_exit && map.runners.len() <= 1 {
                    return Ok(ControlFlow::Break(()));
                }
            }
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::NonRunnable(supplier) => {
                if let Some(resource) = map.resources.get_mut(supplier.id())
                    && let Some(owned) = supplier.take()
                {
                    resource.supply_owned(owned);
                }
            }
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::Get(get_request) => {
                if let Some(resource_info) = map.get_resource(get_request.id()) {
                    resource_info
                        .check_get_request(&get_request, &self.registry)
                        .await;
                }
            }
            RegistryEvent::Exit => {
                if map.runners.len() <= 1 {
                    return Ok(ControlFlow::Break(()));
                }
                self.should_exit = true;
            }
            RegistryEvent::Adopt { bundle, component } => {
                map.adopt(&bundle, component);
            }
            _ => (),
        }
        Ok(ControlFlow::Continue(()))
    }
}

impl Listen<BundleInit> for ResourceMap {
    async fn on(&mut self, message: BundleInit) -> anyhow::Result<()> {
        self.add_bundle(
            message.id(),
            None,
            &[],
            &[],
            &[],
            #[cfg(feature = "runtime-loading")]
            message.init(),
        )
    }
}

#[respond("auditeur:resource_map", get_ref)]
impl ResourceMap {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.resources
            .iter()
            .filter(|(_, resource)| resource.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.bundles
            .iter()
            .filter(|(_, bundle)| bundle.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Message`](crate::Message) loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_messages(&self) -> HashSet<NamespacedId> {
        self.messages
            .iter()
            .filter(|(_, message)| message.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] running in the main loop
    #[must_use]
    pub fn running_resources(&self) -> HashSet<NamespacedId> {
        self.runners.clone()
    }

    /// Returns a static view into the resources and bundles that fit `query`
    #[must_use]
    pub fn query(&self, query: Query) -> SubMap {
        query.get(self)
    }

    #[cfg(feature = "runtime-loading")]
    pub(crate) fn add_channels(&mut self, channels: Channels) {
        for (id, channel) in channels.listen {
            let res = self.add_message(&id, None, Some(TypedAnyChannel::Listen(channel)));
            if let Err(err) = res {
                error!("failed to add listen channel with id '{id}': {err:?}");
            }
        }

        for (id, channel) in channels.take {
            let res = self.add_message(&id, None, Some(TypedAnyChannel::Take(channel)));
            if let Err(err) = res {
                error!("failed to add take channel with id '{id}': {err:?}");
            }
        }
    }

    #[cfg(feature = "runtime-loading")]
    pub(crate) fn listen_channel(&self, id: NamespacedId) -> Option<AnyListenChannel> {
        self.get_message(&id)
            .and_then(|message| message.channel.as_ref())
            .and_then(|channel| channel.as_listen())
            .cloned()
    }

    #[cfg(feature = "runtime-loading")]
    pub(crate) fn take_channel(&self, id: NamespacedId) -> Option<AnyTakeChannel> {
        self.get_message(&id)
            .and_then(|message| message.channel.as_ref())
            .and_then(|channel| channel.as_take())
            .cloned()
    }
}

impl ResourceMapConnection {
    pub(crate) fn new_for_registry(
        event_sender: EventSender,
        giver: Giver<ResourceMapQuery>,
    ) -> Self {
        Self {
            dependency_handle: dependency::Handle::new(
                event_sender,
                dependency::Full {
                    link: dependency::Link {
                        dependency: ComponentIdRef::Resource(ResourceMap::const_ident())
                            .into_owned(),
                        dependent: ComponentIdRef::Resource(ident!("auditeur:registry"))
                            .into_owned(),
                    },
                    removable: dependency::Removable::Inherit,
                },
            ),
            giver,
        }
    }
}
