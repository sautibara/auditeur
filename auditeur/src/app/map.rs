use std::{
    collections::HashSet,
    ops::{AddAssign, Deref, DerefMut, SubAssign},
};

use hashbrown::HashMap;
use tokio::sync::oneshot;

use crate::{
    app::{
        AppFn, AppFnOnce, AppFuture, AppMutFnOnce, AppMutFuture, HandleData, Owned,
        event::{DependencyLifecycle, Event},
        registry::{ErasedComponent, Registries, Registry},
    },
    children::Child,
    component::ComponentIdPath,
    input::CloneIn,
    prelude::*,
};

use super::event::ComponentLifecycle;

#[derive(Default, Clone)]
pub struct Map {
    pub components: im::HashMap<ComponentId, MapComponent>,
    pub dependencies: im::HashMap<DependencyId, DependencyTypeCounts>,
}

#[derive(Default, Clone)]
pub struct MapComponent {
    pub parent: Option<ComponentId>,
    pub marked_for_removal: bool,

    /// If this component is currently loaded in its registry.
    active: bool,
    /// The number of dependencies keeping this component active.
    ///
    /// If this goes to zero, the registry will unload the component, making it loaded but not
    /// active. After, the component is expected to release all of its dependencies, making it
    /// unloaded.
    ///
    /// This is a cached value equal to the number of non-weak dependents.
    strong_count: usize,
    /// The number of dependencies keeping this component loaded.
    ///
    /// Once this and the strong count go to zero, the component will be considered unloaded
    /// in the map.
    ///
    /// This is a cached value equal to the number of weak dependents plus the number of
    /// dependencies (of any type).
    weak_count: usize,
    /// The number of children of this component.
    ///
    /// Once this goes to zero, the component is unloaded in the map, and the component is marked
    /// for removal, the component will be removed.
    ///
    /// This is a cached value.
    child_count: usize,
}

impl MapComponent {
    pub(super) const fn should_unload(&self) -> bool {
        self.active && self.strong_count == 0
    }

    const fn is_unloaded(&self) -> bool {
        !self.active && self.strong_count == 0 && self.weak_count == 0
    }

