use std::fmt::Display;

use super::*;

/// A query for components in a [`ResourceMap`] that is buildable into a [`SubMap`] using
/// [`ResourceMapConnection::query`].
#[derive(Clone, Hash, PartialEq, Eq)]
#[non_exhaustive]
pub enum Query {
    Resource(NamespacedId),
    Message(NamespacedId),
    Bundle(NamespacedId),
    AndDependents(Box<Self>),
    List(Vec<Self>),
}

impl Default for Query {
    /// Returns an empty query.
    fn default() -> Self {
        Self::List(Vec::new())
    }
}

impl Query {
    /// Find just the resource with id `ident` in the map.
    pub fn resource(ident: impl Into<NamespacedId>) -> Self {
        Self::Resource(ident.into())
    }

    /// Find just the message with id `ident` in the map.
    pub fn message(ident: impl Into<NamespacedId>) -> Self {
        Self::Message(ident.into())
    }

    /// Find just the bundle with id `ident` in the map, including all of its children.
    pub fn bundle(ident: impl Into<NamespacedId>) -> Self {
        Self::Bundle(ident.into())
    }

    /// Include all recursive dependents of the items in this query. Specifically, iff a descendent
    /// dependent is non-removable, then its dependents are recursively included as well. If it is
    /// removable, then it is included, but its dependents are not necessarily included.
    #[must_use]
    pub fn and_dependents(self) -> Self {
        Self::AndDependents(Box::new(self))
    }

    /// Add another query to this, as a [list](Self::List).
    #[must_use]
    pub fn and(self, item: Self) -> Self {
        match (self, item) {
            (Self::List(mut left), Self::List(right)) => {
                left.extend(right);
                Self::List(left)
            }
            (single, Self::List(mut list)) | (Self::List(mut list), single) => {
                list.push(single);
                Self::List(list)
            }
            (left, right) => Self::List(vec![left, right]),
        }
    }

