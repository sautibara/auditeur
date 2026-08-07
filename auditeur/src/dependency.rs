use std::{
    borrow::Borrow,
    fmt::{Debug, Display},
    hash::Hash,
    ops::{AddAssign, Deref, DerefMut, SubAssign},
};

use crate::{
    app::{AppMutFuture, Owned},
    component::{ComponentIdPath, registry::Registries},
    input::CloneIn,
    prelude::*,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LoadIndex(pub(crate) usize);

impl Display for LoadIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An id of a component that is guaranteed to be currently added in the map when returned by it.
///
/// Note that this guarantee is only valid at the exact moment that the id is obtained from the
/// map - keeping this id alive doesn't keep the component added.
// WARN: Users of the library should not be able to give these to prove that a component is added
// - it should only be used as a guarantee within the library.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AddedComponentId(pub ComponentId);

impl AddedComponentId {
    pub(crate) const fn new_inner(id: ComponentId) -> Self {
        Self(id)
    }

    pub(crate) fn app() -> Self {
        Self(App::component_id())
    }

    pub(crate) const fn loaded_unchecked(self, index: LoadIndex) -> LoadedComponentId {
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

impl Borrow<ComponentId> for AddedComponentId {
    fn borrow(&self) -> &ComponentId {
        &self.0
    }
}

impl Display for AddedComponentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ComponentId> for AddedComponentId {
    fn from(value: ComponentId) -> Self {
        Self(value)
    }
}

pub(crate) type RegistryId = &'static NamespacedIdRef;

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct LoadedRegistryId(pub RegistryId);

impl LoadedRegistryId {
    pub(crate) fn component_id(&self) -> AddedComponentId {
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

impl Deref for LoadedComponentId {
    type Target = AddedComponentId;
    fn deref(&self) -> &Self::Target {
        &self.component
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
    pub(crate) fn new_unchecked(id: DependencyId) -> Self {
        Self {
            dependent: AddedComponentId(id.dependent),
            dependency: AddedComponentId(id.dependency),
        }
    }

    pub(crate) fn into_id(self) -> DependencyId {
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
    /// The dependency cannot be removed. If this dependency is requested to be removed, the request
    /// will propagate to the dependent.
    Unremovable,
    /// The dependency can be removed, but the app has to ask. The request will not propagate to
    /// the dependent, and instead an
    /// [`Event::RequestDependencyRemoval`](crate::app::Event::RequestDependencyRemoval) will be
    /// sent.
    Removable,
    /// The dependency is short-lived and will be removed automatically without the app asking. The
    /// request will not propagate to the dependent.
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
    pub(crate) const fn is_zero(&self) -> bool {
        self.unremovable == 0 && self.removable == 0 && self.transient == 0 && self.weak == 0
    }

    pub(crate) const fn effective_type(&self) -> DependencyType {
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

    pub(crate) fn clone_with_owned<U: Send>(
        this: &Self,
        owned: &mut Owned,
        sender: &flume::Sender<crate::app::Method>,
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
        sender: &flume::Sender<crate::app::Method>,
    ) -> Self
    where
        T: Clone,
    {
        Self::clone_with_owned(this, owned, sender, |val, _| val.clone())
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
        sender: &flume::Sender<crate::app::Method>,
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
    sender: Option<flume::Sender<crate::app::Method>>,
}

impl DependencyHandle {
    pub(crate) const fn new(id: DependencyPart, sender: flume::Sender<crate::app::Method>) -> Self {
        Self {
            id,
            sender: Some(sender),
        }
    }

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

        let event = crate::app::Method::Map(crate::app::map::Method::DecrementDependency {
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
    fn drop_in_owned(mut self, owned: &mut Owned, sender: &flume::Sender<crate::app::Method>) {
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
