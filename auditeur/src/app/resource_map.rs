//! A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
//! currently loaded

use std::{
    collections::{HashMap, HashSet, hash_map},
    fmt::Debug,
};

use anyhow::{anyhow, bail};
use tracing::{instrument, trace};

use crate::{app::EventSender, bundle, prelude::*, resource::listener};

use super::EventReceiver;
#[cfg(feature = "runtime-loading")]
use super::GetRequest;

// TODO: transient dependencies - dependencies that can be removed easily

const RESERVED_IDENTS: &[&NamespacedIdRef] = &[ident!("auditeur:registry")];

struct LoadedResource {
    id: NamespacedId,
    // number of strong references keeping this resource alive
    strong_count: usize,

    // STRONG: keep the parent bundle alive
    bundle: NamespacedId,
    // STRONG: also want to keep dependencies alive
    dependencies: HashSet<NamespacedId>,
    // WEAK: don't want to keep the dependents alive - we don't want reference loops
    dependents: HashSet<NamespacedId>,

    #[cfg(feature = "runtime-loading")]
    resource: Option<Box<dyn super::LoadedResource>>,
}

impl LoadedResource {
    fn new(map: &mut ResourceMap, id: NamespacedId, bundle: NamespacedId) -> anyhow::Result<Self> {
        map.reference_bundle(&bundle)?;
        Ok(Self {
            id,
            strong_count: 0,
            bundle,
            dependencies: HashSet::new(),
            dependents: HashSet::new(),
            #[cfg(feature = "runtime-loading")]
            resource: None,
        })
    }

    fn drop_references(&mut self, map: &mut ResourceMap) {
        map.drop_bundle_reference(std::mem::replace(
            &mut self.bundle,
            NamespacedId::new("", ""),
        ));
        for id in self.dependencies.drain() {
            map.drop_resource_reference(id);
        }
    }

    fn add_dependency(
        map: &mut ResourceMap,
        dependency_id: &NamespacedIdRef,
        dependent_id: &NamespacedIdRef,
    ) -> anyhow::Result<()> {
        if dependency_id == dependent_id {
            return Ok(());
        }

        let [dependency_opt, dependent_opt] = map
            .resources
            .get_disjoint_mut([dependency_id, dependent_id]);

        let Some(dependency_info) = dependency_opt else {
            bail!("could not find dependency '{dependency_id}'");
        };

        let Some(dependent_info) = dependent_opt else {
            bail!("could not find dependent '{dependent_id}'");
        };

        let already_exists = dependency_info.dependents.contains(dependent_id)
            || dependent_info.dependencies.contains(dependency_id);
        if already_exists {
            return Ok(());
        }

        dependency_info.strong_count += 1;

        dependency_info.dependents.insert(dependent_id.to_owned());
        dependent_info.dependencies.insert(dependency_id.to_owned());

        Ok(())
    }
}

#[allow(clippy::used_underscore_binding)]
impl Debug for LoadedResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("LoadedResource");
        debug_struct
            .field("id", &self.id)
            .field("strong_count", &self.strong_count)
            .field("bundle", &self.bundle)
            .field("dependencies", &self.dependencies)
            .field("dependents", &self.dependents);

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
    // number of strong references keeping this resource alive
    strong_count: usize,

    // STRONG: we want to keep the parent bundle alive
    parent: Option<NamespacedId>,
    // WEAK: child resources can come and go
    resources: HashSet<NamespacedId>,
    // WEAK: child bundles can come and go
    bundles: HashSet<NamespacedId>,

    #[cfg(feature = "runtime-loading")]
    init: Option<bundle::SharedInitFn>,
}

impl LoadedBundle {
    fn new(
        map: &mut ResourceMap,
        id: NamespacedId,
        parent: Option<NamespacedId>,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = parent.as_deref() {
            map.reference_bundle(parent)?;
        }
        Ok(Self {
            id,
            strong_count: 0,
            parent,
            resources: HashSet::new(),
            bundles: HashSet::new(),
            #[cfg(feature = "runtime-loading")]
            init: None,
        })
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

    fn drop_references(&mut self, map: &mut ResourceMap) {
        if let Some(parent) = self.parent.take() {
            map.drop_bundle_reference(parent);
        }
    }

    fn parent<'a>(&self, map: &'a ResourceMap) -> Option<&'a Self> {
        self.parent.as_deref().and_then(|id| map.get_bundle(id))
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
    ///
    /// If the underlying Arc unloads (no strong references point to it anymore), that means that
    /// the resource also must be unloaded. As such, this map holds no strong references to any of
    /// these resources, allowing them to unload if necessary.
    resources: HashMap<NamespacedId, LoadedResource>,
    /// A map of all of the bundles that have ever been loaded
    bundles: HashMap<NamespacedId, LoadedBundle>,
}