    /// Resolve this query in `map`.
    pub(super) fn get(self, map: &ResourceMap) -> SubMap {
        match self {
            Self::Resource(id) => {
                let mut sub_map = SubMap::default();
                sub_map.add_resource(map, &id, ResourceSource::Query);
                sub_map
            }
            Self::Message(id) => {
                let mut sub_map = SubMap::default();
                sub_map.add_message(map, &id, ResourceSource::Query);
                sub_map
            }
            Self::Bundle(id) => {
                let mut sub_map = SubMap::default();
                sub_map.add_bundle(map, &id, &ResourceSource::Query);
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

    fn into_iter(self) -> impl Iterator<Item = Self> {
        match self {
            Self::List(list) => Either::Left(list.into_iter()),
            other => Either::Right(std::iter::once(other)),
        }
    }
}

impl FromIterator<Self> for Query {
    fn from_iter<T: IntoIterator<Item = Self>>(iter: T) -> Self {
        Self::List(iter.into_iter().flat_map(Self::into_iter).collect())
    }
}

/// A static tree view into a [`ResourceMap`].
#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct SubMap {
    /// A map of all the bundles with no parents - the roots of the tree
    pub root_bundles: HashMap<NamespacedId, SubMapBundle>,
}

/// A single bundle and its children within a [`SubMap`].
#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct SubMapBundle {
    /// The direct descendent resources that are currently loaded from this bundle.
    pub resources: HashMap<NamespacedId, HashSet<ResourceSource>>,
    /// The direct descendent messages that are currently loaded from this bundle.
    pub messages: HashMap<NamespacedId, HashSet<ResourceSource>>,
    /// The direct descendent bundles that are currently loaded from this bundle.
    pub bundles: HashMap<NamespacedId, SubMapBundle>,
    /// `true` if this has all children that are currently loaded from this bundle.
    pub complete: bool,
    /// A function that could initialize the bundle, used for [reloading](SubMap::reload_in).
    #[cfg(feature = "runtime-loading")]
    pub init: Option<bundle::SharedInitFn>,
}

impl SubMap {
    /// An iterator over all descendent resources in this map.
    pub fn descendent_resources(
        &self,
    ) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_resources)
    }

    /// An iterator over all descendent messages in this map.
    pub fn descendent_messages(
        &self,
    ) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_messages)
    }

    /// An iterator over the ids of all descendent bundles in this map.
    pub fn descendent_bundle_ids(&self) -> impl Iterator<Item = &NamespacedId> {
        self.root_bundles
            .values()
            .flat_map(SubMapBundle::descendent_bundle_ids)
    }

    /// Runs a closure on every descendent bundle.
    fn visit_bundles_mut(&mut self, func: &mut impl FnMut(&NamespacedIdRef, &mut SubMapBundle)) {
        for (key, value) in &mut self.root_bundles {
            value.visit_bundles_mut(func);
            func(key, value);
        }
    }

    /// If the `runtime-loading` feature is enabled, this replaces the bundle with id `bundle`'s
    /// loader with `func`, or adds a new bundle with that loader if it doesn't exist. If the
    /// feature is disabled, this does nothing.
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

    /// Replaces the bundle with id `bundle`'s loader with `shared_fn`, or adds a new bundle with
    /// that loader if it doesn't exist.
    #[cfg(feature = "runtime-loading")]
    pub fn add_bundle_loader_shared(
        &mut self,
        bundle: &NamespacedIdRef,
        shared_fn: &bundle::SharedInitFn,
    ) {
        let mut added = false;
        self.visit_bundles_mut(&mut |id, bundle_info| {
            if id == bundle {
                bundle_info.complete = true;
                bundle_info.init = Some(shared_fn.clone());
                added = true;
            }
        });

        if !added {
            self.root_bundles.insert(
                bundle.to_owned(),
                SubMapBundle {
                    complete: true,
                    init: Some(shared_fn.clone()),
                    ..Default::default()
                },
            );
        }
    }

    /// Deep merges all bundles in `self` with the bundles in `other`.
    pub fn combine(&mut self, other: Self) {
        for (key, value) in other.root_bundles {
            self.root_bundles
                .entry(key.clone())
                .or_default()
                .combine(value);
        }
    }

    /// Gets or initializes and returns the bundle in this submap that corresponds to the bundle
    /// `bundle` from `map`.
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

    /// Adds the resource with the id `resource` from `map` to this submap.
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

    /// Adds the message with the id `message` from `map` to this submap.
    fn add_message(
        &mut self,
        map: &ResourceMap,
        message: &NamespacedIdRef,
        source: ResourceSource,
    ) {
        let Some(bundle) = map
            .get_message(message)
            .and_then(|message| message.bundle.as_deref())
            .and_then(|bundle| map.get_bundle(bundle))
        else {
            return;
        };

        let sub_bundle = self.get_bundle_mut(map, bundle);
        let source_set = sub_bundle.resources.entry(message.to_owned()).or_default();
        source_set.insert(source);
    }

    /// Adds the bundle with the id `bundle` from `map` to this submap.
    fn add_bundle(&mut self, map: &ResourceMap, bundle: &NamespacedIdRef, source: &ResourceSource) {
        let Some(bundle) = map.get_bundle(bundle) else {
            return;
        };

        let current = self.get_bundle_mut(map, bundle);
        let to_add = SubMapBundle::from_loaded_bundle(bundle, map, source);
        current.combine(to_add);
    }

    /// Adds all the recursive dependents of the items in this map, as specified by
    /// [`Query::AndDependents`].
    fn add_all_dependents(&mut self, map: &ResourceMap) {
        let mut entry_queue: Vec<(ComponentId, Option<ResourceSource>)> = (self
            .descendent_resources()
            .map(|(id, _)| (ComponentId::Resource(id.clone()), None)))
        .chain(
            self.descendent_messages()
                .map(|(id, _)| (ComponentId::Message(id.clone()), None)),
        )
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
                    entry_queue.extend(dependencies.dependents().map(|(id, info)| {
                        (
                            id.into_owned(),
                            Some(ResourceSource::Dependent {
                                dependency: dependency_id.clone(),
                                info: *info,
                            }),
                        )
                    }));
                }

                if let ComponentId::Bundle(bundle_id) = &dependency_id {
                    let bundle = &map.bundles[bundle_id];
                    entry_queue.extend(
                        (bundle.resources(map).map(ComponentId::resource))
                            .chain(bundle.messages(map).map(ComponentId::message))
                            .chain(bundle.bundles(map).map(ComponentId::bundle))
                            .map(|id| (id, Some(ResourceSource::DependentBundle))),
                    );
                }

                if source.is_some() {
                    dependent_entries.insert(entry);
                }
            }
        }

        for (dependent, info) in dependent_entries {
            match (dependent, info) {
                (ComponentId::Resource(resource), Some(info)) => {
                    self.add_resource(map, &resource, info);
                }
                (ComponentId::Message(message), Some(info)) => {
                    self.add_message(map, &message, info);
                }
                _ => (),
            }
        }
    }
}

