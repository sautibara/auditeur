use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    hash::Hash,
    ops::{AddAssign, Deref, DerefMut, SubAssign},
    sync::Arc,
};

use hashbrown::HashMap;
use tokio::sync::{broadcast, oneshot};

use crate::{
    app::{
        AppFn, AppFnOnce, AppFuture, AppMutFnOnce, AppMutFuture, OneshotSender, Owned,
        component::{AppComponentsMut, AppFnComponentsMut},
        event::{DependencyLifecycle, Event},
        registrar::ErasedComponent,
    },
    children::Child,
    component::{ComponentIdPath, IdentifiedComponentType, registry::Registries},
    input::CloneIn,
    prelude::*,
};

use super::event::ComponentLifecycle;

#[derive(Default, Clone, Debug)]
pub struct Map {
    pub components: im::HashMap<AddedComponentId, MapComponent>,
    pub dependencies: im::HashMap<AddedDependencyId, MapDependency>,

    pub listeners: im::HashMap<ListenerId, Listener>,
}

#[derive(Clone, Debug, Default)]
pub struct MapComponent {
    pub parent: Option<AddedComponentId>,
    pub loads: im::HashSet<LoadIndex>,
    pub load_index: LoadIndex,
}

#[derive(Clone, Debug, Default)]
pub struct MapDependency {
    pub loads: im::HashMap<DependencyLoads, DependencyTypeCounts>,
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub enum ListenerId {
    Component {
        id: AddedComponentId,
        change: ComponentStateChange,
    },
    Dependency {
        id: LoadedDependencyId,
        change: DependencyStateChange,
    },
    Registry {
        id: LoadedRegistryId,
        change: RegistryStateChange,
    },
    OnAdd(AddedComponentId),
    OnLoad(LoadedComponentId),
    OnUnload(LoadedComponentId),
    Release(Request),
}

impl ListenerId {
    fn deactivate_component(id: LoadedComponentId) -> Self {
        Self::Component {
            id: id.component,
            change: ComponentStateChange::Deactivate(id.index),
        }
    }

    fn unload_component(id: LoadedComponentId) -> Self {
        Self::Component {
            id: id.component,
            change: ComponentStateChange::Unload(id.index),
        }
    }

    const fn remove_component(id: AddedComponentId) -> Self {
        Self::Component {
            id,
            change: ComponentStateChange::Remove,
        }
    }

    const fn weaken_dependency(id: LoadedDependencyId) -> Self {
        Self::Dependency {
            id,
            change: DependencyStateChange::Weaken,
        }
    }

    const fn remove_dependency(id: LoadedDependencyId) -> Self {
        Self::Dependency {
            id,
            change: DependencyStateChange::Remove,
        }
    }

    const fn remove_registry(id: LoadedRegistryId) -> Self {
        Self::Registry {
            id,
            change: RegistryStateChange::Remove,
        }
    }