#[create]
impl Create for ResourceMap {
    async fn create(sender: EventSender) -> Result<Self> {
        Ok(Self {
            sender,
            runners: HashSet::new(),
            resources: HashMap::new(),
            bundles: HashMap::new(),
        })
    }
}

impl ResourceMap {
    fn add_resource(&mut self, id: NamespacedId, parent: NamespacedId) -> Result<()> {
        let resource = LoadedResource::new(self, id.clone(), parent)?;
        self.resources.insert(id, resource);
        Ok(())
    }

    fn add_bundle(
        &mut self,
        id: NamespacedId,
        parent: Option<&NamespacedIdRef>,
        resources: &[NamespacedId],
        bundles: &[NamespacedId],
        #[cfg(feature = "runtime-loading")] init: Option<bundle::SharedInitFn>,
    ) -> Result<()> {
        let mut bundle = if let Some(bundle) = self.bundles.remove(&id) {
            bundle
        } else {
            LoadedBundle::new(self, id.clone(), parent.map(ToOwned::to_owned))?
        };

        bundle.add_properties(resources.iter().cloned(), bundles.iter().cloned(), init);

        self.bundles.insert(id, bundle);

        Ok(())
    }

    fn drop_resource_reference(&mut self, id: NamespacedId) {
        if let hash_map::Entry::Occupied(mut entry) = self.resources.entry(id) {
            let resource = entry.get_mut();
            resource.strong_count = resource.strong_count.saturating_sub(1);

            if resource.strong_count == 0 {
                let (id, mut resource) = entry.remove_entry();

                trace!("unloaded: resource '{id}'");
                self.sender.send(RegistryEvent::Unloaded(id));
                resource.drop_references(self);
            }
        }
    }

    fn drop_bundle_reference(&mut self, id: NamespacedId) {
        if let hash_map::Entry::Occupied(mut entry) = self.bundles.entry(id) {
            let bundle = entry.get_mut();
            bundle.strong_count = bundle.strong_count.saturating_sub(1);

            if bundle.strong_count == 0 {
                let (id, mut bundle) = entry.remove_entry();
                bundle.drop_references(self);
                trace!("unloaded: bundle '{id}'");
                self.sender.send(RegistryEvent::BundleUnloaded(id));
            }
        }
    }

    fn reference_resource(&mut self, id: &NamespacedIdRef) -> anyhow::Result<()> {
        self.resources
            .get_mut(id)
            .map(|resource| resource.strong_count += 1)
            .ok_or_else(|| anyhow!("failed to get resource '{id}'"))
    }

    fn reference_bundle(&mut self, id: &NamespacedIdRef) -> anyhow::Result<()> {
        self.bundles
            .get_mut(id)
            .map(|bundle| bundle.strong_count += 1)
            .ok_or_else(|| anyhow!("failed to get bundle '{id}'"))
    }

    fn add_dependency(
        &mut self,
        dependent: &NamespacedIdRef,
        dependency: &NamespacedIdRef,
    ) -> Result<()> {
        LoadedResource::add_dependency(self, dependency, dependent)
    }

    fn get_resource(&self, id: &NamespacedIdRef) -> Option<&LoadedResource> {
        self.resources.get(id)
    }