impl SubMapBundle {
    /// Runs a closure on every descendent bundle.
    fn visit_bundles_mut(&mut self, func: &mut impl FnMut(&NamespacedIdRef, &mut Self)) {
        for (key, value) in &mut self.bundles {
            value.visit_bundles_mut(func);
            func(key, value);
        }
    }

    /// Deep merges all values in `self` with the values in `other`.
    pub fn combine(&mut self, other: Self) {
        self.resources.extend(other.resources);
        self.messages.extend(other.messages);
        for (key, value) in other.bundles {
            self.bundles.entry(key).or_default().combine(value);
        }
        self.complete |= other.complete;
        self.init = other.init.or_else(|| std::mem::take(&mut self.init));
    }

    /// An iterator over all descendent resources in this map.
    pub fn descendent_resources(
        &self,
    ) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        DescendentComponents {
            iter: self.resources.iter().chain(self.bundles.values().flat_map(
                (|bundles| Box::new(bundles.descendent_resources()))
                    as fn(&Self) -> Box<DescendentComponents>,
            )),
        }
    }

    /// An iterator over all descendent messages in this map.
    pub fn descendent_messages(
        &self,
    ) -> impl Iterator<Item = (&NamespacedId, &HashSet<ResourceSource>)> {
        DescendentComponents {
            iter: self.messages.iter().chain(self.bundles.values().flat_map(
                (|bundles| Box::new(bundles.descendent_messages()))
                    as fn(&Self) -> Box<DescendentComponents>,
            )),
        }
    }

    /// An iterator over the ids of all descendent bundles in this map.
    pub fn descendent_bundle_ids(&self) -> impl Iterator<Item = &NamespacedId> {
        DescendentBundleIds::InList {
            current: Box::new(DescendentBundleIds::Empty),
            rest: self.bundles.iter(),
        }
    }

    /// Creates a [`SubMapBundle`] from the info in `bundle` from `map`.
    fn from_loaded_bundle(
        bundle: &LoadedBundle,
        map: &ResourceMap,
        source: &ResourceSource,
    ) -> Self {
        Self {
            resources: bundle
                .resources(map)
                .map(ToOwned::to_owned)
                .map(|id| (id, HashSet::from_iter([source.clone()])))
                .collect(),
            messages: bundle
                .messages(map)
                .map(ToOwned::to_owned)
                .map(|id| (id, HashSet::from_iter([source.clone()])))
                .collect(),
            bundles: bundle
                .bundles(map)
                .filter_map(|id| map.get_bundle(id))
                .map(|bundle| {
                    (
                        bundle.id.clone(),
                        Self::from_loaded_bundle(bundle, map, source),
                    )
                })
                .collect(),
            complete: true,
            #[cfg(feature = "runtime-loading")]
            init: bundle.init.clone(),
        }
    }
}

/// A way that a component is added to a [`SubMap`] from a [`Query`]
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum ResourceSource {
    /// The component was listed directly in the query as a [`Query::Resource`],
    /// [`Query::Message`], or [`Query::Bundle`].
    Query,
    /// The component was included as a child of a dependent bundle through
    /// [`Query::AndDependents`].
    DependentBundle,
    /// The component was included as a dependent of another component through
    /// [`Query::AndDependents`].
    Dependent {
        dependency: ComponentId,
        info: RemovableCounts,
    },
}

impl ResourceSource {
    /// Returns `true` if this is a [`Self::Dependent`] for a removable dependency.
    #[must_use]
    pub const fn is_removable_dependency(&self) -> bool {
        matches!(self, Self::Dependent { info, .. } if info.removable())
    }
}

// -- creating load plans -- //

/// Data on which components loaded successfully.
#[derive(Default, Debug)]
#[cfg(feature = "runtime-loading")]
pub struct LoadResults {
    pub resource_result: LoadResult,
    pub message_result: LoadResult,
}

impl LoadResults {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.resource_result.failed.is_empty() && self.message_result.failed.is_empty()
    }
}

impl Display for LoadResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "failed to load resources {:?} and messages {:?}",
            &self.resource_result.failed, &self.message_result.failed
        )
    }
}

/// Data on which components of a single type loaded successfully.
#[derive(Default, Debug)]
#[cfg(feature = "runtime-loading")]
pub struct LoadResult {
    pub loaded: HashSet<NamespacedId>,
    pub failed: HashSet<NamespacedId>,
}

/// Values that are used while figuring out how to load a submap.
#[derive(Default, Debug)]
#[cfg(feature = "runtime-loading")]
struct LoadState {
    plans: Vec<LoadPlan>,
    resource_result: LoadResult,
    message_result: LoadResult,
}

