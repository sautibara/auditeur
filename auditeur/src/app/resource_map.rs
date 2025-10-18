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
    app::{
        channels::{
            AnyListenChannel, AnyTakeChannel, EventReceiver, EventSender, Giver, TypedAnyChannel,
        },
        event::{
            self, BundleInit, ComponentId, ComponentIdRef, DependencyFull, DependencyHandle,
            DependencyLink, Removable,
        },
    },
    bundle,
    prelude::*,
    resource::listener,
};

#[cfg(feature = "runtime-loading")]
use crate::app::{channels::Channels, event::GetRequest};

pub(crate) fn init() -> Bundle {
    new_bundle!("auditeur:resource_map").with_listener::<ResourceMap>(())
}

// TODO: transient dependencies - dependencies that can be removed easily

const RESERVED_IDENTS: &[&NamespacedIdRef] = &[ident!("auditeur:registry")];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RemovableCounts {
    inherit: usize,
    set_true: usize,
    set_false: usize,
}

impl RemovableCounts {
    const fn removable(&self) -> bool {
        // default is unremovable, and it also has priority
        self.set_true > 0 && self.set_false == 0
    }

    const fn is_empty(&self) -> bool {
        self.inherit == 0 && self.set_true == 0 && self.set_false == 0
    }

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

#[derive(Default, Debug)]
struct ComponentDependencies {
    // number of strong references keeping this resource alive
    strong_count: usize,

    // STRONG: want to keep dependencies alive - we're still using them
    dependencies: hashbrown::HashMap<ComponentId, RemovableCounts>,
    // WEAK: don't want to keep dependents alive - we don't use them
    dependents: hashbrown::HashMap<ComponentId, RemovableCounts>,
}

struct DependencyInfoRes<'a> {
    dependency_counts: &'a mut RemovableCounts,
    dependent_counts: &'a mut RemovableCounts,
    dependency_strong_count: &'a mut usize,
}

impl ComponentDependencies {
    fn dependency_info<'a>(
        map: &'a mut ResourceMap,
        link: &DependencyLink,
    ) -> anyhow::Result<Option<DependencyInfoRes<'a>>> {
        let DependencyLink {
            dependency,
            dependent,
        } = link;

        if dependency == dependent {
            return Ok(None);
        }

        let [dependency_opt, dependent_opt] =
            map.dependencies.get_many_mut([dependency, dependent]);

        let Some(dependency_info) = dependency_opt else {
            bail!("could not find dependency '{dependency}'");
        };

        let Some(dependent_info) = dependent_opt else {
            bail!("could not find dependent '{dependent}'");
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

    fn add_dependency(map: &mut ResourceMap, dependency: DependencyFull) -> anyhow::Result<()> {
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

    fn remove_dependency(map: &mut ResourceMap, dependency: DependencyFull) -> anyhow::Result<()> {
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
}

trait LoadedComponent: Sized {
    fn init_references(&self, map: &mut ResourceMap) -> anyhow::Result<()>;
    fn drop_references(&self, map: &mut ResourceMap);

    fn id(&self) -> &NamespacedIdRef;
    fn wrap_id(id: &NamespacedIdRef) -> ComponentIdRef<'_>;
    fn component_id(&self) -> ComponentIdRef<'_> {
        Self::wrap_id(self.id())
    }

    fn dependencies<'a>(&'a self, map: &'a ResourceMap) -> &'a ComponentDependencies {
        // debug_assert!(map.get_resource(&self.id).is_some());

        // we can index here because the resource existing ensures that its dependencies are also in
        // the map (it is a precondition)
        &map.dependencies[&self.component_id()]
    }

    fn maps_mut(
        map: &mut ResourceMap,
    ) -> (
        &mut hashbrown::HashMap<NamespacedId, Self>,
        &mut hashbrown::HashMap<ComponentId, ComponentDependencies>,
    );
}

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
    fn supply_owned(&mut self, resource: Box<dyn super::LoadedResource>) {
        tracing::trace!(
            "adding {} to the ResourceMap as a NonRunnable",
            resource.ident()
        );
        self.resource = Some(resource);
    }

    /// Runs `func` on the contained owned resource, if it exists
    async fn check_get_request(&self, get_request: &GetRequest, registry: &Registry) {
        let resource_option = &self.resource;
        if let Some(resource) = resource_option.as_deref() {
            get_request.check_ref(resource.resource(), registry).await;
        }
    }
}

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

    fn add_properties(
        &mut self,
        resources: impl IntoIterator<Item = NamespacedId>,
        bundles: impl IntoIterator<Item = NamespacedId>,
        #[cfg(feature = "runtime-loading")] init: Option<bundle::SharedInitFn>,
    ) {
        self.resources.extend(resources);
        self.bundles.extend(bundles);
        if cfg!(feature = "runtime-loading") {
            self.init = init.or_else(|| self.init.take());
        }
    }

    fn parent<'a>(&self, map: &'a ResourceMap) -> Option<&'a Self> {
        self.parent.as_deref().and_then(|id| map.get_bundle(id))
    }
}

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