    fn get_bundle(&self, id: &NamespacedIdRef) -> Option<&LoadedBundle> {
        self.bundles.get(id)
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
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
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
        // trace!("handling registry event");
        match event {
            RegistryEvent::BundleResolving {
                id,
                parent,
                resources,
                bundles,
            } => {
                map.add_bundle(
                    id,
                    parent.as_deref(),
                    &resources,
                    &bundles,
                    #[cfg(feature = "runtime-loading")]
                    None,
                )?;
            }
            RegistryEvent::BundleLoading {
                id,
                parent,
                resources,
                bundles,
                #[cfg(feature = "runtime-loading")]
                init,
            } => {
                map.add_bundle(
                    id,
                    parent.as_deref(),
                    &resources,
                    &bundles,
                    #[cfg(feature = "runtime-loading")]
                    init.take(),
                )?;
            }
            RegistryEvent::Resolving { id, bundle } | RegistryEvent::Loading { id, bundle } => {
                if map.get_resource(&id).is_none() {
                    map.add_resource(id, bundle)?;
                }
            }
            RegistryEvent::Dependency {
                dependent,
                dependency,
            } => {
                if !RESERVED_IDENTS.contains(&&*dependent) && dependency != dependent {
                    map.add_dependency(&dependent, &dependency)?;
                }
            }
            RegistryEvent::Running(id) => {
                map.reference_resource(&id)?;
                map.runners.insert(id);
            }
            RegistryEvent::Stopped(id) => {
                map.runners.remove(&id);
                map.drop_resource_reference(id);
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

#[respond("auditeur:resource_map", get_ref)]
impl ResourceMap {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_resources(&self, (): ()) -> HashSet<NamespacedId> {
        self.resources
            .iter()
            .filter(|(_, resource)| resource.strong_count != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_bundles(&self, (): ()) -> HashSet<NamespacedId> {
        self.bundles
            .iter()
            .filter(|(_, bundle)| bundle.strong_count != 0)
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
}

#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct SubMapBundle {
    pub resources: HashSet<NamespacedId>,
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

    pub fn descendent_resources(&self) -> impl Iterator<Item = &NamespacedId> {
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
            resources: bundle.resources.clone(),
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
        let resources: HashSet<_> = self.descendent_resources().cloned().collect();
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
        std::collections::hash_set::Iter<'a, NamespacedId>,
        std::iter::FlatMap<
            std::collections::hash_map::Values<'a, NamespacedId, SubMapBundle>,
            Box<Self>,
            fn(&'a SubMapBundle) -> Box<Self>,
        >,
    >,
}

impl<'a> Iterator for DescendentResources<'a> {
    type Item = &'a NamespacedId;

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

    pub fn resources(&self) -> impl Iterator<Item = &NamespacedId> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_resources)
    }

    pub fn bundle_ids(&self) -> impl Iterator<Item = &NamespacedId> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_resources)
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

    fn add_resource(&mut self, map: &ResourceMap, resource: &NamespacedIdRef) {
        let Some(bundle) =
            (map.get_resource(resource)).and_then(|resource| map.get_bundle(&resource.bundle))
        else {
            return;
        };

        let sub_bundle = self.get_bundle_mut(map, bundle);
        sub_bundle.resources.insert(resource.to_owned());
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
        let mut queue: Vec<_> = self.resources().cloned().collect();
        let mut dependents = HashSet::new();

        while let Some(id) = queue.pop() {
            if !dependents.contains(&id)
                && let Some(resource) = map.get_resource(&id)
            {
                dependents.insert(id);
                queue.extend(resource.dependents.iter().cloned());
            }
        }

        for dependent in dependents {
            self.add_resource(map, &dependent);
        }
    }

    /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    /// may never return if a resource can't be unloaded or a dependent isn't specified.
    ///
    /// This is a utility that does the same thing as [`Self::unload`], but it's only available if
    /// the `runtime-loading` feature is specified.
    #[cfg(feature = "runtime-loading")]
    pub async fn unload_in_registry(&self, registry: &Registry) {
        self.unload_inner(&registry.stopper(), &mut registry.event_receiver())
            .await;
    }

    /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    /// may never return if a resource can't be unloaded or a dependent isn't specified.
    pub async fn unload(&self, stopper: &Stopper, events: &EventReceiver) {
        self.unload_inner(stopper, &mut events.resubscribe()).await;
    }

    async fn unload_inner(&self, stopper: &Stopper, events: &mut EventReceiver) {
        let resources: HashSet<NamespacedId> = self.resources().cloned().collect();

        if resources.is_empty() {
            return;
        }

        events.mark_seen();

        for resource in &resources {
            stopper.stop_id(resource);
        }

        let mut loaded_resources = resources.clone();
        while let Some(event) = events.recv().await {
            match event {
                RegistryEvent::Loaded(id) => {
                    if resources.contains(&id) {
                        loaded_resources.insert(id);
                    }
                }
                RegistryEvent::Unloaded(id) => {
                    if resources.contains(&id) {
                        loaded_resources.remove(&id);

                        if loaded_resources.is_empty() {
                            return;
                        }
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
                sub_map.add_resource(map, &id);
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