/// A plan on how to load a bundle.
#[cfg(feature = "runtime-loading")]
#[derive(Debug)]
struct LoadPlan {
    init: bundle::SharedInitFn,
    load_type: LoadType,
    parent_bundles: Vec<NamespacedId>,
}

/// Determines which parts of a bundle to load.
#[cfg(feature = "runtime-loading")]
#[derive(Debug)]
enum LoadType {
    Partial {
        resources: HashSet<NamespacedId>,
        messages: HashSet<NamespacedId>,
    },
    Complete,
}

#[cfg(feature = "runtime-loading")]
impl LoadResult {
    /// Combine two sets of load results, potentially making some previous load failures successes.
    fn extend(&mut self, other: &Self) {
        self.loaded = &self.loaded | &other.loaded;
        self.failed = &self.failed | &other.failed;
        self.failed = &self.failed - &self.loaded;
    }

    /// Changes all failures to successes and returns them.
    fn load_failed(&mut self) -> HashSet<NamespacedId> {
        let failed = std::mem::take(&mut self.failed);
        self.loaded = &self.loaded | &failed;
        failed
    }
}

#[cfg(feature = "runtime-loading")]
impl LoadState {
    /// Combine two sets of load states, loading both sides.
    fn add(&mut self, other: Self) {
        self.plans.extend(other.plans);
        self.resource_result.extend(&other.resource_result);
        self.message_result.extend(&other.message_result);
    }

    /// Returns `true` if no loads failed.
    fn success(&self) -> bool {
        self.resource_result.failed.is_empty() && self.message_result.failed.is_empty()
    }
}