    const fn sticky(&self) -> bool {
        matches!(
            self,
            Self::Dependency { .. }
                | Self::Component {
                    change: ComponentStateChange::Remove,
                    ..
                }
                | Self::OnAdd(_)
                | Self::OnLoad(_)
                | Self::OnUnload(_)
        )
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub enum ComponentStateChange {
    Deactivate(LoadIndex),
    Unload(LoadIndex),
    Remove,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LoadIndex(usize);

impl ComponentStateChange {
    const fn lifecycle_on_add(self) -> ComponentLifecycle {
        match self {
            Self::Remove => ComponentLifecycle::Add,
            Self::Unload(_) => ComponentLifecycle::Load,
            Self::Deactivate(_) => ComponentLifecycle::Active,
        }
    }

    const fn lifecycle_on_remove(self) -> ComponentLifecycle {
        match self {
            Self::Deactivate(_) => ComponentLifecycle::Unloading,
            Self::Unload(_) => ComponentLifecycle::Unload,
            Self::Remove => ComponentLifecycle::Remove,
        }
    }
}

impl Display for LoadIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub enum DependencyStateChange {
    Remove,
    Weaken,
}

impl DependencyStateChange {
    const fn lifecycle_on_add(self) -> Option<DependencyLifecycle> {
        match self {
            Self::Remove => Some(DependencyLifecycle::Add),
            Self::Weaken => None,
        }
    }

    const fn lifecycle_on_remove(self) -> Option<DependencyLifecycle> {
        match self {
            Self::Remove => Some(DependencyLifecycle::Remove),
            Self::Weaken => None,
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub enum RegistryStateChange {
    Remove,
}

#[derive(Clone, Default, Debug)]
pub struct Listener {
    pub blocking: im::HashSet<ListenerId>,
    pub blocked_on: im::HashSet<ListenerId>,

    pub requests: im::HashSet<Request>,
    pub sticky: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Channels<'a> {
    pub event_sender: &'a broadcast::Sender<Event>,
    pub method_sender: &'a flume::Sender<super::Method>,
}

impl Map {
    /// Validates that the component with the id `id` is currently in the map.
    pub(super) fn validate_id(&self, id: ComponentId) -> Result<AddedComponentId, ComponentId> {
        let id = AddedComponentId(id);
        if self.components.contains_key(&id) {
            Ok(id)
        } else {
            Err(id.into_id())
        }
    }

    pub(super) fn add_component(
        &mut self,
        id: ComponentId,
        channels: Channels<'_>,
    ) -> Result<AddedComponentId, Error> {
        let id = AddedComponentId(id);

        let entry = self.components.entry(id.clone());
        let im::hashmap::Entry::Vacant(entry) = entry else {
            return Err(Error::NameCollision { id: id.into_id() });
        };

        entry.insert(MapComponent::default());

        let remove_component_id = ListenerId::remove_component(id.clone());
        let registry_id = if *id == Registries::component_id() {
            // HACK: load registry after adding only for the registries registry
            // (so that it can refer to itself)
            self.add_listener(&remove_component_id, channels);
            self.load_registry(id.registry, channels)
                .expect("loading Registries registry shouldn't ever fail")
        } else {
            let registry_id = (self.load_registry(id.registry, channels)).map_err(|_| {
                self.components.remove(&id);
                Error::RegistryNotFound {
                    component: id.clone_id(),
                }
            })?;

            self.add_listener(&remove_component_id, channels);

            registry_id
        };

        let remove_registry_id = ListenerId::remove_registry(registry_id);
        self.add_listener_block(remove_registry_id, remove_component_id, channels);

        Ok(id)
    }

    pub(super) fn adopt(
        &mut self,
        child: AddedComponentId,
        parent: AddedComponentId,
        channels: Channels<'_>,
    ) {
        let component_info = &mut self.components[&child];
        let old_parent = component_info.parent.replace(parent.clone());

        self.add_listener_block(
            ListenerId::remove_component(parent),
            ListenerId::remove_component(child.clone()),
            channels,
        );

        if let Some(old_parent) = old_parent {
            self.remove_listener_block(
                &ListenerId::remove_component(old_parent),
                &ListenerId::remove_component(child),
                channels,
            );
        }
    }

    fn ignore_lifecycle_callbacks(id: &ComponentId) -> bool {
        *id == App::component_id() || id.registry == Registries::id()
    }

    fn finish_on_add(&mut self, id: AddedComponentId, channels: Channels<'_>) {
        self.release_stick(&ListenerId::OnAdd(id), channels);
    }

    fn finish_on_load(&mut self, id: LoadedComponentId, channels: Channels<'_>) {
        self.release_stick(&ListenerId::OnLoad(id), channels);
    }

    fn finish_unload(&mut self, id: LoadedComponentId, channels: Channels<'_>) {
        self.release_stick(&ListenerId::OnUnload(id), channels);
    }

    fn spawn_on_add(&mut self, id: AddedComponentId, channels: Channels<'_>) {
        if Self::ignore_lifecycle_callbacks(&id) {
            return;
        }

        let app_id = self.load_component(AddedComponentId::app(), channels);

        // Add the listener w/ its blocks.
        let listener = ListenerId::OnAdd(id.clone());
        self.add_listener(&listener, channels);
        self.add_listener_block(
            ListenerId::deactivate_component(app_id),
            listener.clone(),
            channels,
        );
        self.add_listener_block(
            ListenerId::remove_component(id.clone()),
            listener.clone(),
            channels,
        );

        // Spawn the load.
        let _ = channels.method_sender.send(super::Method::Registries(
            super::registrar::Method::SpawnOnAdd(id),
        ));
    }

    fn spawn_on_load(&mut self, id: LoadedComponentId, channels: Channels<'_>) {
        if Self::ignore_lifecycle_callbacks(&id.component) {
            return;
        }

        let app_id = self.load_component(AddedComponentId::app(), channels);

        // Add the listener w/ its blocks.
        let listener = ListenerId::OnLoad(id.clone());
        self.add_listener(&listener, channels);
        self.add_listener_block(
            ListenerId::deactivate_component(app_id),
            listener.clone(),
            channels,
        );
        self.add_listener_block(
            ListenerId::deactivate_component(id.clone()),
            listener.clone(),
            channels,
        );

        // Spawn the load.
        let _ = channels.method_sender.send(super::Method::Registries(
            super::registrar::Method::SpawnOnLoad(id),
        ));
    }

    fn spawn_unload(&mut self, id: LoadedComponentId, channels: Channels<'_>) {
        // Increment the load index, since we've started unloading this one.
        let current_load_index = &mut self.components[&id.component].load_index;
        *current_load_index = LoadIndex(current_load_index.0 + 1);

        if *id.component == App::component_id() {
            // We can't call SpawnUnload for the app, since SpawnUnload uses an app
            // handle (and thus would load the app again).

            // Request a remove of the app.
            let remove_id = ListenerId::remove_component(AddedComponentId::app());
            if self.listeners.contains_key(&remove_id) {
                let (releaser, _) = request_channel();
                self.request(&remove_id, &releaser, channels);
            }

            // Release the app's event listener.
            let _ = channels
                .method_sender
                .send(super::Method::ReleaseEventListener);

            return;
        } else if Self::ignore_lifecycle_callbacks(&id.component) {
            return;
        }

        // Add the listener w/ its blocks.
        // No need to keep the app loaded because it'll stay loaded as long as the component is
        //   (see [`Self::load_component`]).
        let listener = ListenerId::OnUnload(id.clone());
        self.add_listener(&listener, channels);
        self.add_listener_block(
            ListenerId::unload_component(id.clone()),
            listener.clone(),
            channels,
        );

        // Spawn the unload.
        let _ = channels.method_sender.send(super::Method::Registries(
            super::registrar::Method::SpawnUnload(id),
        ));
    }

    fn increment_dependency(
        &mut self,
        part: DependencyPart,
        channels: Channels<'_>,
    ) -> Dependency<()> {
        let DependencyPart { id, ty } = part;

        let dependency = self.dependencies.entry(id.id.clone()).or_default();
        let counts = dependency.loads.entry(id.loads).or_default();
        *counts += ty;
        let counts = *counts;

        let remove = ListenerId::remove_dependency(id.clone());
        if !self.listeners.contains_key(&remove) {
            let added = self.add_listener(&remove, channels);
            assert!(added);

            self.add_listener_block(
                ListenerId::unload_component(id.dependency()),
                remove.clone(),
                channels,
            );

            self.add_listener_block(
                ListenerId::unload_component(id.dependent()),
                remove.clone(),
                channels,
            );
        }

        let weaken = ListenerId::weaken_dependency(id.clone());
        if !counts.effective_type().is_weak() && !self.listeners.contains_key(&weaken) {
            let added = self.add_listener(&weaken, channels);
            assert!(added);

            self.add_listener_block(
                ListenerId::deactivate_component(id.dependency()),
                weaken.clone(),
                channels,
            );

            let deactivate_dependent = ListenerId::deactivate_component(id.dependent());
            if self.listeners.contains_key(&deactivate_dependent) {
                self.add_listener_block(weaken, deactivate_dependent, channels);
            }
        }

        Dependency {
            value: (),
            handle: DependencyHandle {
                id: DependencyPart { id, ty },
                sender: Some(channels.method_sender.clone()),
            },
        }
    }

    fn decrement_dependency(&mut self, part: &DependencyPart, channels: Channels<'_>) {
        let DependencyPart { id, ty } = part;

        let dependency = &mut self.dependencies[&id.id];
        let counts = &mut dependency.loads[&id.loads];
        *counts -= *ty;
        let counts = *counts;

        let weaken = ListenerId::weaken_dependency(id.clone());
        if (counts.effective_type().is_weak() || counts.is_zero())
            && let Some(listener) = self.listeners.get(&weaken)
        {
            for blocker in &listener.blocked_on.clone() {
                self.remove_listener_block(&weaken, blocker, channels);
            }
            self.release_stick(&weaken, channels);
        }

        let remove = ListenerId::remove_dependency(id.clone());
        if counts.is_zero() && self.listeners.contains_key(&remove) {
            self.release_stick(&remove, channels);
        }
    }

    pub(super) fn load_component(
        &mut self,
        id: AddedComponentId,
        channels: Channels<'_>,
    ) -> LoadedComponentId {
        let load_index = self.components[&id].load_index;
        let id = id.loaded_unchecked(load_index);

        let unload_id = ListenerId::unload_component(id.clone());
        let added = self.add_listener(&unload_id, channels);
        if !added {
            // already loaded
            return id;
        }

        // hey we have another load now, add it to the list
        self.components[&id.component].loads.insert(load_index);

        let deactivate_id = ListenerId::deactivate_component(id.clone());

        let added_deactivate = self.add_listener(&deactivate_id, channels);
        assert!(
            added_deactivate,
            "component shouldn't be activated without also being loaded"
        );

        let remove_id = ListenerId::remove_component(id.component.clone());
        self.add_listener_block(unload_id.clone(), deactivate_id, channels);
        self.add_listener_block(remove_id, unload_id.clone(), channels);

        // Keep the app loaded until we finish unloading this component.
        if !Self::ignore_lifecycle_callbacks(&id.component) {
            let app_id = self.load_component(AddedComponentId::app(), channels);
            self.add_listener_block(ListenerId::unload_component(app_id), unload_id, channels);
        }

        id
    }

    fn load_registry(
        &mut self,
        id: RegistryId,
        channels: Channels<'_>,
    ) -> Result<LoadedRegistryId, RegistryId> {
        let id = LoadedRegistryId(id);
        let remove_id = ListenerId::remove_registry(id.clone());
        if self.listeners.contains_key(&remove_id) {
            return Ok(id);
        }

        let component_id = self.validate_id(id.component_id().0).map_err(|_| id.0)?;

        let added = self.add_listener(&remove_id, channels);
        assert!(added);

        let loaded_id = self.load_component(component_id, channels);
        let component_deactivate_id = ListenerId::deactivate_component(loaded_id);
        self.add_listener_block(component_deactivate_id, remove_id, channels);

        Ok(id)
    }

    fn on_app_removal(channels: Channels<'_>) {
        let _ = channels.event_sender.send(Event::Exit);
        let _ = channels.method_sender.send(super::Method::Exit);
    }

    const fn prevent_removal(listener: &ListenerId) -> bool {
        matches!(
            listener,
            ListenerId::Component {
                change: ComponentStateChange::Remove,
                ..
            } | ListenerId::Dependency { .. }
                | ListenerId::OnAdd(_)
                | ListenerId::OnLoad(_)
                | ListenerId::OnUnload(_)
        )
    }

    #[allow(clippy::needless_pass_by_ref_mut, clippy::unused_self)] // wiwiwi
    fn on_add(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        match listener {
            ListenerId::Component { id, change } => {
                let _ = channels.event_sender.send(Event::Component {
                    id: id.clone_id(),
                    lifecycle: change.lifecycle_on_add(),
                });

                match change {
                    ComponentStateChange::Remove => {
                        self.spawn_on_add(id.clone(), channels);
                    }
                    ComponentStateChange::Unload(_) => (),
                    ComponentStateChange::Deactivate(load_index) => {
                        let id = id.clone().loaded_unchecked(*load_index);
                        self.spawn_on_load(id, channels);
                    }
                }
            }
            ListenerId::Dependency { id, change } => {
                if let Some(lifecycle) = change.lifecycle_on_add() {
                    let id = id.clone().into_id();
                    let _ = channels
                        .event_sender
                        .send(Event::Dependency { id, lifecycle });
                }
            }
            ListenerId::Registry { .. } => {
                // For now, registries are an implementation detail of the map.
                // They're shown in it, but events aren't given.
            }
            ListenerId::Release(_)
            | ListenerId::OnAdd(_)
            | ListenerId::OnLoad(_)
            | ListenerId::OnUnload(_) => (),
        }
    }

    fn on_remove(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        match listener {
            ListenerId::Component { id, change } => {
                let _ = channels.event_sender.send(Event::Component {
                    id: id.clone_id(),
                    lifecycle: change.lifecycle_on_remove(),
                });

                match change {
                    ComponentStateChange::Remove => {
                        self.components.remove(id);

                        // Check if we're removing the app, and if so, exit.
                        if **id == App::component_id() {
                            Self::on_app_removal(channels);
                        }
                    }
                    ComponentStateChange::Unload(load_index) => {
                        self.components[id].loads.remove(load_index);
                    }
                    ComponentStateChange::Deactivate(load_index) => {
                        let id = id.clone().loaded_unchecked(*load_index);
                        self.spawn_unload(id, channels);
                    }
                }
            }
            ListenerId::Dependency { id, change } => {
                if let Some(lifecycle) = change.lifecycle_on_remove() {
                    let id = id.clone().into_id();
                    let _ = channels
                        .event_sender
                        .send(Event::Dependency { id, lifecycle });
                }

                if matches!(change, DependencyStateChange::Remove) {
                    let entry = self.dependencies.entry(id.id.clone());
                    let im::hashmap::Entry::Occupied(mut entry) = entry else {
                        panic!("dependency should exist in map since we have its loaded id");
                    };

                    entry.get_mut().loads.remove(&id.loads);
                    if entry.get().loads.is_empty() {
                        entry.remove();
                    }
                }
            }
            ListenerId::Registry { .. } => {
                // For now, registries are an implementation detail of the map.
                // They're shown in it, but events aren't given.
            }
            ListenerId::Release(request) => {
                let _ = channels.event_sender.send(Event::RequestSuccess {
                    request: request.clone(),
                });
                request.release();
            }
            ListenerId::OnAdd(_) | ListenerId::OnLoad(_) | ListenerId::OnUnload(_) => (),
        }
    }

    /// Returns `true` iff the request should propagate to the listener's blockers.
    fn on_request(
        &mut self,
        listener: &ListenerId,
        releaser: &Request,
        blocked_on: &im::HashSet<ListenerId>,
        new_request: bool,
        channels: Channels<'_>,
    ) {
        let propagate = move |this: &mut Self| {
            (blocked_on.iter()).for_each(|id| this.request(id, releaser, channels));
        };
        match listener {
            // component removal propagates to is children and also allows it to be removed
            ListenerId::Component {
                change: ComponentStateChange::Remove,
                ..
            } => {
                self.release_stick(listener, channels);
                propagate(self);
            }
            // component unloading only propagates to its deactivation
            ListenerId::Component {
                change: ComponentStateChange::Unload(index),
                id,
            } => {
                let id = id.clone().loaded_unchecked(*index);
                let id = ListenerId::deactivate_component(id);
                if self.listeners.contains_key(&id) {
                    self.request(&id, releaser, channels);
                }
            }
            // component deactivation propagates to all its non-weak dependencies
            // registry removal propagates to all its children
            // release propagates to its blockers
            ListenerId::Component {
                change: ComponentStateChange::Deactivate(_),
                ..
            }
            | ListenerId::Registry { .. }
            | ListenerId::Release(_)
            | ListenerId::OnAdd(_)
            | ListenerId::OnLoad(_)
            | ListenerId::OnUnload(_) => propagate(self),
            // dependency removal propagates to its weaken listener, if it exists
            ListenerId::Dependency {
                id,
                change: DependencyStateChange::Remove,
            } => {
                let id = ListenerId::weaken_dependency(id.clone());
                if self.listeners.contains_key(&id) {
                    self.request(&id, releaser, channels);
                }
            }
            // dependency weakening propagates in different ways
            ListenerId::Dependency {
                id,
                change: DependencyStateChange::Weaken,
            } => {
                let dependency = self.dependencies[&id.id].loads[&id.loads];
                // unremovable dependencies propagate to the dependent
                if dependency.unremovable > 0 {
                    propagate(self);
                }
                // removable dependencies send an event
                if dependency.removable > 0 && new_request {
                    let id = id.clone().into_id();
                    let _ = channels
                        .event_sender
                        .send(Event::RequestDependencyRemoval { id });
                }
            }
        }
    }

    fn add_listener(&mut self, listener: &ListenerId, channels: Channels<'_>) -> bool {
        let im::hashmap::Entry::Vacant(entry) = self.listeners.entry(listener.clone()) else {
            return false;
        };

        let prevent_removal = Self::prevent_removal(entry.key());
        entry.insert(Listener {
            sticky: prevent_removal,
            ..Default::default()
        });

        self.on_add(listener, channels);
        true
    }

    fn release_stick(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        self.listeners[listener].sticky = false;
        self.check_removal(listener, channels);
    }

    fn check_removal(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        let im::hashmap::Entry::Occupied(entry) = self.listeners.entry(listener.clone()) else {
            return;
        };

        if entry.get().sticky || !entry.get().blocked_on.is_empty() {
            return;
        }

        let (blocked_on, info) = entry.remove_entry();

        for listener in &info.blocking {
            self.remove_listener_block_partial(listener, &blocked_on);
        }

        self.on_remove(&blocked_on, channels);

        for listener in &info.blocking {
            self.check_removal(listener, channels);
        }

        // println!("{self:#?}");
    }

    fn request(&mut self, id: &ListenerId, releaser: &Request, channels: Channels<'_>) {
        let listener = (self.listeners.get_mut(id))
            .expect("should not request a listener that does not exist");

        let request_is_new = listener.requests.is_empty();

        let old_request = listener.requests.insert(releaser.clone());
        if old_request.is_some() {
            return;
        }

        // clone this here to release our mutable reference
        let blocked_on = listener.blocked_on.clone();
        self.on_request(id, releaser, &blocked_on, request_is_new, channels);
    }

    #[allow(clippy::needless_pass_by_value)]
    fn add_listener_block(
        &mut self,
        blocked: ListenerId,
        blocked_on: ListenerId,
        channels: Channels<'_>,
    ) {
        let blocked_info = &mut self.listeners[&blocked];
        blocked_info.blocked_on.insert(blocked_on.clone());
        let requests = blocked_info.requests.clone();

        let blocked_on_info = &mut self.listeners[&blocked_on];
        blocked_on_info.blocking.insert(blocked.clone());

        let set = im::HashSet::from_iter([blocked_on]);
        for releaser in requests {
            self.on_request(&blocked, &releaser, &set, false, channels);
        }
    }

    fn remove_listener_block(
        &mut self,
        blocked: &ListenerId,
        blocked_on: &ListenerId,
        channels: Channels<'_>,
    ) {
        self.remove_listener_block_partial(blocked, blocked_on);
        self.check_removal(blocked, channels);
    }

    fn remove_listener_block_partial(&mut self, blocked: &ListenerId, blocked_on: &ListenerId) {
        let blocked_info = &mut self.listeners[blocked];
        blocked_info.blocked_on.remove(blocked_on);

        if let Some(blocked_on_info) = self.listeners.get_mut(blocked_on) {
            blocked_on_info.blocking.remove(blocked);
        }
    }
}

// Public-facing API
impl Map {
    #[must_use]
    pub fn descendants_of(
        &self,
        id: impl IntoIterator<Item = ComponentId>,
    ) -> std::collections::HashSet<ComponentId> {
        let mut descendants = std::collections::HashSet::new();
        let mut queue: Vec<_> = id.into_iter().collect();

        while let Some(id) = queue.pop() {
            let remove = ListenerId::remove_component(AddedComponentId(id.clone()));
            let Some(listener) = self.listeners.get(&remove) else {
                continue;
            };

            descendants.insert(id);
            queue.extend(listener.blocked_on.iter().filter_map(|listener| {
                // WARN: this assumes that a component remove will only be blocked on other
                // component removes if they're for its children.
                if let ListenerId::Component {
                    id,
                    change: ComponentStateChange::Remove,
                } = listener
                {
                    Some(id.clone().into_id())
                } else {
                    None
                }
            }));
        }

        descendants
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RequestAction {
    RemoveComponent(ComponentId),
    UnloadComponent(ComponentId),
    RemoveDependency(DependencyId),
}

impl Display for RequestAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemoveComponent(id) => write!(f, "remove component '{id}'"),
            Self::UnloadComponent(id) => write!(f, "unload component '{id}'"),
            Self::RemoveDependency(id) => write!(f, "remove dependency '{id}'"),
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct RequestSet {
    pub set: hashbrown::HashSet<RequestAction>,
}

impl RequestSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn remove(component: ComponentId) -> Self {
        Self::new().and_remove(component)
    }

    #[must_use]
    pub fn and_remove(mut self, component: ComponentId) -> Self {
        self.set.insert(RequestAction::RemoveComponent(component));
        self
    }

    #[must_use]
    pub fn unload(component: ComponentId) -> Self {
        Self::new().and_unload(component)
    }

    #[must_use]
    pub fn and_unload(mut self, component: ComponentId) -> Self {
        self.set.insert(RequestAction::UnloadComponent(component));
        self
    }

    #[must_use]
    pub fn remove_dependency(dependency: DependencyId) -> Self {
        Self::new().and_remove_dependency(dependency)
    }

    #[must_use]
    pub fn and_remove_dependency(mut self, dependency: DependencyId) -> Self {
        self.set.insert(RequestAction::RemoveDependency(dependency));
        self
    }
}

impl FromIterator<RequestAction> for RequestSet {
    fn from_iter<T: IntoIterator<Item = RequestAction>>(iter: T) -> Self {
        Self {
            set: hashbrown::HashSet::from_iter(iter),
        }
    }
}

impl Owned {
    pub(super) fn add_children(
        &mut self,
        children: Children,
        sender: &flume::Sender<super::Method>,
    ) {
        struct AddChildren<'a> {
            owned: &'a mut Owned,
            sender: &'a flume::Sender<super::Method>,
            children: HashMap<ComponentId, Child>,
            errored: HashSet<ComponentId>,
        }

        impl<'a> AddChildren<'a> {
            fn new(
                owned: &'a mut Owned,
                children: Children,
                sender: &'a flume::Sender<super::Method>,
            ) -> Self {
                Self {
                    owned,
                    sender,
                    errored: HashSet::new(),
                    children: children
                        .into_children()
                        .map(|child| (child.component.object.component_id_dyn(), child))
                        .collect(),
                }
            }

            /// Returns `true` if the component has been successfully added to the app.
            fn add(&mut self, id: ComponentId) -> bool {
                !self.errored.contains(&id)
                    && self.children.remove(&id).is_none_or(|child| {
                        child.parent.clone().is_none_or(|parent| self.add(parent))
                            && self.add(id.registry_id())
                            && self.actually_add(child, id)
                    })
            }

            fn actually_add(&mut self, child: Child, id: ComponentId) -> bool {
                let res = self.owned.add_child(child, self.sender);
                let res = self.owned.drain_result(res);
                if res.is_none() {
                    self.errored.insert(id);
                }
                res.is_some()
            }
        }

        let mut adder = AddChildren::new(self, children, sender);
        #[expect(clippy::needless_collect)] // needed b/c we're mutating adder
        for id in adder.children.keys().cloned().collect::<Vec<_>>() {
            adder.add(id);
        }
    }

    pub(super) fn add_child(
        &mut self,
        child: Child,
        sender: &flume::Sender<super::Method>,
    ) -> Result<AddedComponentId, Error> {
        let parent = (child.parent.clone())
            .map(|parent| self.map.validate_id(parent))
            .transpose()
            .map_err(|parent| Error::ParentNotFound {
                child: child.component.object.component_id_dyn(),
                parent,
            })?;

        self.add_child_validated(child.component, parent, sender)
    }

    pub(super) fn add_child_validated(
        &mut self,
        component: ErasedComponent,
        parent: Option<AddedComponentId>,
        sender: &flume::Sender<super::Method>,
    ) -> Result<AddedComponentId, Error> {
        let id = component.object.component_id_dyn();

        let channels = Channels {
            event_sender: &self.event_sender,
            method_sender: sender,
        };

        let id = self.map.add_component(id, channels)?;

        if let Some(parent) = parent {
            self.map.adopt(id.clone(), parent, channels);
        }

        self.registrar.add(component);

        Ok(id)
    }

    pub(super) fn increment_dependency(
        &mut self,
        part: DependencyPart,
        sender: &flume::Sender<super::Method>,
    ) -> Dependency<()> {
        self.map.increment_dependency(
            part,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        )
    }

    fn decrement_dependency(
        &mut self,
        part: &DependencyPart,
        sender: &flume::Sender<super::Method>,
    ) {
        self.map.decrement_dependency(
            part,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );
    }

    pub(super) fn wait_for(
        &mut self,
        request: RequestSet,
        sender: &flume::Sender<super::Method>,
    ) -> (ListenerId, Waiter) {
        let (releaser, waiter) = request_channel();
        let release_id = ListenerId::Release(releaser);
        let added = self.map.add_listener(
            &release_id,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );
        assert!(
            added,
            "added releaser shouldn't be a duplicate, since we just created it"
        );

        for request in request.set {
            let mut success = false;

            let mut add = |this: &mut Self, listener: ListenerId| {
                if !this.map.listeners.contains_key(&listener) {
                    return;
                }

                this.map.add_listener_block(
                    release_id.clone(),
                    listener,
                    Channels {
                        event_sender: &this.event_sender,
                        method_sender: sender,
                    },
                );

                success = true;
            };

            match &request {
                RequestAction::RemoveComponent(id) => {
                    // NOTE: it's okay to assert that this is added b/c we check if the listener
                    // exists below
                    let id = AddedComponentId(id.clone());
                    add(self, ListenerId::remove_component(id));
                }
                RequestAction::UnloadComponent(id) => {
                    let id = AddedComponentId(id.clone());

                    if let Some(component) = self.map.components.get(&id) {
                        (component.loads.clone().iter().copied())
                            .map(|index| {
                                ListenerId::unload_component(id.clone().loaded_unchecked(index))
                            })
                            .for_each(|listener| add(self, listener));
                    }
                }
                RequestAction::RemoveDependency(id) => {
                    let id = AddedDependencyId::new_unchecked(id.clone());

                    if let Some(dependency) = self.map.dependencies.get(&id) {
                        (dependency.loads.clone().keys().copied())
                            .map(|loads| {
                                ListenerId::remove_dependency(LoadedDependencyId {
                                    id: id.clone(),
                                    loads,
                                })
                            })
                            .for_each(|listener| add(self, listener));
                    }
                }
            }

            if !success {
                self.add_error(Error::InvalidRequest { request });
            }
        }

        // Now that we've gone through the entire request set, we should check if the listener
        // can already be removed in the case that the request set was empty.
        self.map.check_removal(
            &release_id,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );

        (release_id, waiter)
    }

    pub(super) fn request(
        &mut self,
        request: RequestSet,
        sender: &flume::Sender<super::Method>,
    ) -> (ListenerId, Waiter) {
        let (release_id, waiter) = self.wait_for(request, sender);
        if self.map.listeners.contains_key(&release_id) {
            self.map.request(
                &release_id,
                waiter.request(),
                Channels {
                    event_sender: &self.event_sender,
                    method_sender: sender,
                },
            );
        }
        (release_id, waiter)
    }
}

/// An id of a component that is guaranteed to be currently added in the map when returned by it.
///
/// Note that this guarantee is only valid at the exact moment that the id is obtained from the
/// map - keeping this id alive doesn't keep the component added.
// WARN: Users of the library should not be able to give these to prove that a component is added
// - it should only be used as a guarantee within the library.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AddedComponentId(ComponentId);

impl AddedComponentId {
    pub(crate) const fn new_inner(id: ComponentId) -> Self {
        Self(id)
    }

    fn app() -> Self {
        Self(App::component_id())
    }

    const fn loaded_unchecked(self, index: LoadIndex) -> LoadedComponentId {
        LoadedComponentId {
            component: self,
            index,
        }
    }

    #[must_use]
    pub fn clone_id(&self) -> ComponentId {
        self.deref().clone()
    }

    #[must_use]
    pub fn into_id(self) -> ComponentId {
        self.0
    }
}

impl Deref for AddedComponentId {
    type Target = ComponentId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for AddedComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

type RegistryId = &'static NamespacedIdRef;

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedRegistryId(RegistryId);

impl LoadedRegistryId {
    fn component_id(&self) -> AddedComponentId {
        AddedComponentId(ComponentId {
            registry: Registries::id(),
            component: ComponentIdPath::Static(self.0),
        })
    }
}

impl Deref for LoadedRegistryId {
    type Target = RegistryId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// An id of a specifically numbered load of a component.
///
/// This has the same guarantees as [`AddedComponentId`].
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedComponentId {
    pub component: AddedComponentId,
    pub index: LoadIndex,
}

impl LoadedComponentId {
    #[must_use]
    pub fn into_id(self) -> ComponentId {
        self.component.into_id()
    }
}

impl Display for LoadedComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' ({})", self.component, self.index)
    }
}

/// An id of a dependency that may or may not be added to the map yet.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct DependencyId {
    pub dependent: ComponentId,
    pub dependency: ComponentId,
}

impl Display for DependencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' -> '{}'", self.dependent, self.dependency)
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AddedDependencyId {
    pub dependent: AddedComponentId,
    pub dependency: AddedComponentId,
}

impl AddedDependencyId {
    fn new_unchecked(id: DependencyId) -> Self {
        Self {
            dependent: AddedComponentId(id.dependent),
            dependency: AddedComponentId(id.dependency),
        }
    }

    fn into_id(self) -> DependencyId {
        DependencyId {
            dependent: self.dependent.into_id(),
            dependency: self.dependency.into_id(),
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct DependencyLoads {
    pub dependent: LoadIndex,
    pub dependency: LoadIndex,
}

/// An id of a loaded dependency in the map.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedDependencyId {
    pub id: AddedDependencyId,
    pub loads: DependencyLoads,
}

impl LoadedDependencyId {
    #[must_use]
    pub fn new(dependent: LoadedComponentId, dependency: LoadedComponentId) -> Self {
        Self {
            id: AddedDependencyId {
                dependent: dependent.component,
                dependency: dependency.component,
            },
            loads: DependencyLoads {
                dependent: dependent.index,
                dependency: dependency.index,
            },
        }
    }

    #[must_use]
    pub fn into_id(self) -> DependencyId {
        self.id.into_id()
    }

    #[must_use]
    pub fn dependent(&self) -> LoadedComponentId {
        (self.id.dependent.clone()).loaded_unchecked(self.loads.dependent)
    }

    #[must_use]
    pub fn dependency(&self) -> LoadedComponentId {
        (self.id.dependency.clone()).loaded_unchecked(self.loads.dependency)
    }
}

impl Display for LoadedDependencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} -> {}", self.dependent(), self.dependency())
    }
}

/// An id of a part of a loaded dependency in the map.
///
/// A single dependency may be made up of multiple parts - multiple [`Dependency`] handles -
/// that all have separate [types](DependencyType). The dependency is only considered removed
/// if all parts are dropped.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct DependencyPart {
    pub id: LoadedDependencyId,
    pub ty: DependencyType,
}

/// A value that determines if a [`Dependency`] can be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyType {
    /// The dependency cannot be removed.
    Unremovable,
    /// The dependency can be removed, but the app has to ask.
    Removable,
    /// The dependency is short-lived and will be removed automatically without the app asking.
    Transient,
    /// The dependency should not keep the depended-on component alive. Instead, this dependency
    /// is expected to automatically be removed when the **depended-on component** is unloaded.
    ///
    /// Note: Don't use this unless you know what you're doing. It's very rare that you don't want
    /// to keep the depended-on component alive. Additionally, the depended-on component has to
    /// know about the dependency, which is even more rare. Instead, use [`Self::Transient`] as a
    /// good default.
    Weak,
}

impl DependencyType {
    /// Returns `true` if the dependency type is [`Weak`].
    ///
    /// [`Weak`]: DependencyType::Weak
    #[must_use]
    pub const fn is_weak(&self) -> bool {
        matches!(self, Self::Weak)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DependencyTypeCounts {
    pub unremovable: usize,
    pub removable: usize,
    pub transient: usize,
    pub weak: usize,
}

impl DependencyTypeCounts {
    const fn is_zero(&self) -> bool {
        self.unremovable == 0 && self.removable == 0 && self.transient == 0 && self.weak == 0
    }

    const fn effective_type(&self) -> DependencyType {
        if self.unremovable > 0 {
            DependencyType::Unremovable
        } else if self.removable > 0 {
            DependencyType::Removable
        } else if self.transient > 0 {
            DependencyType::Transient
        } else if self.weak > 0 {
            DependencyType::Weak
        } else {
            DependencyType::Transient
        }
    }
}

impl AddAssign<DependencyType> for DependencyTypeCounts {
    fn add_assign(&mut self, ty: DependencyType) {
        match ty {
            DependencyType::Unremovable => self.unremovable += 1,
            DependencyType::Removable => self.removable += 1,
            DependencyType::Transient => self.transient += 1,
            DependencyType::Weak => self.weak += 1,
        }
    }
}

impl SubAssign<DependencyType> for DependencyTypeCounts {
    fn sub_assign(&mut self, ty: DependencyType) {
        match ty {
            DependencyType::Unremovable => self.unremovable = self.unremovable.saturating_sub(1),
            DependencyType::Removable => self.removable = self.removable.saturating_sub(1),
            DependencyType::Transient => self.transient = self.transient.saturating_sub(1),
            DependencyType::Weak => self.weak = self.weak.saturating_sub(1),
        }
    }
}

/// A value that keeps a dependency alive as long as it lives.
///
/// Values from other components will typically be wrapped in this type to keep the other component
/// alive. A dependency is incremented when a value of this type is created and decremented when it
/// is dropped. Only when all individual dependency parts are dropped will the whole dependency be
/// removed in the app.
#[derive(Debug)]
pub struct Dependency<T> {
    pub(super) value: T,
    pub(super) handle: DependencyHandle,
}

impl<T> Dependency<T> {
    pub const fn info_of(this: &Self) -> &DependencyPart {
        &this.handle.id
    }
}

impl Dependency<()> {
    #[must_use]
    pub const fn info(&self) -> &DependencyPart {
        Self::info_of(self)
    }

    pub(crate) fn insert<T>(self, value: T) -> Dependency<T> {
        Dependency {
            value,
            handle: self.handle,
        }
    }
}

impl<T: Send> Dependency<T> {
    pub async fn clone_with<U: Send>(
        this: &Self,
        app: &mut App,
        func: impl AsyncFnOnce(&T, &mut App) -> U + Send,
    ) -> Dependency<U>
    where
        T: Sync,
    {
        let dependency = app
            .components_mut()
            .increment_dependency(this.handle.id.clone())
            .await;
        let value = func(&this.value, app).await;
        Dependency {
            value,
            handle: dependency.handle,
        }
    }

    pub(super) fn clone_with_owned<U: Send>(
        this: &Self,
        owned: &mut Owned,
        sender: &flume::Sender<super::Method>,
        func: impl FnOnce(&T, &mut Owned) -> U + Send,
    ) -> Dependency<U> {
        let dependency = owned.increment_dependency(this.handle.id.clone(), sender);
        let value = func(&this.value, owned);
        Dependency {
            value,
            handle: dependency.handle,
        }
    }

    pub(super) fn clone_in_owned(
        this: &Self,
        owned: &mut Owned,
        sender: &flume::Sender<super::Method>,
    ) -> Self
    where
        T: Clone,
    {
        Self::clone_with_owned(this, owned, sender, |val, _| val.clone())
    }

    pub(crate) async fn consume(this: Self, func: impl AsyncFnOnce(T)) {
        let Self { value, handle } = this;
        func(value).await;
        drop(handle);
    }

    pub(crate) fn release_unsafe(this: &mut Self) {
        this.handle.release_unsafe();
    }

    /// Drops this dependency using a specific app handle.
    ///
    /// This can be more efficient than the default drop implementation, but only if the app handle
    /// is the owned app handle. Otherwise, it will do the same exact thing as the automatic drop.
    ///
    /// # Panics
    ///
    /// - If `app` is not the app that this dependency was obtained in.
    pub fn drop_in(this: Self, app: &mut App) -> impl AppMutFuture<'_, Output = ()> {
        drop(this.value);
        this.handle.drop_in(app)
    }

    /// Drops this dependency in an owned app.
    ///
    /// # Panics
    ///
    /// - Potential panic if `owned` is not the app that this dependency was obtained in.
    pub(super) fn drop_in_owned(
        this: Self,
        owned: &mut Owned,
        sender: &flume::Sender<super::Method>,
    ) {
        drop(this.value);
        this.handle.drop_in_owned(owned, sender);
    }
}

impl<T> Deref for Dependency<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Dependency<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: CloneIn> CloneIn for Dependency<T> {
    async fn clone_in(&self, app: &mut App) -> Self
    where
        Self: Sized,
    {
        Self::clone_with(self, app, async |inner, app| inner.clone_in(app).await).await
    }
}

#[derive(Debug)]
pub(super) struct DependencyHandle {
    id: DependencyPart,
    // The sender is wrapped in an option to ensure that the dependency decrement happens exactly
    // once.
    sender: Option<flume::Sender<super::Method>>,
}

impl DependencyHandle {
    /// Returns a dependency handle *that doesn't keep any dependency alive*.
    pub(super) fn empty_unsafe() -> Self {
        Self {
            id: DependencyPart {
                // NOTE: the dependent and dependency don't matter, since nothing will be sent
                id: LoadedDependencyId {
                    id: AddedDependencyId {
                        dependent: AddedComponentId(App::component_id()),
                        dependency: AddedComponentId(App::component_id()),
                    },
                    loads: DependencyLoads {
                        dependent: LoadIndex(0),
                        dependency: LoadIndex(0),
                    },
                },
                ty: DependencyType::Weak,
            },
            sender: None,
        }
    }

    fn release_unsafe(&mut self) {
        // If drop_in is called, the sender will be None, so we should ignore the drop.
        let Some(sender) = self.sender.take() else {
            return;
        };

        let event = super::Method::Map(Method::DecrementDependency {
            id: self.id.clone(),
        });

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(sender.into_send_async(event));
        } else {
            let _ = sender.send(event);
        }
    }

    /// Drops this dependency using a specific app handle.
    ///
    /// This can be more efficient than the default drop implementation, but only if the app handle
    /// is the owned app handle. Otherwise, it will do the same exact thing as the automatic drop.
    ///
    /// # Panics
    ///
    /// - If `app` is not the app that this dependency was obtained in.
    fn drop_in(mut self, app: &mut App) -> impl AppMutFuture<'_, Output = ()> {
        let Some(sender) = self.sender.take() else {
            panic!("drop_in takes ownership of self, so the sender should only be taken once")
        };

        assert!(
            sender.same_channel(&app.handle.method_sender),
            "dependency should be dropped in the app it came from"
        );

        app.components_mut().decrement_dependency(self.id.clone())
    }

    /// Drops this dependency in an owned app.
    ///
    /// # Panics
    ///
    /// - Potential panic if `owned` is not the app that this dependency was obtained in.
    fn drop_in_owned(mut self, owned: &mut Owned, sender: &flume::Sender<super::Method>) {
        self.sender
            .take()
            .expect("drop_in takes ownership of self, so the sender should only be taken once");

        owned.decrement_dependency(&self.id, sender);
    }
}

impl Drop for DependencyHandle {
    fn drop(&mut self) {
        self.release_unsafe();
    }
}

#[derive(Debug)]
pub(super) enum Method {
    Get {
        sender: OneshotSender<Map>,
    },
    Request {
        set: RequestSet,
        sender: OneshotSender<Waiter>,
    },
    WaitFor {
        set: RequestSet,
        sender: OneshotSender<Waiter>,
    },
    // Dependency
    TryIncrementDependency {
        dependency: ComponentId,
        dependent: LoadedComponentId,
        ty: DependencyType,
        sender: OneshotSender<Result<Dependency<()>, Error>>,
    },
    IncrementLoadedDependency {
        dependency: AddedComponentId,
        dependent: LoadedComponentId,
        ty: DependencyType,
        sender: OneshotSender<Dependency<()>>,
    },
    IncrementDependency {
        id: DependencyPart,
        sender: OneshotSender<Dependency<()>>,
    },
    DecrementDependency {
        id: DependencyPart,
    },
    // Internals
    OnAddFinished(AddedComponentId),
    OnLoadFinished(LoadedComponentId),
    UnloadFinished(LoadedComponentId),
}

impl Method {
    pub(super) async fn run(self, app: &mut App) {
        match self {
            Self::Get { sender } => {
                let map = app.map().get().await;
                sender.send(map);
            }
            Self::Request { set, sender } => {
                let waiter = app.components_mut().request(set).await;
                sender.send(waiter);
            }
            Self::WaitFor { set, sender } => {
                let waiter = app.components_mut().wait_for(set).await;
                sender.send(waiter);
            }
            Self::IncrementDependency { id, sender } => {
                let dependency = app.components_mut().increment_dependency(id).await;
                sender.send(dependency);
            }
            Self::TryIncrementDependency {
                dependency,
                dependent,
                ty,
                sender,
            } => {
                let dependency = app
                    .components_mut()
                    .try_increment_dependency(dependency, dependent, ty)
                    .await;
                sender.send(dependency);
            }
            Self::IncrementLoadedDependency {
                dependency,
                dependent,
                ty,
                sender,
            } => {
                let dependency = app
                    .components_mut()
                    .increment_loaded_dependency(dependency, dependent, ty)
                    .await;
                sender.send(dependency);
            }
            Self::DecrementDependency { id } => app.components_mut().decrement_dependency(id).await,
            Self::OnAddFinished(component) => app.components_mut().on_add_finished(component).await,
            Self::OnLoadFinished(component) => {
                app.components_mut().on_load_finished(component).await;
            }
            Self::UnloadFinished(component) => {
                app.components_mut().unload_finished(component).await;
            }
        }
    }
}

/// Return type of [`AppFn::map()`], see [`AppMap`].
#[must_use]
pub struct AppFnMap;

/// Return type of [`App::map()`].
pub struct AppMap<'a> {
    app: &'a App,
}

impl AppFnMap {
    /// See [`AppMap::get`].
    pub const fn get(self) -> impl AppFnOnce<Output = Map> {
        AppFn::oneshot(
            (),
            |(), sender, _| super::Method::Map(Method::Get { sender }),
            |(), owned, _| owned.map.clone(),
        )
    }
}

impl<'a> AppMap<'a> {
    pub(super) const fn new(app: &'a App) -> Self {
        Self { app }
    }