    const fn should_remove(&self) -> bool {
        self.marked_for_removal && self.child_count == 0 && self.is_unloaded()
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

    fn index(&self, id: &AddedComponentId) -> &MapComponent {
        // This is okay (shouldn't ever panic) because `id` is an AddedComponentId
        &self.components[id]
    }

    fn index_mut(&mut self, id: &AddedComponentId) -> &mut MapComponent {
        // This is okay (shouldn't ever panic) because `id` is an AddedComponentId
        &mut self.components[id]
    }

    pub(super) fn add(&mut self, child: ComponentId) -> Result<AddedComponentId, Error> {
        if self.components.contains_key(&child) {
            return Err(Error::NameCollision { id: child });
        }

        self.components.entry(child.clone()).or_default();

        Ok(AddedComponentId(child))
    }

    pub(super) fn adopt(&mut self, child: AddedComponentId, parent: AddedComponentId) {
        // Increment the new parent's child count.
        let parent_info = self.index_mut(&parent);
        parent_info.child_count += 1;
        let marked_for_removal = parent_info.marked_for_removal;

        // Update the component's info.
        let component = self.components.entry(child.into_inner()).or_default();
        component.marked_for_removal |= marked_for_removal;
        let old_parent = component.parent.replace(parent.into_inner());

        // Decrement the old parent's child count.
        if let Some(old_parent) = old_parent {
            let old_parent = AddedComponentId(old_parent);
            let old_parent_info = self.index_mut(&old_parent);
            old_parent_info.child_count -= 1;
        }
    }

    fn increment_dependency(
        &mut self,
        part: DependencyPart,
        sender: &flume::Sender<super::Method>,
    ) -> DependencyInfo {
        // The dependent always gets a weak reference.
        self.index_mut(&part.dependent).weak_count += 1;

        // The dependency may get a strong or weak reference.
        // We also check if the component is getting loaded here before updating the value.
        let dependency = self.index_mut(&part.dependency);
        let new_loaded = dependency.strong_count == 0 && dependency.weak_count == 0;
        if part.ty.is_weak() {
            dependency.weak_count += 1;
        } else {
            dependency.strong_count += 1;
        }

        // Update the dependency count and check if it's a new one.
        let dependency_count = (self.dependencies)
            .entry(DependencyId {
                dependent: part.dependent.deref().clone(),
                dependency: part.dependency.deref().clone(),
            })
            .or_default();
        let new_dependency = dependency_count.is_zero();
        *dependency_count += part.ty;

        let dependency = Dependency {
            value: (),
            handle: DependencyHandle {
                id: part,
                sender: Some(sender.clone()),
            },
        };

        DependencyInfo {
            dependency,
            new_dependency,
            new_loaded,
        }
    }

    fn decrement_dependency(&mut self, part: &DependencyPart) -> DecrementInfo {
        // Remove the dependency first.
        let mut dependency_removed = false;
        if let im::hashmap::Entry::Occupied(mut entry) = self.dependencies.entry(DependencyId {
            dependent: part.dependent.deref().clone(),
            dependency: part.dependency.deref().clone(),
        }) {
            *entry.get_mut() -= part.ty;

            // So that we could check if it was removed.
            dependency_removed = entry.get().is_zero();
            if dependency_removed {
                entry.remove();
            }
        }

        // Then update the dependency.
        let dependency = self.index_mut(&part.dependency);
        if part.ty.is_weak() {
            dependency.weak_count = dependency.weak_count.saturating_sub(1);
        } else {
            dependency.strong_count = dependency.strong_count.saturating_sub(1);
        }

        // And finally the dependent.
        let dependent = self.index_mut(&part.dependent);
        dependent.weak_count = dependent.weak_count.saturating_sub(1);

        DecrementInfo { dependency_removed }
    }
}

struct DependencyInfo {
    dependency: Dependency<()>,
    new_dependency: bool,
    new_loaded: bool,
}

struct DecrementInfo {
    dependency_removed: bool,
}

impl Owned {
    /// Adds a registry to the registrar, returning its id.
    ///
    /// # Errors
    ///
    /// - [`Error::NameCollision`] if that registry has already been added.
    ///
    /// # Panics
    ///
    /// - If the [`Registries`] registry has not already been added to the app.
    pub(super) fn add_registry<R: Registry>(
        &mut self,
        parent: Option<AddedComponentId>,
        handle: &HandleData,
    ) -> Result<AddedRegistryId, Error> {
        let (component, _) = ErasedComponent::new::<Registries, R>(());
        let registry_id = ComponentId::within::<Registries, Registries>();
        // WARN: we're assuming that the registries registry is already added here
        let registry_id = AddedRegistryId(AddedComponentId(registry_id));
        let id =
            AddChild::begin(self, component)?.finish_validated(self, parent, registry_id, handle);
        Ok(AddedRegistryId(id))
    }

    pub(super) fn add_children(&mut self, children: Children, handle: &HandleData) {
        struct AddChildren<'a> {
            owned: &'a mut Owned,
            handle: &'a HandleData,
            children: HashMap<ComponentId, Child>,
            errored: HashSet<ComponentId>,
        }

        impl<'a> AddChildren<'a> {
            fn new(owned: &'a mut Owned, children: Children, handle: &'a HandleData) -> Self {
                Self {
                    owned,
                    handle,
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
                let res = AddChild::begin(self.owned, child.component)
                    .and_then(|add| add.finish(self.owned, child.parent, self.handle));
                let res = self.owned.drain_result(res);
                if res.is_none() {
                    self.errored.insert(id);
                }
                res.is_some()
            }
        }

        let mut adder = AddChildren::new(self, children, handle);
        #[expect(clippy::needless_collect)] // needed b/c we're mutating adder
        for id in adder.children.keys().cloned().collect::<Vec<_>>() {
            adder.add(id);
        }
    }

