use std::{
    collections::HashSet,
    fmt::Display,
    hash::Hash,
    ops::{AddAssign, Deref, DerefMut, SubAssign},
    sync::Arc,
};

use hashbrown::HashMap;
use tokio::sync::{broadcast, oneshot};

use crate::{
    app::{
        AppFn, AppFnOnce, AppFuture, AppMutFnOnce, AppMutFuture, HandleData, Owned,
        event::{DependencyLifecycle, Event},
        registry::{ErasedComponent, Registries},
    },
    children::Child,
    component::{ComponentIdPath, IdentifiedComponentType},
    input::CloneIn,
    prelude::*,
};

use super::event::ComponentLifecycle;

type RegistryId = &'static NamespacedIdRef;

#[derive(Default, Clone)]
pub struct Map {
    pub components: im::HashMap<ComponentId, MapComponent>,
    pub dependencies: im::HashMap<DependencyId, DependencyTypeCounts>,

    pub listeners: im::HashMap<ListenerId, Listener>,
}

#[derive(Clone)]
pub struct MapComponent {
    pub parent: Option<ComponentId>,
    pub load_index: LoadIndex,
}

impl Default for MapComponent {
    fn default() -> Self {
        Self {
            parent: None,
            load_index: LoadIndex(0),
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub enum ListenerId {
    Component {
        id: ComponentId,
        change: ComponentStateChange,
    },
    Dependency {
        id: DependencyId,
        change: DependencyStateChange,
    },
    Registry {
        id: RegistryId,
        change: RegistryStateChange,
    },
    Release(Releaser),
}

impl ListenerId {
    const fn deactivate_component(id: ComponentId, index: LoadIndex) -> Self {
        Self::Component {
            id,
            change: ComponentStateChange::Deactivate(index),
        }
    }

    const fn unload_component(id: ComponentId, index: LoadIndex) -> Self {
        Self::Component {
            id,
            change: ComponentStateChange::Unload(index),
        }
    }

    const fn remove_component(id: ComponentId) -> Self {
        Self::Component {
            id,
            change: ComponentStateChange::Remove,
        }
    }

    const fn remove_dependency(id: DependencyId) -> Self {
        Self::Dependency {
            id,
            change: DependencyStateChange::Remove,
        }
    }

    const fn remove_registry(id: RegistryId) -> Self {
        Self::Registry {
            id,
            change: RegistryStateChange::Remove,
        }
    }

    const fn prevent_removal(&self) -> bool {
        matches!(self, Self::Dependency { .. })
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum ComponentStateChange {
    Deactivate(LoadIndex),
    Unload(LoadIndex),
    Remove,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum DependencyStateChange {
    Remove,
}

impl DependencyStateChange {
    const fn lifecycle_on_add(self) -> DependencyLifecycle {
        match self {
            Self::Remove => DependencyLifecycle::Add,
        }
    }

    const fn lifecycle_on_remove(self) -> DependencyLifecycle {
        match self {
            Self::Remove => DependencyLifecycle::Remove,
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum RegistryStateChange {
    Remove,
}

#[derive(Clone, Default)]
pub struct Listener {
    pub blocking: im::HashSet<ListenerId>,
    pub blocked_on: im::HashSet<ListenerId>,

    prevent_removal: bool,
    pub requested: bool,
}

#[derive(Clone, Copy)]
struct Channels<'a> {
    event_sender: &'a broadcast::Sender<Event>,
    method_sender: &'a flume::Sender<super::Method>,
}

impl Map {
    /// Validates that the component with the id `id` is currently in the map.
    pub(super) fn validate_id(&self, id: ComponentId) -> Result<AddedComponentId, ComponentId> {
        if self.components.contains_key(&id) {
            Ok(AddedComponentId(id))
        } else {
            Err(id)
        }
    }

    pub(super) fn validate_registry_id(
        &self,
        id: &'static NamespacedIdRef,
    ) -> Result<AddedRegistryId, &'static NamespacedIdRef> {
        let component_id = ComponentId {
            registry: Registries::id(),
            component: ComponentIdPath::Static(id),
        };

        self.validate_id(component_id)
            .map(AddedRegistryId)
            .map_err(|_| id)
    }

    pub(super) fn add_component(
        &mut self,
        id: ComponentId,
        channels: Channels<'_>,
    ) -> Result<AddedComponentId, Error> {
        let entry = self.components.entry(id.clone());
        let im::hashmap::Entry::Vacant(entry) = entry else {
            return Err(Error::NameCollision { id });
        };

        entry.insert(MapComponent::default());

        let map_err = |_| Error::RegistryNotFound {
            component: id.clone(),
        };

        let remove_component_id = ListenerId::remove_component(id.clone());
        if id == Registries::component_id() {
            // HACK: load registry after adding only for the registries registry
            // (so that it can refer to itself)
            self.add_listener(&remove_component_id, channels);
            self.load_registry(id.registry, channels).map_err(map_err)?;
        } else {
            self.load_registry(id.registry, channels).map_err(map_err)?;
            self.add_listener(&remove_component_id, channels);
        }

        let remove_registry_id = ListenerId::remove_registry(id.registry);
        self.add_listener_block(remove_registry_id, remove_component_id, channels);

        Ok(AddedComponentId(id))
    }

    pub(super) fn adopt(
        &mut self,
        child: AddedComponentId,
        parent: AddedComponentId,
        channels: Channels<'_>,
    ) {
        let component_info = &mut self.components[&child.0];
        let old_parent = component_info.parent.replace(parent.0.clone());

        self.add_listener_block(
            ListenerId::remove_component(parent.0),
            ListenerId::remove_component(child.0.clone()),
            channels,
        );

        if let Some(old_parent) = old_parent {
            self.remove_listener_block(
                &ListenerId::remove_component(old_parent),
                &ListenerId::remove_component(child.0),
                channels,
            );
        }
    }

    fn increment_dependency(
        &mut self,
        part: DependencyPart,
        channels: Channels<'_>,
    ) -> Dependency<()> {
        let id = DependencyId {
            dependent: part.dependent.0.clone(),
            dependency: part.dependency.0.clone(),
        };
        let dependency_listener = ListenerId::remove_dependency(id.clone());

        let dependency_info = &mut self.dependencies[&id];
        let prev_info = *dependency_info;
        *dependency_info += part.ty;

        let new_dependency = prev_info.is_zero();
        let newly_non_weak = !dependency_info.effective_type().is_weak()
            && (new_dependency || prev_info.effective_type().is_weak());

        if new_dependency {
            let added = self.add_listener(&dependency_listener, channels);
            assert!(added);

            let load_index = self.load_component(&part.dependency, channels);
            self.add_listener_block(
                ListenerId::unload_component(id.dependency.clone(), load_index),
                dependency_listener.clone(),
                channels,
            );

            let load_index = self.load_component(&part.dependent, channels);
            self.add_listener_block(
                ListenerId::unload_component(id.dependent.clone(), load_index),
                dependency_listener.clone(),
                channels,
            );
        }

        if newly_non_weak {
            // load_component ensures that this load index is already activated or will eventually
            // activate (since it uses the current load index), so we can be sure this is okay
            let load_index = self.load_component(&part.dependent, channels);
            self.add_listener_block(
                ListenerId::deactivate_component(id.dependency.clone(), load_index),
                dependency_listener,
                channels,
            );
        }

        Dependency {
            value: (),
            handle: DependencyHandle {
                id: part,
                sender: Some(channels.method_sender.clone()),
            },
        }
    }

    fn decrement_dependency(&mut self, part: &DependencyPart, channels: Channels<'_>) {
        let id = DependencyId {
            dependent: part.dependent.0.clone(),
            dependency: part.dependency.0.clone(),
        };

        let dependency_info = &mut self.dependencies[&id];
        *dependency_info -= part.ty;
        let removed = dependency_info.is_zero();

        if !removed {
            return;
        }

        let listener_id = ListenerId::remove_dependency(id);
        self.release_prevented_removal(&listener_id, channels);
    }

    fn load_component(&mut self, id: &AddedComponentId, channels: Channels<'_>) -> LoadIndex {
        let load_index = self.components[&id.0].load_index;

        let unload_id = ListenerId::unload_component(id.0.clone(), load_index);
        let added = self.add_listener(&unload_id, channels);
        if !added {
            // already loaded
            return load_index;
        }

        let deactivate_id = ListenerId::deactivate_component(id.0.clone(), load_index);

        let added_deactivate = self.add_listener(&deactivate_id, channels);
        assert!(
            added_deactivate,
            "component shouldn't be activated without also being loaded"
        );

        let remove_id = ListenerId::remove_component(id.0.clone());
        self.add_listener_block(unload_id.clone(), deactivate_id, channels);
        self.add_listener_block(remove_id, unload_id, channels);

        load_index
    }

    fn load_registry(&mut self, id: RegistryId, channels: Channels<'_>) -> Result<(), RegistryId> {
        let remove_id = ListenerId::remove_registry(id);
        if self.listeners.contains_key(&remove_id) {
            return Ok(());
        }

        let component_id = self.validate_registry_id(id)?;

        let added = self.add_listener(&remove_id, channels);
        assert!(added);

        let index = self.load_component(&component_id, channels);
        let component_deactivate_id = ListenerId::deactivate_component(component_id.clone(), index);
        self.add_listener_block(component_deactivate_id, remove_id, channels);

        Ok(())
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
        )
    }

    fn on_add(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        match listener {
            ListenerId::Component { id, change } => {
                let _ = channels.event_sender.send(Event::Component {
                    id: id.clone(),
                    lifecycle: change.lifecycle_on_add(),
                });

                if matches!(change, ComponentStateChange::Deactivate(_)) {
                    // Keep the component active until we finish loading it.
                    let part = DependencyPart {
                        dependent: AddedComponentId::app(),
                        dependency: AddedComponentId(id.clone()),
                        ty: DependencyType::Transient,
                    };
                    let dependency = self.increment_dependency(part, channels);

                    // Actually initiate the load.
                    let _ = channels.method_sender.send(super::Method::Registries(
                        super::registry::Method::SpawnLoad { dependency },
                    ));
                }
            }
            ListenerId::Dependency { id, change } => {
                let _ = channels.event_sender.send(Event::Dependency {
                    id: id.clone(),
                    lifecycle: change.lifecycle_on_add(),
                });
            }
            ListenerId::Registry { .. } => {
                // For now, registries are an implementation detail of the map.
                // They're shown in it, but events aren't given.
            }
            ListenerId::Release(_) => (),
        }
    }

    fn on_remove(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        match listener {
            ListenerId::Component { id, change } => {
                let _ = channels.event_sender.send(Event::Component {
                    id: id.clone(),
                    lifecycle: change.lifecycle_on_remove(),
                });

                if matches!(change, ComponentStateChange::Deactivate(_)) {
                    // Keep the component loaded until we finish unloading it.
                    let part = DependencyPart {
                        dependent: AddedComponentId::app(),
                        dependency: AddedComponentId(id.clone()),
                        ty: DependencyType::Weak,
                    };
                    let dependency = self.increment_dependency(part, channels);

                    // Actually initiate the unload.
                    let _ = channels.method_sender.send(super::Method::Registries(
                        super::registry::Method::SpawnUnload { dependency },
                    ));

                    // Increment the load index, since we've started unloading this one.
                    // WARN: it's important that this index increment happens after the dependency
                    // increment, since the dependency increment depends on the current load index
                    let load_index = &mut self.components[id].load_index;
                    *load_index = LoadIndex(load_index.0 + 1);
                } else if matches!(change, ComponentStateChange::Remove) {
                    self.components.remove(id);

                    // Check if we're removing the app, and if so, exit.
                    if *id == App::component_id() {
                        Self::on_app_removal(channels);
                    }
                }
            }
            ListenerId::Dependency { id, change } => {
                let _ = channels.event_sender.send(Event::Dependency {
                    id: id.clone(),
                    lifecycle: change.lifecycle_on_remove(),
                });

                if matches!(change, DependencyStateChange::Remove) {
                    self.dependencies.remove(id);
                }
            }
            ListenerId::Registry { .. } => {
                // For now, registries are an implementation detail of the map.
                // They're shown in it, but events aren't given.
            }
            ListenerId::Release(releaser) => releaser.release(),
        }
    }

    /// Returns `true` iff the request should propagate to the listener's blockers.
    fn on_request(&mut self, listener: &ListenerId, channels: Channels<'_>) -> bool {
        match listener {
            // remove propagates and also allows it to be removed
            ListenerId::Component {
                change: ComponentStateChange::Remove,
                ..
            } => {
                self.release_prevented_removal(listener, channels);
                true
            }
            // unload doesn't propagate to its blockers, only its deactivation
            ListenerId::Component {
                change: ComponentStateChange::Unload(index),
                id,
            } => {
                self.request(
                    &ListenerId::deactivate_component(id.clone(), *index),
                    channels,
                );
                false
            }
            // other changes for components propagate to their dependencies
            // registry propagates to all of its "children"
            ListenerId::Component { .. } | ListenerId::Registry { .. } => true,
            // dependencies only propagate if they're unremovable
            ListenerId::Dependency {
                id,
                change: DependencyStateChange::Remove,
            } => {
                let dependency = &self.dependencies[id];
                match dependency.effective_type() {
                    DependencyType::Unremovable => true,
                    // if they're removable, we send the event
                    DependencyType::Removable => {
                        let _ = channels
                            .event_sender
                            .send(Event::RequestDependencyRemoval { id: id.clone() });

                        false
                    }
                    DependencyType::Transient | DependencyType::Weak => false,
                }
            }
            // release doesn't propagate because it shouldn't have any blockers
            ListenerId::Release(_) => false,
        }
    }

    fn add_listener(&mut self, listener: &ListenerId, channels: Channels<'_>) -> bool {
        let im::hashmap::Entry::Vacant(entry) = self.listeners.entry(listener.clone()) else {
            return false;
        };

        let prevent_removal = Self::prevent_removal(entry.key());
        entry.insert(Listener {
            prevent_removal,
            ..Default::default()
        });

        self.on_add(listener, channels);
        true
    }

    fn release_prevented_removal(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        let was_prevented = std::mem::replace(&mut self.listeners[listener].prevent_removal, false);
        if was_prevented {
            self.check_removal(listener, channels);
        }
    }

    fn check_removal(&mut self, listener: &ListenerId, channels: Channels<'_>) {
        let im::hashmap::Entry::Occupied(entry) = self.listeners.entry(listener.clone()) else {
            return;
        };

        if entry.get().prevent_removal || !entry.get().blocked_on.is_empty() {
            return;
        }

        let (blocked_on, info) = entry.remove_entry();
        self.on_remove(&blocked_on, channels);
        for listener in info.blocking {
            self.remove_listener_block(&listener, &blocked_on, channels);
        }
    }

    fn request(&mut self, id: &ListenerId, channels: Channels<'_>) {
        let listener = self.listeners.entry(id.clone()).or_default();
        let already_requested = std::mem::replace(&mut listener.requested, true);
        if already_requested {
            return;
        }

        // get this here to release our mutable reference
        let blocked_on = listener.blocked_on.clone();

        let propagate = self.on_request(id, channels);
        if !propagate {
            return;
        }

        for listener in blocked_on {
            self.request(&listener, channels);
        }
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
        let requested = blocked_info.requested;

        let blocked_on_info = &mut self.listeners[&blocked_on];
        blocked_on_info.blocking.insert(blocked);

        if requested {
            self.request(&blocked_on, channels);
        }
    }

    fn remove_listener_block(
        &mut self,
        blocked: &ListenerId,
        blocked_on: &ListenerId,
        channels: Channels<'_>,
    ) {
        let blocked_info = &mut self.listeners[blocked];
        blocked_info.blocked_on.remove(blocked_on);

        if let Some(blocked_on_info) = self.listeners.get_mut(blocked_on) {
            blocked_on_info.blocking.remove(blocked);
        }

        self.check_removal(blocked, channels);
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Request {
    RemoveComponent(ComponentId),
    UnloadComponent(ComponentId),
    RemoveDependency(DependencyId),
}

impl Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemoveComponent(id) => write!(f, "remove component '{id}'"),
            Self::UnloadComponent(id) => write!(f, "unload component '{id}'"),
            Self::RemoveDependency(id) => write!(f, "remove dependency '{id}'"),
        }
    }
}

#[derive(Clone, Default)]
pub struct RequestSet {
    pub set: hashbrown::HashSet<Request>,
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
        self.set.insert(Request::RemoveComponent(component));
        self
    }

    #[must_use]
    pub fn unload(component: ComponentId) -> Self {
        Self::new().and_unload(component)
    }

    #[must_use]
    pub fn and_unload(mut self, component: ComponentId) -> Self {
        self.set.insert(Request::UnloadComponent(component));
        self
    }

    #[must_use]
    pub fn remove_dependency(dependency: DependencyId) -> Self {
        Self::new().and_remove_dependency(dependency)
    }

    #[must_use]
    pub fn and_remove_dependency(mut self, dependency: DependencyId) -> Self {
        self.set.insert(Request::RemoveDependency(dependency));
        self
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
            self.map.adopt(id.clone_inner(), parent, channels);
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

    fn decrement_dependency(&mut self, part: &DependencyPart, handle: &HandleData) {
        self.map.decrement_dependency(
            part,
            Channels {
                event_sender: &self.event_sender,
                method_sender: &handle.method_sender,
            },
        );
    }

    pub(super) fn wait_for(
        &mut self,
        request: RequestSet,
        sender: &flume::Sender<super::Method>,
    ) -> (ListenerId, Waiter) {
        let (releaser, waiter) = waiter_channel();
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
            let child_id = match &request {
                Request::RemoveComponent(id) => ListenerId::remove_component(id.clone()),
                Request::UnloadComponent(id) => {
                    let Some(component) = self.map.components.get(id) else {
                        self.add_error(Error::InvalidRequest { request });
                        continue;
                    };

                    ListenerId::unload_component(id.clone(), component.load_index)
                }
                Request::RemoveDependency(id) => ListenerId::remove_dependency(id.clone()),
            };

            if !self.map.listeners.contains_key(&child_id) {
                self.add_error(Error::InvalidRequest { request });
                continue;
            }

            self.map.add_listener_block(
                release_id.clone(),
                child_id,
                Channels {
                    event_sender: &self.event_sender,
                    method_sender: sender,
                },
            );
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
        self.map.request(
            &release_id,
            Channels {
                event_sender: &self.event_sender,
                method_sender: sender,
            },
        );
        (release_id, waiter)
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

#[derive(Debug)]
pub struct DependencyPart {
    pub(super) dependent: AddedComponentId,
    pub(super) dependency: AddedComponentId,
    pub(super) ty: DependencyType,
}

impl DependencyPart {
    pub(super) fn clone_inner(&self) -> Self {
        Self {
            dependency: self.dependency.clone_inner(),
            dependent: self.dependent.clone_inner(),
            ty: self.ty,
        }
    }
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

impl<T: Send + Sync> Dependency<T> {
    pub async fn clone_with<U: Send>(
        this: &Self,
        app: &mut App,
        func: impl AsyncFnOnce(&T, &mut App) -> U + Send,
    ) -> Dependency<U> {
        let dependency = app
            .map_mut()
            .increment_dependency(this.handle.id.clone_inner())
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
        let dependency = owned.increment_dependency(this.handle.id.clone_inner(), sender);
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

    /// Drops this dependency using a specific app handle.
    ///
    /// This can be more efficient than the default drop implementation, but only if the app handle
    /// is the owned app handle. Otherwise, it will do the same exact thing as the automatic drop.
    ///
    /// # Panics
    ///
    /// - If `app` is not the app that this dependency was obtained in.
    fn drop_in(this: Self, app: &mut App) -> impl AppMutFuture<'_, Output = ()> {
        drop(this.value);
        this.handle.drop_in(app)
    }

    /// Drops this dependency in an owned app.
    ///
    /// # Panics
    ///
    /// - Potential panic if `owned` is not the app that this dependency was obtained in.
    pub(super) fn drop_in_owned(this: Self, owned: &mut Owned, handle: &HandleData) {
        drop(this.value);
        this.handle.drop_in_owned(owned, handle);
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

        app.map_mut().decrement_dependency(self.id.clone_inner())
    }

    /// Drops this dependency in an owned app.
    ///
    /// # Panics
    ///
    /// - Potential panic if `owned` is not the app that this dependency was obtained in.
    fn drop_in_owned(mut self, owned: &mut Owned, handle: &HandleData) {
        self.sender
            .take()
            .expect("drop_in takes ownership of self, so the sender should only be taken once");

        owned.decrement_dependency(&self.id, handle);
    }
}

impl Drop for DependencyHandle {
    fn drop(&mut self) {
        // If drop_in is called, the sender will be None, so we should ignore the drop.
        let Some(sender) = self.sender.take() else {
            return;
        };

        let event = super::Method::Map(Method::DecrementDependency {
            id: self.id.clone_inner(),
        });

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(sender.into_send_async(event));
        } else {
            let _ = sender.send(event);
        }
    }
}

pub(super) enum Method {
    Get {
        sender: oneshot::Sender<Map>,
    },
    Request {
        set: RequestSet,
        sender: oneshot::Sender<Waiter>,
    },
    WaitFor {
        set: RequestSet,
        sender: oneshot::Sender<Waiter>,
    },
    // Dependency
    TryIncrementDependency {
        dependency: ComponentId,
        dependent: ComponentId,
        ty: DependencyType,
        sender: oneshot::Sender<Result<Dependency<()>, Error>>,
    },
    IncrementDependency {
        id: DependencyPart,
        sender: oneshot::Sender<Dependency<()>>,
    },
    DecrementDependency {
        id: DependencyPart,
    },
}

impl Method {
    pub(super) async fn run(self, app: &mut App) {
        match self {
            Self::Get { sender } => {
                let map = app.map().get().await;
                let _ = sender.send(map);
            }
            Self::Request { set, sender } => {
                let waiter = app.map_mut().request(set).await;
                let _ = sender.send(waiter);
            }
            Self::WaitFor { set, sender } => {
                let waiter = app.map_mut().wait_for(set).await;
                let _ = sender.send(waiter);
            }
            Self::IncrementDependency { id, sender } => {
                let dependency = app.map_mut().increment_dependency(id).await;
                let _ = sender.send(dependency);
            }
            Self::TryIncrementDependency {
                dependency,
                dependent,
                ty,
                sender,
            } => {
                let dependency = app
                    .map_mut()
                    .try_increment_dependency(dependency, dependent, ty)
                    .await;
                let _ = sender.send(dependency);
            }
            Self::DecrementDependency { id } => app.map_mut().decrement_dependency(id).await,
        }
    }
}

/// Return type of [`AppFn::map()`], see [`AppMap`].
#[must_use]
pub struct AppFnMap;
impl AppFnMap {
    /// See [`AppMap::get`].
    pub const fn get(self) -> impl AppFnOnce<Output = Map> {
        AppFn::oneshot(
            (),
            |(), sender, _| super::Method::Map(Method::Get { sender }),
            async |(), owned, _| owned.map.clone(),
        )
    }
}

#[must_use]
pub struct AppFnMapMut;
impl AppFnMapMut {
    #[expect(clippy::unused_self)]
    pub(super) const fn increment_dependency(
        self,
        id: DependencyPart,
    ) -> impl AppMutFnOnce<Output = Dependency<()>> {
        AppFn::oneshot_mut(
            id,
            |id, sender, _| super::Method::Map(Method::IncrementDependency { id, sender }),
            async |id, owned, handle| owned.increment_dependency(id, &handle.method_sender),
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
            async |request, owned, handle| {
                let (_, waiter) = owned.request(request, &handle.method_sender);
                waiter
            },
        )
    }

    pub const fn wait_for(self, request: RequestSet) -> impl AppMutFnOnce<Output = Waiter> {
        AppFn::oneshot_mut(
            request,
            |request, sender, _| {
                super::Method::Map(Method::Request {
                    set: request,
                    sender,
                })
            },
            async |request, owned, handle| {
                let (_, waiter) = owned.wait_for(request, &handle.method_sender);
                waiter
            },
        )
    }
}

/// Return type of [`App::map()`].
pub struct AppMap<'a> {
    app: &'a App,
}

impl<'a> AppMap<'a> {
    pub(super) const fn new(app: &'a App) -> Self {
        Self { app }
    }

    pub fn get(self) -> impl AppFuture<'a, Output = Map> {
        AppFn.map().get().call(self.app)
    }
}

/// Return type of [`App::map_mut()`].
pub(super) struct AppMapMut<'a> {
    app: &'a mut App,
}

impl<'a> AppMapMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    pub(super) fn try_increment_dependency(
        self,
        dependency: ComponentId,
        dependent: ComponentId,
        ty: DependencyType,
    ) -> impl AppMutFuture<'a, Output = Result<Dependency<()>, Error>> {
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
            async |(dependency, dependent, ty), owned, handle| {
                let dependency = owned.map.validate_id(dependency).map_err(|dependency| {
                    Error::DependencyNotFound {
                        dependency,
                        dependent: dependent.clone(),
                    }
                })?;

                let dependent = owned.map.validate_id(dependent).map_err(|dependent| {
                    Error::DependentNotFound {
                        dependency: dependency.clone(),
                        dependent,
                    }
                })?;

                let id = DependencyPart {
                    dependent,
                    dependency,
                    ty,
                };
                Ok(owned.increment_dependency(id, &handle.method_sender))
            },
        )
        .call(self.app)
    }