    pub fn get(self) -> impl AppFuture<'a, Output = Map> {
        AppFn.map().get().call(self.app)
    }
}

impl AppFnComponentsMut {
    #[expect(clippy::unused_self)]
    pub(super) const fn increment_dependency(
        self,
        id: DependencyPart,
    ) -> impl AppMutFnOnce<Output = Dependency<()>> {
        AppFn::oneshot_mut(
            id,
            |id, sender, _| super::Method::Map(Method::IncrementDependency { id, sender }),
            |id, owned, handle| owned.increment_dependency(id, &handle.method_sender),
        )
    }

    #[expect(clippy::unused_self)]
    pub(super) fn try_increment_dependency(
        self,
        dependency: ComponentId,
        dependent: LoadedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFnOnce<Output = Result<Dependency<()>, Error>> {
        AppFn::oneshot_mut(
            (dependency, dependent, ty),
            |(dependency, dependent, ty), sender, _| {
                super::Method::Map(Method::TryIncrementDependency {
                    dependency,
                    dependent,
                    ty,
                    sender,
                })
            },
            |(dependency, dependent, ty), owned, handle| {
                let dependency = owned.map.validate_id(dependency).map_err(|dependency| {
                    Error::DependencyNotFound {
                        dependency,
                        dependent: dependent.component.clone_id(),
                    }
                })?;

                let dependency = owned.map.load_component(
                    dependency,
                    Channels {
                        event_sender: &owned.event_sender,
                        method_sender: &handle.method_sender,
                    },
                );

                let id = DependencyPart {
                    id: LoadedDependencyId::new(dependent, dependency),
                    ty,
                };
                Ok(owned.increment_dependency(id, &handle.method_sender))
            },
        )
    }

    pub const fn request(self, request: RequestSet) -> impl AppMutFnOnce<Output = Waiter> {
        AppFn::oneshot_mut(
            request,
            |request, sender, _| {
                super::Method::Map(Method::Request {
                    set: request,
                    sender,
                })
            },
            |request, owned, handle| {
                let (_, waiter) = owned.request(request, &handle.method_sender);
                waiter
            },
        )
    }

    pub const fn wait_for(self, request: RequestSet) -> impl AppMutFnOnce<Output = Waiter> {
        AppFn::oneshot_mut(
            request,
            |request, sender, _| {
                super::Method::Map(Method::WaitFor {
                    set: request,
                    sender,
                })
            },
            |request, owned, handle| {
                let (_, waiter) = owned.wait_for(request, &handle.method_sender);
                waiter
            },
        )
    }
}

impl<'a> AppComponentsMut<'a> {
    pub(super) fn try_increment_dependency(
        self,
        dependency: ComponentId,
        dependent: LoadedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFuture<'a, Output = Result<Dependency<()>, Error>> {
        AppFn
            .components_mut()
            .try_increment_dependency(dependency, dependent, ty)
            .call(self.app)
    }