    pub(super) fn map_activate(&mut self, id: &ComponentId, active: bool, handle: &HandleData) {
        if let Some(component) = self.map.components.get_mut(id) {
            let was_active = std::mem::replace(&mut component.active, active);

            let lifecycle_change = match (was_active, active) {
                (false, true) => Some(ComponentLifecycle::Active),
                (true, false) => Some(ComponentLifecycle::Unloading),
                _ => None,
            };

            if let Some(lifecycle) = lifecycle_change {
                let _ = self.event_sender.send(Event::Component {
                    id: id.clone(),
                    lifecycle,
                });
            } else {
                // We don't need to compute unloads b/c no change was made.
                return;
            }
        } else {
            // Component doesn't exist, so we shouldn't compute unloads.
            return;
        }

        // If this was just loaded from a weak dependency, then it has to be immediately unloaded.
        // If this was just unloaded, then it may have been fully unloaded.
        // As such, we always have to check unloads if something changes.
        let id = AddedComponentId(id.clone());
        self.compute_unloads(&id, handle);
    }

    /// `active`: If this dependency is being incremented with a corresponding registry load.
    pub(super) fn increment_dependency_inner(
        &mut self,
        part: DependencyPart,
        active: bool,
        sender: &flume::Sender<super::Method>,
    ) -> Dependency<()> {
        let DependencyInfo {
            dependency,
            new_dependency,
            new_loaded,
        } = self.map.increment_dependency(part.clone_inner(), sender);

        // Send any events that we have to.

        if new_loaded {
            let _ = self.event_sender.send(Event::Component {
                id: part.dependency.deref().clone(),
                lifecycle: ComponentLifecycle::Load,
            });

            match (active, part.ty.is_weak()) {
                // If this is a newly loaded component but this dependency increment won't make it
                // active, then we want to manually activate it in the future.
                (false, false) => {
                    // We want to duplicate the dependency to keep it alive until we load it.
                    let dependency = DependencyPart {
                        // We want the dependency to be strong, so that it doesn't unload before it
                        // gets loaded the first time, but also transient, since it'll be dropped
                        // right after it's created.
                        ty: DependencyType::Transient,
                        ..part.clone_inner()
                    };

                    // We can call the map method because we know the dependency already exists.
                    let DependencyInfo {
                        dependency,
                        new_dependency,
                        new_loaded,
                    } = self.map.increment_dependency(dependency, sender);
                    assert!(
                        !new_dependency && !new_loaded,
                        "this dependency already exists, so nothing should be new"
                    );

                    // Then we queue a load for the future.
                    let _ =
                        sender.send(super::Method::Map(Method::LoadRawDependency { dependency }));
                }
                // If this dependency increment will activate the component, but the dependency is
                // weak and so it shouldn't be active, we want to deactivate (unload) the component.
                (true, true) => {
                    let _ = sender.send(super::Method::Registries(
                        super::registry::Method::SpawnUnload {
                            id: part.dependency.deref().clone(),
                        },
                    ));
                }
                _ => (),
            }
        }

        if new_dependency {
            let _ = self.event_sender.send(Event::Dependency {
                id: DependencyId {
                    dependent: part.dependent.into_inner(),
                    dependency: part.dependency.into_inner(),
                },
                lifecycle: DependencyLifecycle::Add,
            });
        }

        // Give a dependency handle, since we just updated the count.
        // This ensures that all dependencies eventually get removed in the end.
        dependency
    }

    pub(super) fn increment_dependency(
        &mut self,
        part: DependencyPart,
        sender: &flume::Sender<super::Method>,
    ) -> Dependency<()> {
        self.increment_dependency_inner(part, false, sender)
    }

    pub(super) fn increment_dependency_for_get(
        &mut self,
        part: DependencyPart,
        sender: &flume::Sender<super::Method>,
    ) -> Dependency<()> {
        self.increment_dependency_inner(part, true, sender)
    }

    fn decrement_dependency(&mut self, part: &DependencyPart, handle: &HandleData) {
        let DecrementInfo { dependency_removed } = self.map.decrement_dependency(part);

        if dependency_removed {
            let _ = self.event_sender.send(Event::Dependency {
                id: DependencyId {
                    dependent: part.dependent.deref().clone(),
                    dependency: part.dependency.deref().clone(),
                },
                lifecycle: DependencyLifecycle::Remove,
            });
        }

        self.compute_unloads(&part.dependency, handle);
        self.compute_unloads(&part.dependent, handle);

        todo!()
    }