/// A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
/// currently loaded
///
/// This tree is necessary for telling when [resources](RegistryEvent::Unloaded) or
/// [bundles](RegistryEvent::BundleUnloaded) are unloaded. While running resources can be
/// [stopped](RegistryEvent::Stopped), they only unload when every reference to them also unloads.
/// As such, the map keeps track of these [references](RegistryEvent::Reference)
///
/// Similarly, bundles only unload when all of its child resources and bundles also unload
///
/// This listener is automatically loaded to every [`App`] that is constructed
/// when the `resource-map` feature is specified.
// The way that the map works is that each resource and bundle is stored in an Arc. When resources
// or bundles are dropped, they send messages to the Registry that they get unloaded, so that their
// lifetimes become the lifetime of what they represent. As such, the underlying functionality of
// the Arc is leveraged to count our references for us. To provide a basis for ownership, the map
// owns a list of the currently running resources which become the roots of what remains loaded - a
// running resource never unloads. Then, each shared resource holds a strong reference to each of
// its dependencies and its parent bundle, keeping them all alive while it's loaded. Bundles, too,
// keep their parents alive, if they exist. From this, dependencies are always kept alive at least
// as long as their dependents.
// TODO: add methods to actually view the entire map
#[derive(Runnable, Debug)]
#[listener(
    "auditeur:resource_map",
    registry_component = false,
    respond,
    listen(BundleInit),
    components((component = MainComponent, input = ())),
)]
pub struct ResourceMap {
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

impl Create for ResourceMap {
    type RegistryInput = EventSender;
    type ExternalInput = ();