    pub(crate) fn increment_loaded_dependency(
        self,
        dependency: AddedComponentId,
        dependent: LoadedComponentId,
        ty: DependencyType,
    ) -> impl AppMutFuture<'a, Output = Dependency<()>> {
        AppFn::oneshot_mut(
            (dependency, dependent, ty),
            |(dependency, dependent, ty), sender, _| {
                super::Method::Map(Method::IncrementLoadedDependency {
                    dependency,
                    dependent,
                    ty,
                    sender,
                })
            },
            |(dependency, dependent, ty), owned, handle| {
                let dependency = owned.map.load_component(
                    dependency,
                    Channels {
                        event_sender: &owned.event_sender,
                        method_sender: &handle.method_sender,
                    },
                );

                let id = DependencyPart {
                    id: LoadedDependencyId::new(dependent, dependency),
                    ty,
                };

                owned.increment_dependency(id, &handle.method_sender)
            },
        )
        .call(self.app)
    }

    pub(super) fn increment_dependency(
        self,
        id: DependencyPart,
    ) -> impl AppMutFuture<'a, Output = Dependency<()>> {
        AppFn
            .components_mut()
            .increment_dependency(id)
            .call(self.app)
    }

    fn decrement_dependency(self, id: DependencyPart) -> impl AppMutFuture<'a, Output = ()> {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::DecrementDependency { id }),
            |id, owned, handle| {
                owned.decrement_dependency(&id, &handle.method_sender);
            },
        )
        .call(self.app)
    }

    pub fn request(self, request: RequestSet) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.components_mut().request(request).call(self.app)
    }

    pub fn wait_for(self, request: RequestSet) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.components_mut().wait_for(request).call(self.app)
    }

    pub(super) async fn on_add_finished(self, id: AddedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::OnAddFinished(id)),
            |id, owned, handle| {
                owned.map.finish_on_add(
                    id,
                    Channels {
                        event_sender: &owned.event_sender,
                        method_sender: &handle.method_sender,
                    },
                );
            },
        )
        .call(self.app)
        .await;
    }

    pub(super) async fn on_load_finished(self, id: LoadedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::OnLoadFinished(id)),
            |id, owned, handle| {
                owned.map.finish_on_load(
                    id,
                    Channels {
                        event_sender: &owned.event_sender,
                        method_sender: &handle.method_sender,
                    },
                );
            },
        )
        .call(self.app)
        .await;
    }

    pub(super) async fn unload_finished(self, id: LoadedComponentId) {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::UnloadFinished(id)),
            |id, owned, handle| {
                owned.map.finish_unload(
                    id,
                    Channels {
                        event_sender: &owned.event_sender,
                        method_sender: &handle.method_sender,
                    },
                );
            },
        )
        .call(self.app)
        .await;
    }
}