    pub(super) fn increment_dependency(
        self,
        id: DependencyPart,
    ) -> impl AppMutFuture<'a, Output = Dependency<()>> {
        AppFn.map_mut().increment_dependency(id).call(self.app)
    }

    fn decrement_dependency(self, id: DependencyPart) -> impl AppMutFuture<'a, Output = ()> {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::DecrementDependency { id }),
            async |id, owned, handle| {
                owned.decrement_dependency(&id, handle);
            },
        )
        .call(self.app)
    }

    pub fn request(self, request: RequestSet) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.map_mut().request(request).call(self.app)
    }

    pub fn wait_for(self, request: RequestSet) -> impl AppMutFuture<'a, Output = Waiter> {
        AppFn.map_mut().wait_for(request).call(self.app)
    }
}

/// A component id that is (loosely) guaranteed to be currently alive in the map.
// NOTE: don't implement Clone for this type - it'd make it harder to prove
#[derive(Debug)]
pub(super) struct AddedComponentId(ComponentId);

impl AddedComponentId {
    fn into_inner(self) -> ComponentId {
        self.0
    }

    pub fn clone_inner(&self) -> Self {
        Self(self.deref().clone())
    }

    pub fn app() -> Self {
        Self(ComponentId::within::<App, App>())
    }
}

impl Deref for AddedComponentId {
    type Target = ComponentId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(super) struct AddedRegistryId(AddedComponentId);

impl Deref for AddedRegistryId {
    type Target = AddedComponentId;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

fn waiter_channel() -> (Releaser, Waiter) {
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
    (Releaser { sender }, Waiter { receiver })
}

pub struct Waiter {
    receiver: oneshot::Receiver<()>,
}

impl Waiter {
    pub async fn wait(self) {
        let _ = self.receiver.await;
    }
}

#[derive(Clone)]
pub struct Releaser {
    sender: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl Releaser {
    pub(crate) fn release(&self) {
        if let Ok(mut opt) = self.sender.try_lock()
            && let Some(sender) = opt.take()
        {
            let _ = sender.send(());
        }
    }
}

impl Hash for Releaser {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let pointer = Arc::as_ptr(&self.sender);
        pointer.hash(state);
    }
}

impl PartialEq for Releaser {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}

impl Eq for Releaser {}