    async fn create(
        sender: Self::RegistryInput,
        (): Self::ExternalInput,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            sender,
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
        bundles: &[NamespacedId],
        #[cfg(feature = "runtime-loading")] init: Option<bundle::SharedInitFn>,
    ) -> Result<()> {
        let mut bundle = self.take_or_init_component(id, || LoadedBundle::new(id.to_owned()))?;

        if let Some(parent) = parent {
            bundle.replace_parent(self, parent)?;
        }

        bundle.add_properties(resources.iter().cloned(), bundles.iter().cloned(), init);

        self.replace_component(bundle);

        Ok(())
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

    fn add_dependency(&mut self, dependency: DependencyFull) -> Result<()> {
        // implementation is in [`ComponentDependencies`] to make determining references its issue
        ComponentDependencies::add_dependency(self, dependency)
    }

    fn remove_dependency(&mut self, dependency: DependencyFull) -> Result<()> {
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

    // publicize the implementation for the respond version of add_channels
    // TODO: provide a way to do this automatically
    pub(crate) fn add_channels_pub(&mut self, channels: Channels) {
        self.add_channels(channels, ());
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
    type UserInput<'s> = ();

    async fn wait(&mut self, _: &ResourceMap) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        let res = self.receiver.recv().await;
        Ok(res.map_or(ControlFlow::Break(()), ControlFlow::Continue))
    }

    #[instrument(skip(self, map))]
    async fn handle(
        &mut self,
        event: RegistryEvent,
        map: &mut ResourceMap,
        (): Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        trace!("handling registry event");
        match event {
            RegistryEvent::BundleResolving {
                id,
                parent,
                resources,
                bundles,
            }
            | RegistryEvent::BundleLoading {
                id,
                parent,
                resources,
                bundles,
            } => {
                map.add_bundle(
                    &id,
                    parent,
                    &resources,
                    &bundles,
                    #[cfg(feature = "runtime-loading")]
                    None,
                )?;
            }
            RegistryEvent::Resolving { id, bundle } | RegistryEvent::Loading { id, bundle } => {
                if map.get_resource(&id).is_none() {
                    map.add_resource(id, bundle)?;
                }
            }
            RegistryEvent::ListenMessageLoaded { id, bundle } => {
                map.add_message(&id, Some(bundle), None)?;
            }
            RegistryEvent::TakeMessageLoaded { id, bundle } => {
                map.add_message(&id, Some(bundle), None)?;
            }
            RegistryEvent::IncrementDependency(dependency) => {
                if (dependency.link.dependent)
                    .as_resource()
                    .is_none_or(|resource_id| !RESERVED_IDENTS.contains(&resource_id))
                {
                    map.add_dependency(dependency)?;
                }
            }
            RegistryEvent::DecrementDependency(dependency) => {
                if (dependency.link.dependent)
                    .as_resource()
                    .is_none_or(|resource_id| !RESERVED_IDENTS.contains(&resource_id))
                {
                    map.remove_dependency(dependency)?;
                }
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
            _ => (),
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[has_input]
impl Listen<BundleInit> for ResourceMap {
    async fn on(&mut self, message: BundleInit, (): ()) -> anyhow::Result<()> {
        self.add_bundle(
            message.id(),
            None,
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
    pub fn loaded_resources(&self, (): ()) -> HashSet<NamespacedId> {
        self.resources
            .iter()
            .filter(|(_, resource)| resource.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_bundles(&self, (): ()) -> HashSet<NamespacedId> {
        self.bundles
            .iter()
            .filter(|(_, bundle)| bundle.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Message`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_messages(&self, (): ()) -> HashSet<NamespacedId> {
        self.messages
            .iter()
            .filter(|(_, message)| message.dependencies(self).strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] running in the main loop
    #[must_use]
    pub fn running_resources(&self, (): ()) -> HashSet<NamespacedId> {
        self.runners.clone()
    }

    /// Returns a static view into the resources and bundles that fit `query`
    pub fn query(&self, query: Query, (): ()) -> SubMap {
        query.get(self)
    }

    #[cfg(feature = "runtime-loading")]
    pub(crate) fn add_channels(&mut self, channels: Channels, (): ()) {
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
    pub(crate) fn listen_channel(&self, id: NamespacedId, (): ()) -> Option<AnyListenChannel> {
        self.get_message(&id)
            .and_then(|message| message.channel.as_ref())
            .and_then(|channel| channel.as_listen())
            .cloned()
    }

    #[cfg(feature = "runtime-loading")]
    pub(crate) fn take_channel(&self, id: NamespacedId, (): ()) -> Option<AnyTakeChannel> {
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
            dependency_handle: DependencyHandle::new(
                event_sender,
                DependencyFull {
                    link: DependencyLink {
                        dependency: ComponentIdRef::Resource(ResourceMap::const_ident())
                            .into_owned(),
                        dependent: ComponentIdRef::Resource(ident!("auditeur:registry"))
                            .into_owned(),
                    },
                    removable: Removable::Inherit,
                },
            ),
            giver,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum ResourceSource {
    Query,
    DependentBundle,
    Dependent {
        dependency: ComponentId,
        info: RemovableCounts,
    },
}

impl ResourceSource {
    #[must_use]
    pub const fn is_removable_dependency(&self) -> bool {
        matches!(self, Self::Dependent { info, .. } if info.removable())
    }
}

#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct SubMapBundle {
    pub resources: HashMap<NamespacedId, HashSet<ResourceSource>>,
    pub bundles: HashMap<NamespacedId, SubMapBundle>,
    pub complete: bool,
    #[cfg(feature = "runtime-loading")]
    pub init: Option<bundle::SharedInitFn>,
}

impl SubMapBundle {
    fn visit_bundles_mut(&mut self, func: &mut impl FnMut(&NamespacedIdRef, &mut Self)) {
        for (key, value) in &mut self.bundles {
            value.visit_bundles_mut(func);
            func(key, value);
        }
    }

    pub fn combine(&mut self, other: Self) {
        self.resources.extend(other.resources);
        for (key, value) in other.bundles {
            self.bundles.entry(key).or_default().combine(value);
        }
        self.complete |= other.complete;
        self.init = other.init.or_else(|| std::mem::take(&mut self.init));
    }

    pub fn descendent_resources(
        &self,
    ) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        DescendentResources {
            iter: self.resources.iter().chain(self.bundles.values().flat_map(
                (|bundles| Box::new(bundles.descendent_resources()))
                    as fn(&Self) -> Box<DescendentResources>,
            )),
        }
    }

    pub fn descendent_bundle_ids(&self) -> impl Iterator<Item = &NamespacedId> {
        DescendentBundleIds::InList {
            current: Box::new(DescendentBundleIds::Empty),
            rest: self.bundles.iter(),
        }
    }

    fn from_loaded_bundle(bundle: &LoadedBundle, map: &ResourceMap) -> Self {
        Self {
            resources: bundle
                .resources
                .iter()
                .cloned()
                .map(|id| (id, HashSet::from_iter([ResourceSource::Query])))
                .collect(),
            bundles: bundle
                .bundles
                .iter()
                .filter_map(|id| map.get_bundle(id))
                .map(|bundle| (bundle.id.clone(), Self::from_loaded_bundle(bundle, map)))
                .collect(),
            complete: true,
            #[cfg(feature = "runtime-loading")]
            init: bundle.init.clone(),
        }
    }

    fn load_plan(
        &self,
        id: &NamespacedIdRef,
        parents: &mut Vec<NamespacedId>,
    ) -> (Vec<LoadPlan>, LoadResult) {
        let resources: HashSet<_> = self
            .descendent_resources()
            .map(|(id, _info)| id)
            .cloned()
            .collect();
        if self.complete
            && let Some(init) = &self.init
        {
            let plan = LoadPlan {
                init: init.clone(),
                load_type: LoadType::Complete,
                parent_bundles: parents.clone(),
            };
            let result = LoadResult {
                loaded: resources,
                failed: HashSet::new(),
            };
            (vec![plan], result)
        } else {
            let mut plans = Vec::new();
            let mut result = LoadResult {
                loaded: HashSet::new(),
                failed: resources,
            };

            parents.push(id.to_owned());
            for (child_id, child_bundle) in &self.bundles {
                let (child_plans, child_result) = child_bundle.load_plan(child_id, parents);
                plans.extend(child_plans);
                result.extend_loaded(&child_result.loaded);
            }
            parents.pop();

            if !result.failed.is_empty()
                && let Some(init) = &self.init
            {
                let to_load = std::mem::take(&mut result.failed);
                result.loaded = &result.loaded | &to_load;
                let plan = LoadPlan {
                    init: init.clone(),
                    load_type: LoadType::Partial(to_load),
                    parent_bundles: parents.clone(),
                };
                plans.push(plan);
            }

            (plans, result)
        }
    }
}

// rust has trouble finding out this type in [`SubMapBundle::descendent_resources`], so we have to
// write the whole thing out ourselves ;-;
#[allow(clippy::type_complexity)]
struct DescendentResources<'a> {
    iter: std::iter::Chain<
        std::collections::hash_map::Iter<'a, NamespacedId, HashSet<ResourceSource>>,
        std::iter::FlatMap<
            std::collections::hash_map::Values<'a, NamespacedId, SubMapBundle>,
            Box<Self>,
            fn(&'a SubMapBundle) -> Box<Self>,
        >,
    >,
}

impl<'a> Iterator for DescendentResources<'a> {
    type Item = (&'a NamespacedId, &'a HashSet<ResourceSource>);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

// rust has trouble finding out this type in [`SubMapBundle::descendent_bundle_ids`], so we have to
// write the whole thing out ourselves ;-;
#[allow(clippy::type_complexity)]
#[derive(Default)]
enum DescendentBundleIds<'a> {
    #[default]
    Empty,
    InList {
        current: Box<Self>,
        rest: std::collections::hash_map::Iter<'a, NamespacedId, SubMapBundle>,
    },
}

impl<'a> Iterator for DescendentBundleIds<'a> {
    type Item = &'a NamespacedId;

    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::take(self) {
            Self::Empty => None,
            Self::InList {
                mut current,
                mut rest,
            } => current.next().or_else(|| {
                rest.next().map(|(id, bundle)| {
                    *self = Self::InList {
                        current: Box::new(Self::default()),
                        rest: bundle.bundles.iter(),
                    };
                    id
                })
            }),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct SubMap {
    pub root_bundles: HashMap<NamespacedId, SubMapBundle>,
}

impl SubMap {
    fn visit_bundles_mut(&mut self, func: &mut impl FnMut(&NamespacedIdRef, &mut SubMapBundle)) {
        for (key, value) in &mut self.root_bundles {
            value.visit_bundles_mut(func);
            func(key, value);
        }
    }

    pub fn add_bundle_loader<F: Fn() -> Bundle + Send + Sync + 'static>(
        &mut self,
        bundle: &NamespacedIdRef,
        func: F,
    ) {
        #[cfg(feature = "runtime-loading")]
        {
            let shared_fn = bundle::SharedInitFn::new(func);
            self.add_bundle_loader_shared(bundle, &shared_fn);
        }
    }

    #[cfg(feature = "runtime-loading")]
    pub fn add_bundle_loader_shared(
        &mut self,
        bundle: &NamespacedIdRef,
        shared_fn: &bundle::SharedInitFn,
    ) {
        self.visit_bundles_mut(&mut |id, bundle_info| {
            if id == bundle {
                bundle_info.complete = true;
                bundle_info.init = Some(shared_fn.clone());
            }
        });
    }

    pub fn combine(&mut self, other: Self) {
        for (key, value) in other.root_bundles {
            self.root_bundles
                .entry(key.clone())
                .or_default()
                .combine(value);
        }
    }

    pub fn resources(&self) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_resources)
    }

    pub fn bundle_ids(&self) -> impl Iterator<Item = &NamespacedId> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_bundle_ids)
    }

    fn get_bundle_mut(&mut self, map: &ResourceMap, bundle: &LoadedBundle) -> &mut SubMapBundle {
        let mut parent_stack = Vec::from_iter(bundle.parent(map));

        while let Some(parent) = parent_stack.last().and_then(|bundle| bundle.parent(map)) {
            parent_stack.push(parent);
        }

        let mut parents = parent_stack
            .into_iter()
            .rev()
            .chain(std::iter::once(bundle));

        let root = parents
            .next()
            .expect("there should always be a root bundle");

        let mut bundle = self
            .root_bundles
            .entry(root.id.clone())
            .or_insert_with(|| SubMapBundle {
                #[cfg(feature = "runtime-loading")]
                init: root.init.clone(),
                ..Default::default()
            });

        for parent in parents {
            bundle = bundle
                .bundles
                .entry(parent.id.clone())
                .or_insert_with(|| SubMapBundle {
                    #[cfg(feature = "runtime-loading")]
                    init: parent.init.clone(),
                    ..Default::default()
                });
        }

        bundle
    }

    fn add_resource(
        &mut self,
        map: &ResourceMap,
        resource: &NamespacedIdRef,
        source: ResourceSource,
    ) {
        let Some(bundle) =
            (map.get_resource(resource)).and_then(|resource| map.get_bundle(&resource.bundle))
        else {
            return;
        };

        let sub_bundle = self.get_bundle_mut(map, bundle);
        let source_set = sub_bundle.resources.entry(resource.to_owned()).or_default();
        source_set.insert(source);
    }

    fn add_bundle(&mut self, map: &ResourceMap, bundle: &NamespacedIdRef) {
        let Some(bundle) = map.get_bundle(bundle) else {
            return;
        };

        let current = self.get_bundle_mut(map, bundle);
        let to_add = SubMapBundle::from_loaded_bundle(bundle, map);
        current.combine(to_add);
    }

    fn add_all_dependents(&mut self, map: &ResourceMap) {
        let mut entry_queue: Vec<(ComponentId, Option<ResourceSource>)> = self
            .resources()
            .map(|(id, _)| (ComponentId::Resource(id.clone()), None))
            .collect();
        let mut dependent_entries = HashSet::new();

        while let Some(entry) = entry_queue.pop() {
            let (dependency_id, source) = &entry;

            if !dependent_entries.contains(&entry)
                && let Some(dependencies) = map.dependencies.get(dependency_id)
            {
                if source
                    .as_ref()
                    .is_none_or(|source| !source.is_removable_dependency())
                {
                    entry_queue.extend(dependencies.dependents.iter().map(|(id, info)| {
                        (
                            id.to_owned(),
                            Some(ResourceSource::Dependent {
                                dependency: dependency_id.clone(),
                                info: *info,
                            }),
                        )
                    }));
                }

                if let ComponentId::Bundle(bundle_id) = &dependency_id {
                    let bundle = &map.bundles[bundle_id];
                    entry_queue.extend(bundle.resources.iter().map(|resource_id| {
                        (
                            ComponentId::Resource(resource_id.to_owned()),
                            Some(ResourceSource::DependentBundle),
                        )
                    }));
                    entry_queue.extend(bundle.bundles.iter().map(|bundle_id| {
                        (
                            ComponentId::Bundle(bundle_id.to_owned()),
                            Some(ResourceSource::DependentBundle),
                        )
                    }));
                }

                if source.is_some() {
                    dependent_entries.insert(entry);
                }
            }
        }

        for (dependent, info) in dependent_entries {
            if let ComponentId::Resource(dependent) = dependent
                && let Some(info) = info
            {
                self.add_resource(map, &dependent, info);
            }
        }
    }

    /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    /// may never return if a resource can't be unloaded or a dependent isn't specified.
    pub async fn unload_in(&self, registry: &Registry) {
        self.unload_inner(&registry.stopper(), &mut registry.event_receiver())
            .await;
    }

    /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    /// may never return if a resource can't be unloaded or a dependent isn't specified.
    pub async fn unload_with(&self, stopper: &Stopper, events: &EventReceiver) {
        self.unload_inner(stopper, &mut events.resubscribe()).await;
    }

    async fn unload_inner(&self, stopper: &Stopper, events: &mut EventReceiver) {
        let mut resources_to_unload = HashSet::new();
        let mut dependencies_to_remove = HashSet::new();

        for (resource, sources) in self.resources() {
            let mut current_dependencies = Vec::new();
            let mut should_unload = false;

            for source in sources {
                if let ResourceSource::Dependent { dependency, info } = source
                    && info.removable()
                {
                    current_dependencies.push((resource.to_owned(), dependency.to_owned()));
                } else {
                    should_unload = true;
                    break;
                }
            }

            if should_unload || sources.is_empty() {
                resources_to_unload.insert(resource.to_owned());
            } else {
                dependencies_to_remove.extend(current_dependencies);
            }
        }

        if resources_to_unload.is_empty() && dependencies_to_remove.is_empty() {
            return;
        }

        events.mark_seen();

        for resource in &resources_to_unload {
            stopper.stop_id(resource);
        }

        for (dependent, dependency) in &dependencies_to_remove {
            stopper.remove_dependency(event::DependencyLink {
                dependent: ComponentId::Resource(dependent.clone()),
                dependency: dependency.clone(),
            });
        }

        let mut loaded_resources = resources_to_unload.clone();
        let mut loaded_dependencies = dependencies_to_remove.clone();
        while let Some(event) = events.recv().await {
            match event {
                RegistryEvent::Loaded(id) => {
                    if resources_to_unload.contains(&id) {
                        loaded_resources.insert(id);
                    }
                }
                RegistryEvent::Unloaded(id) => {
                    if let ComponentId::Resource(resource) = id {
                        loaded_resources.remove(&resource);
                    }

                    if loaded_resources.is_empty() && loaded_dependencies.is_empty() {
                        return;
                    }
                }
                RegistryEvent::NewDependency(DependencyLink {
                    dependency,
                    dependent: ComponentId::Resource(dependent),
                }) => {
                    let entry = (dependent, dependency);
                    if dependencies_to_remove.contains(&entry) {
                        loaded_dependencies.insert(entry);
                    }
                }
                RegistryEvent::RemovedDependency(DependencyLink {
                    dependency,
                    dependent: ComponentId::Resource(dependent),
                }) => {
                    loaded_dependencies.remove(&(dependent, dependency));

                    if loaded_resources.is_empty() && loaded_dependencies.is_empty() {
                        return;
                    }
                }
                // just in case
                RegistryEvent::Exit => {
                    return;
                }
                _ => (),
            }
        }
    }

    /// Loads all resources in the map with their `init` functions, and returns which resources in
    /// this map had `init` functions.
    ///
    /// # Errors
    ///
    /// - If a resource could not be added to the registry
    ///     - Note that this doesn't update the load result, as the result only tracks which
    ///       resources have init functions.
    pub async fn reload(&self, registry: &mut Registry) -> Result<LoadResult, Error> {
        let mut app = registry.new_app();

        let (plans, result) = self.load_plans();
        for plan in plans {
            let bundle = plan.build();
            app = app.with_bundle(bundle);
        }

        app.add_to_registry(registry)
            .await
            .map_err(|(_, err)| err)?;
        Ok(result)
    }

    fn load_plans(&self) -> (Vec<LoadPlan>, LoadResult) {
        let mut plans = Vec::new();
        let mut result = LoadResult::default();

        let mut parents = Vec::new();
        for (id, bundle) in &self.root_bundles {
            let (root_plans, root_result) = bundle.load_plan(id, &mut parents);
            plans.extend(root_plans);
            result.extend(&root_result);
        }

        (plans, result)
    }
}

#[cfg(feature = "runtime-loading")]
#[derive(Debug)]
struct LoadPlan {
    init: bundle::SharedInitFn,
    load_type: LoadType,
    parent_bundles: Vec<NamespacedId>,
}

#[derive(Debug)]
enum LoadType {
    Partial(HashSet<NamespacedId>),
    Complete,
}

#[cfg(feature = "runtime-loading")]
impl LoadPlan {
    fn build(self) -> Bundle {
        let mut bundle = self.init.call();

        // TODO: filter channels as well
        if let LoadType::Partial(resources) = &self.load_type {
            bundle = bundle.filter_resources(resources);
            bundle = bundle.without_reload();
        } else {
            bundle = bundle.reload_with_shared(self.init);
        }

        for parent in self.parent_bundles.into_iter().rev() {
            bundle = Bundle::new(parent).with_bundle(bundle);
        }

        bundle
    }
}

#[derive(Default, Debug)]
#[cfg(feature = "runtime-loading")]
pub struct LoadResult {
    pub loaded: HashSet<NamespacedId>,
    pub failed: HashSet<NamespacedId>,
}

#[cfg(feature = "runtime-loading")]
impl LoadResult {
    fn extend_loaded(&mut self, set: &HashSet<NamespacedId>) {
        self.loaded = &self.loaded | set;
        self.failed = &self.failed - set;
    }

    fn extend(&mut self, other: &Self) {
        self.loaded = &self.loaded | &other.loaded;
        self.failed = &self.failed | &other.failed;
    }
}

#[derive(Clone)]
#[non_exhaustive]
pub enum Query {
    Resource(NamespacedId),
    Bundle(NamespacedId),
    AndDependents(Box<Self>),
    List(Vec<Self>),
}

impl Default for Query {
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl Query {
    pub fn resource(ident: impl Into<NamespacedId>) -> Self {
        Self::Resource(ident.into())
    }

    pub fn bundle(ident: impl Into<NamespacedId>) -> Self {
        Self::Bundle(ident.into())
    }

    #[must_use]
    pub fn and_dependents(self) -> Self {
        Self::AndDependents(Box::new(self))
    }

    #[must_use]
    pub fn and(self, item: Self) -> Self {
        match self {
            Self::List(mut list) => {
                list.push(item);
                Self::List(list)
            }
            other => Self::List(vec![other, item]),
        }
    }

    fn get(self, map: &ResourceMap) -> SubMap {
        match self {
            Self::Resource(id) => {
                let mut sub_map = SubMap::default();
                sub_map.add_resource(map, &id, ResourceSource::Query);
                sub_map
            }
            Self::Bundle(id) => {
                let mut sub_map = SubMap::default();
                sub_map.add_bundle(map, &id);
                sub_map
            }
            Self::AndDependents(query) => {
                let mut sub_map = query.get(map);
                sub_map.add_all_dependents(map);
                sub_map
            }
            Self::List(list) => {
                let mut sub_map = SubMap::default();
                for query in list {
                    let query_map = query.get(map);
                    sub_map.combine(query_map);
                }
                sub_map
            }
        }
    }
}

impl FromIterator<Self> for Query {
    fn from_iter<T: IntoIterator<Item = Self>>(iter: T) -> Self {
        Self::List(Vec::from_iter(iter))
    }
}