    fn compute_unloads(&mut self, id: &AddedComponentId, handle: &HandleData) {
        let component_info = self.map.index(id);
        if component_info.should_unload() {
            let _ = handle.method_sender.send(super::Method::Registries(
                super::registry::Method::SpawnUnload {
                    id: id.deref().clone(),
                },
            ));
        }

        if component_info.is_unloaded() {
            let _ = self.event_sender.send(Event::Component {
                id: id.deref().clone(),
                lifecycle: ComponentLifecycle::Unload,
            });

            self.compute_removes(id, handle);
        }
    }

    fn compute_removes(&mut self, id: &AddedComponentId, handle: &HandleData) {
        let component_info = self.map.index(id);
        if component_info.should_remove() {
            self.remove(id, handle);
        }
    }

    fn remove(&mut self, id: &AddedComponentId, handle: &HandleData) {
        let method = super::Method::Map(Method::FinishRemoval {
            id: id.clone_inner(),
        });

        let sender = handle.method_sender.clone();
        self.registrar.remove_unchecked(id, move || {
            let _ = sender.send(method);
        });
    }

    fn finish_removal(&mut self, id: &AddedComponentId, handle: &HandleData) {
        // Now that we removed it from the registrar, we could send the event.
        // (We're not holding onto anything given to us from the component anymore)
        let _ = self.event_sender.send(Event::Component {
            id: id.deref().clone(),
            lifecycle: ComponentLifecycle::Remove,
        });

        // And then we could remove it from the map and potentially remove its parent.
        if let Some(component) = self.map.components.remove(id)
            && let Some(parent) = component.parent
        {
            let parent = AddedComponentId(parent);

            let parent_info = self.map.index_mut(&parent);
            parent_info.child_count = parent_info.child_count.saturating_sub(1);

            self.compute_removes(&parent, handle);
        }
    }
}

pub(super) struct AddChild {
    component: ErasedComponent,
    added_id: AddedComponentId,
}

impl AddChild {
    /// Adds an erased component to the map, without adding it to the registrar yet.
    ///
    /// # Errors
    ///
    /// - [`Error::NameCollision`] if a component with `child.component`'s id already exists.
    /// - [`Error::ParentNotFound`] if `child.parent` is non-none and doesn't exist in the map.
    /// - [`Error::RegistryNotFound`] if `child.component`'s registry has not been added to the app.
    pub(super) fn begin(owned: &mut Owned, component: ErasedComponent) -> Result<Self, Error> {
        let id = component.object.component_id_dyn();

        let added_id = owned.map.add(id)?;

        let _ = owned.event_sender.send(Event::Component {
            id: added_id.deref().clone(),
            lifecycle: ComponentLifecycle::Add,
        });

        Ok(Self {
            component,
            added_id,
        })
    }

    pub(super) const fn id(&self) -> &AddedComponentId {
        &self.added_id
    }

    /// Adds the component, which is already added to the map, to the registrar.
    ///
    /// # Panics
    ///
    /// - If `registry` is not the id of the registry of this component.
    pub(super) fn finish_validated(
        self,
        owned: &mut Owned,
        parent: Option<AddedComponentId>,
        registry: AddedRegistryId,
        handle: &HandleData,
    ) -> AddedComponentId {
        assert_eq!(
            &self.added_id.registry, &*registry.component,
            "the given registry id should be equal to the component's registry id"
        );

        if let Some(parent) = parent {
            owned.map.adopt(self.added_id.clone_inner(), parent);
        }

        let part = DependencyPart {
            dependent: handle.app_id().clone_inner(),
            dependency: registry.0,
            ty: DependencyType::Removable,
        };
        let registry_dependency = owned.increment_dependency(part, &handle.method_sender);

        owned.registrar.add(self.component, registry_dependency);

        self.added_id
    }