#[cfg(feature = "runtime-loading")]
impl LoadPlan {
    /// Build this plan into a [`Bundle`], using the stored [`bundle::SharedInitFn`].
    fn build(self) -> Bundle {
        let mut bundle = self.init.call();

        if let LoadType::Partial {
            resources,
            messages,
        } = &self.load_type
        {
            bundle = bundle.filter_resources(resources);
            bundle = bundle.filter_messages(messages);
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

#[cfg(feature = "runtime-loading")]
impl SubMap {
    /// Returns the plans on how to load this submap back into the app as well as which components
    /// can or can't be loaded.
    fn load_plans(&self) -> LoadState {
        let mut result = LoadState::default();

        let mut parents = Vec::new();
        for (id, bundle) in &self.root_bundles {
            let new_result = bundle.load_plans(id, &mut parents);
            result.add(new_result);
        }

        result
    }
}

#[cfg(feature = "runtime-loading")]
impl SubMapBundle {
    /// Returns the plans on how to load this bundle back into the app as well as which components
    /// can or can't be loaded.
    fn load_plans(&self, id: &NamespacedIdRef, parents: &mut Vec<NamespacedId>) -> LoadState {
        let resources: HashSet<_> = self
            .descendent_resources()
            .filter(|(_, info)| !info.iter().any(ResourceSource::is_removable_dependency))
            .map(|(id, _info)| id)
            .cloned()
            .collect();
        let messages: HashSet<_> = self
            .descendent_messages()
            .filter(|(_, info)| !info.iter().any(ResourceSource::is_removable_dependency))
            .map(|(id, _info)| id)
            .cloned()
            .collect();

        if self.complete
            && let Some(init) = &self.init
        {
            return LoadState {
                plans: vec![LoadPlan {
                    init: init.clone(),
                    load_type: LoadType::Complete,
                    parent_bundles: parents.clone(),
                }],
                resource_result: LoadResult {
                    loaded: resources,
                    ..Default::default()
                },
                message_result: LoadResult {
                    loaded: messages,
                    ..Default::default()
                },
            };
        }

        let mut result = LoadState::default();
        result.resource_result.failed = resources;
        result.message_result.failed = messages;

        parents.push(id.to_owned());
        for (child_id, child_bundle) in &self.bundles {
            let child_result = child_bundle.load_plans(child_id, parents);
            result.add(child_result);
        }
        parents.pop();

        if !result.success()
            && let Some(init) = &self.init
        {
            let resources = result.resource_result.load_failed();
            let messages = result.message_result.load_failed();
            result.plans.push(LoadPlan {
                init: init.clone(),
                load_type: LoadType::Partial {
                    resources,
                    messages,
                },
                parent_bundles: parents.clone(),
            });
        }

        result
    }
}

// -- loading and unloading -- //

/// Methods for unloading and reloading components in this submap from a registry.
impl SubMap {
    /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    /// may never return if a resource can't be unloaded or a dependent isn't specified.
    pub async fn unload_in(&self, registry: &Registry) {
        self.unload_inner(&registry.stopper(), &mut registry.event_receiver())
            .await;
    }

    // /// Unloads all resources in this map. This waits until all resources are unloaded, and as such
    // /// may never return if a resource can't be unloaded or a dependent isn't specified.
    // pub async fn unload_with(&self, stopper: &Stopper, events: &EventReceiver) {
    //     self.unload_inner(stopper, &mut events.resubscribe()).await;
    // }

    async fn unload_inner(&self, stopper: &Stopper, events: &mut EventReceiver) {
        let mut components_to_unload = hashbrown::HashSet::new();
        let mut dependencies_to_remove = hashbrown::HashSet::new();

        let resources = self
            .descendent_resources()
            .map(|(id, sources)| (ComponentIdRef::Resource(id).into_owned(), sources));
        let messages = self
            .descendent_messages()
            .map(|(id, sources)| (ComponentIdRef::Message(id).into_owned(), sources));
        for (resource, sources) in resources.chain(messages) {
            let mut current_dependencies = Vec::new();
            let mut should_unload = false;

            for source in sources {
                if let ResourceSource::Dependent { dependency, info } = source
                    && info.removable()
                {
                    current_dependencies.push((resource.clone(), dependency.to_owned()));
                } else {
                    should_unload = true;
                    break;
                }
            }

            if should_unload || sources.is_empty() {
                components_to_unload.insert(resource.clone());
            } else {
                dependencies_to_remove.extend(current_dependencies);
            }
        }

        if components_to_unload.is_empty() && dependencies_to_remove.is_empty() {
            return;
        }

        events.mark_seen();

        for component in &components_to_unload {
            if let ComponentId::Resource(resource) = component {
                stopper.stop_id(resource);
            }
        }

        for (dependent, dependency) in &dependencies_to_remove {
            stopper.remove_dependency(dependency::Link {
                dependent: dependent.clone(),
                dependency: dependency.clone(),
            });
        }

        let mut loaded_components = components_to_unload.clone();
        let mut loaded_dependencies = dependencies_to_remove.clone();
        while let Some(event) = events.recv().await {
            match event {
                RegistryEvent::Loaded(id) => {
                    let id = ComponentId::Resource(id);
                    if components_to_unload.contains(&id) {
                        loaded_components.insert(id);
                    }
                }
                RegistryEvent::TakeMessageLoaded { id, .. }
                | RegistryEvent::ListenMessageLoaded { id, .. } => {
                    let id = ComponentId::Message(id);
                    if components_to_unload.contains(&id) {
                        loaded_components.insert(id);
                    }
                }
                RegistryEvent::Unloaded(id) => {
                    loaded_components.remove(&id);

                    if loaded_components.is_empty() && loaded_dependencies.is_empty() {
                        return;
                    }
                }
                RegistryEvent::NewDependency(dependency::Link {
                    dependency,
                    dependent,
                }) => {
                    let entry = (dependent, dependency);
                    if dependencies_to_remove.contains(&entry) {
                        loaded_dependencies.insert(entry);
                    }
                }
                RegistryEvent::RemovedDependency(dependency::Link {
                    dependency,
                    dependent,
                }) => {
                    loaded_dependencies.remove(&(dependent, dependency));

                    if loaded_components.is_empty() && loaded_dependencies.is_empty() {
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
    /// this map could successfully be loaded.
    ///
    /// # Errors
    ///
    /// - If a resource could not be added to the registry
    ///     - Note that this doesn't update the load result, as the result only tracks which
    ///       resources have init functions.
    pub async fn reload_in(&self, registry: &mut Registry) -> Result<LoadResults, (App, Error)> {
        let mut app = registry.new_app();

        let LoadState {
            plans,
            resource_result,
            message_result,
        } = self.load_plans();

        for plan in plans {
            let bundle = plan.build();
            app = app.with_bundle(bundle);
        }

        app.add_to_registry(registry).await?;

        Ok(LoadResults {
            resource_result,
            message_result,
        })
    }
}

// rust has trouble finding out this type in [`SubMapBundle::descendent_resources`], so we have to
// write the whole thing out ourselves ;-;
#[allow(clippy::type_complexity)]
struct DescendentComponents<'a> {
    iter: std::iter::Chain<
        std::collections::hash_map::Iter<'a, NamespacedId, HashSet<ResourceSource>>,
        std::iter::FlatMap<
            std::collections::hash_map::Values<'a, NamespacedId, SubMapBundle>,
            Box<Self>,
            fn(&'a SubMapBundle) -> Box<Self>,
        >,
    >,
}

impl<'a> Iterator for DescendentComponents<'a> {
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