pub(crate) fn request_channel() -> (Request, Waiter) {
    let (sender, receiver) = oneshot::channel();

    let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
    let releaser = Request { sender };
    let waiter = Waiter {
        receiver,
        request: releaser.clone(),
    };

    (releaser, waiter)
}

pub struct Waiter {
    receiver: oneshot::Receiver<()>,
    request: Request,
}

impl Waiter {
    pub async fn wait(self) {
        let _ = self.receiver.await;
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        !self.receiver.is_empty()
    }

    #[must_use]
    pub const fn request(&self) -> &Request {
        &self.request
    }
}

#[derive(Clone)]
pub struct Request {
    sender: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl Request {
    pub(crate) fn release(&self) {
        if let Ok(mut opt) = self.sender.try_lock()
            && let Some(sender) = opt.take()
        {
            let _ = sender.send(());
        }
    }
}

impl Hash for Request {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let pointer = Arc::as_ptr(&self.sender);
        pointer.hash(state);
    }
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}

impl Eq for Request {}

impl Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Request({:?})", Arc::as_ptr(&self.sender))
    }
}

#[cfg(test)]
mod test {
    use crate::{
        Error,
        app::{
            self, Channels, Owned,
            event::Event,
            map::{self, Dependency},
            registrar,
        },
        children::Children,
        component::IdentifiedComponentType,
    };