    /// Adds the component, which is already added to the map, to the registrar.
    ///
    /// # Errors
    ///
    /// - [`Error::ParentNotFound`] if `child.parent` is non-none and doesn't exist in the map.
    /// - [`Error::RegistryNotFound`] if `child.component`'s registry has not been added to the app.
    pub(super) fn finish(
        self,
        owned: &mut Owned,
        parent: Option<ComponentId>,
        handle: &HandleData,
    ) -> Result<AddedComponentId, Error> {
        let id = self.added_id.deref().clone();
        let res = (|| {
            let parent = parent
                .map(|parent| owned.map.validate_id(parent))
                .transpose()
                .map_err(|parent| Error::ParentNotFound {
                    child: id.clone(),
                    parent,
                })?;

            let registry_id = (owned.map)
                .validate_registry_id(self.added_id.registry)
                .map_err(|_| Error::RegistryNotFound {
                    component: id.clone(),
                })?;

            Ok(self.finish_validated(owned, parent, registry_id, handle))
        })();

        if res.is_err() {
            owned.map.components.remove(&id);
        }

        res
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

#[derive(Clone, Hash, PartialEq, Eq)]
pub struct DependencyId {
    pub dependent: ComponentId,
    pub dependency: ComponentId,
}

#[derive(Debug)]
pub struct DependencyPart {
    pub(super) dependent: AddedComponentId,
    pub(super) dependency: AddedComponentId,
    pub(super) ty: DependencyType,
}

impl DependencyPart {
    fn clone_inner(&self) -> Self {
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
}

impl<T: Send + Sync> Dependency<T> {
    pub async fn clone_with<U: Send>(
        this: &Self,
        app: &mut App,
        func: impl AsyncFnOnce(&T, &mut App) -> U + Send,
    ) -> Dependency<U> {
        let dependency = app
            .map_mut()
            .increment_dependency(DependencyPart {
                dependent: this.handle.id.dependent.clone_inner(),
                dependency: this.handle.id.dependency.clone_inner(),
                ty: this.handle.id.ty,
            })
            .await;
        let value = func(&this.value, app).await;
        Dependency {
            value,
            handle: dependency.handle,
        }
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
    // Dependency
    TryIncrementDependency {
        dependency: AddedComponentId,
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
    // Internal
    LoadRawDependency {
        dependency: Dependency<()>,
    },
    FinishRemoval {
        id: AddedComponentId,
    },
}

impl Method {
    pub(super) async fn run(self, app: &mut App) {
        match self {
            Self::Get { sender } => {
                let map = app.map().get().await;
                let _ = sender.send(map);
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
            Self::LoadRawDependency { dependency } => {
                app.map_mut().load_raw_dependency(dependency).await;
            }
            Self::FinishRemoval { id } => app.map_mut().finish_removal(id).await,
        }
    }
}

/// Return type of [`AppFn::map()`], see [`AppMap`].
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
        dependency: AddedComponentId,
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
                let dependent = owned.map.validate_id(dependent).map_err(|dependent| {
                    Error::DependentNotFound {
                        dependency: dependency.deref().clone(),
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
        AppFn::oneshot_mut(
            id,
            |id, sender, _| super::Method::Map(Method::IncrementDependency { id, sender }),
            async |id, owned, handle| owned.increment_dependency(id, &handle.method_sender),
        )
        .call(self.app)
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

    async fn load_raw_dependency(self, dependency: Dependency<()>) {
        let is_handle = matches!(self.app.variant, super::Inner::Handle);

        // We want to load the dependency as the dependent, so set up the app for that.
        let mut app = (self.app.dependent_mut())
            .with_ty(DependencyType::Transient)
            .with_added_dependent(dependency.info().dependent.clone_inner())
            .await;

        // Then, with an app handle, we load the dependency.
        let fut = async move |app: &mut App| {
            let res = app
                .registries_mut()
                .load(dependency.info().dependency.deref().clone())
                .await;

            let _ = app.errors_mut().drain_result(res).await;

            // Make sure that we wait to drop the dependency until here so that we keep it alive.
            drop(dependency);
        };

        // But we can't load it if we're the owned app - it would deadlock in the method queue.
        if is_handle {
            fut(&mut app).await;
        } else {
            // So if we are the owned app, we spawn another task that runs it with a handle.
            let mut handle = app.handle().await;
            tokio::task::spawn(async move {
                fut(&mut handle).await;
            });
        }
    }

    fn finish_removal(self, id: AddedComponentId) -> impl AppMutFuture<'a, Output = ()> {
        AppFn::spawn_mut(
            id,
            |id, _| super::Method::Map(Method::FinishRemoval { id }),
            async |id, owned, handle| owned.finish_removal(&id, handle),
        )
        .call(self.app)
    }
}