    struct Check {
        checker: fn(&app::Method) -> bool,
        code: &'static str,
    }

    macro_rules! check {
        (|$method:pat_param| $expr:expr) => {
            Check {
                checker: |$method| $expr,
                code: stringify!($expr),
            }
        };
    }

    fn run_methods(
        owned: &mut Owned,
        sender: &flume::Sender<app::Method>,
        method_receiver: &flume::Receiver<app::Method>,
        expected: impl IntoIterator<Item = Check>,
    ) {
        macro_rules! channels {
            ($method_sender:expr, $owned: expr) => {
                Channels {
                    event_sender: &$owned.event_sender,
                    method_sender: $method_sender,
                }
            };
        }

        let mut expected: Vec<_> = expected.into_iter().collect();

        while let Ok(method) = method_receiver.try_recv() {
            expected.retain(|check| !(check.checker)(&method));

            match method {
                app::Method::Registries(registrar::Method::SpawnOnAdd(comp)) => {
                    owned.map.finish_on_add(comp, channels!(sender, owned));
                }
                app::Method::Registries(registrar::Method::SpawnOnLoad(comp)) => {
                    owned.map.finish_on_load(comp, channels!(sender, owned));
                }
                app::Method::Registries(registrar::Method::SpawnUnload(comp)) => {
                    owned.map.finish_unload(comp, channels!(sender, owned));
                }
                app::Method::ReleaseEventListener => {
                    if let Some(dependency) = owned.event_sender_dependency.take() {
                        Dependency::drop_in_owned(dependency, owned, sender);
                    }
                }
                app::Method::Map(map::Method::DecrementDependency { id }) => {
                    owned
                        .map
                        .decrement_dependency(&id, channels!(sender, owned));
                }
                _ => (),
            }
        }

        if !expected.is_empty() {
            for check in expected {
                println!("[error] check failed: {}", check.code);
            }
            panic!("had errors - see stdout");
        }
    }

    #[test]
    fn initialize() -> Result<(), Error> {
        let mut owned = Owned::default();
        let (method_sender, method_receiver) = flume::unbounded();
        owned.initialize(Children::new(), &method_sender)?;

        run_methods(
            &mut owned,
            &method_sender,
            &method_receiver,
            [
                check!(|method| matches!(
                    method,
                    app::Method::Registries(registrar::Method::SpawnOnAdd(comp))
                    if **comp == Event::component_id()
                )),
                check!(|method| matches!(
                    method,
                    app::Method::Registries(registrar::Method::SpawnOnLoad(comp))
                    if *comp.component == Event::component_id()
                )),
                check!(|method| matches!(
                    method,
                    app::Method::Registries(registrar::Method::SpawnUnload(comp))
                    if *comp.component == Event::component_id()
                )),
                check!(|method| matches!(method, app::Method::Exit)),
            ],
        );

        Ok(())
    }
}
