use std::marker::PhantomData;

use bevy_utils_proc_macros::all_tuples;
use futures::Future;

// TODO: implement Input for NamespacedId, make it the resource's id

use crate::prelude::*;
use std::collections::HashSet;

pub mod dependency;
pub mod get;
pub mod subset;

/// Values that can be obtained from the [`Registry`] when building resources.
///
/// # Dependencies
///
/// These must hold [`dependency::Handle`]s for all dependencies that these inputs contain. If this
/// is derived, then it is already handled for you. But if it's done manually, then please make
/// sure that this is the case.
pub trait Input: Sized + Send + Sync + 'static {
    /// Create `Self` using the given [`Registry`] and id `as_resource`
    ///
    /// # Errors
    ///
    /// - If the input failed to gather the resources necessary
    fn create(
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> impl Future<Output = Result<Self, Error>> + Send;

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

impl<I: Input> Input for Option<I> {
    async fn create(registry: &Registry, dependent: ComponentIdRef<'_>) -> Result<Self, Error> {
        I::create(registry, dependent).await.map(Some)
    }

    fn constraints() -> Constraints {
        I::constraints()
    }
}

impl<T: Send + Sync + 'static> Input for PhantomData<T> {
    async fn create(_: &Registry, _: ComponentIdRef<'_>) -> Result<Self, Error> {
        Ok(Self)
    }

    fn constraints() -> Constraints {
        Constraints::empty()
    }
}

macro_rules! impl_input {
    ($($T:ident),*) => {
        impl<$($T: Input),*> Input for ($($T,)*) {
            #[allow(unused_variables)]
            async fn create(registry: &Registry, dependent: ComponentIdRef<'_>) -> Result<Self, Error> {
                futures::try_join!($($T::create(registry, dependent),)*)
            }

            fn constraints() -> Constraints {
                Constraints::empty()
                    $(.extend($T::constraints()))*
            }
        }
    };
}

all_tuples!(impl_input, 0, 15, T);

/// A set of [constraints](Constraint) that must be satisfied before a [resource](Resource) or
/// [bundle](Bundle) can be loaded in an [app](App)
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Constraints {
    constraints: HashSet<Constraint>,
}

impl Constraints {
    /// Create a new, empty set of [constraints](Constraint)
    #[must_use]
    pub fn empty() -> Self {
        Self {
            constraints: HashSet::new(),
        }
    }

    /// Insert a [`Constraint`] into the set
    #[must_use]
    pub fn insert(mut self, constraint: Constraint) -> Self {
        self.constraints.insert(constraint);
        self
    }

    /// Specify that the component with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_component(self, id: impl Into<ComponentId>) -> Self {
        self.insert(Constraint::After(id.into()))
    }

    /// Specify that the [`Resource`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_resource(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Resource(id.into()))
    }

    /// Specify that the [`Bundle`] with id `id` must be loaded before loading this resource or
    /// bundle
    #[must_use]
    pub fn after_bundle(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Bundle(id.into()))
    }

    /// Specify that the [`Message`](crate::Message) with id `id` must be loaded before loading this
    /// resource or bundle
    #[must_use]
    pub fn after_message(self, id: impl Into<NamespacedId>) -> Self {
        self.after_component(ComponentId::Message(id.into()))
    }

    /// Combine the constraints in both this set and the `other` set, removing any duplicates
    #[must_use]
    pub fn extend(mut self, other: Self) -> Self {
        self.constraints.extend(other.constraints);
        self
    }

    /// Returns `true` if there are no [constraints](Constraint) within this set
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    /// Obtain a borrowing iterator over each [`Constraint`] in the set
    pub fn iter(&self) -> impl Iterator<Item = &Constraint> {
        self.constraints.iter()
    }

    /// Creates an iterator over the dependency links that model this constraint.
    pub fn into_dependencies(
        self,
        dependent: ComponentIdRef<'_>,
    ) -> impl Iterator<Item = dependency::Link> {
        self.constraints
            .into_iter()
            .map(move |constraint| match constraint {
                Constraint::After(component) => dependency::Link {
                    dependency: component,
                    dependent: dependent.into_owned(),
                },
            })
    }

    /// Creates an iterator over dependency handles that model this constraint
    pub fn into_dependency_handles(
        self,
        dependent: ComponentIdRef<'_>,
        event_sender: EventSender,
        removable: dependency::Removable,
    ) -> impl Iterator<Item = dependency::Handle> {
        self.into_dependencies(dependent)
            .map(move |link| dependency::Full { link, removable })
            .map(move |full| dependency::Handle::new(event_sender.clone(), full))
    }
}

impl IntoIterator for Constraints {
    type Item = Constraint;
    type IntoIter = std::collections::hash_set::IntoIter<Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.into_iter()
    }
}

impl<'a> IntoIterator for &'a Constraints {
    type Item = &'a Constraint;
    type IntoIter = std::collections::hash_set::Iter<'a, Constraint>;

    fn into_iter(self) -> Self::IntoIter {
        self.constraints.iter()
    }
}

/// A constraint that must be satisfied before a [resource](Resource) or [bundle](Bundle) can be
/// loaded in an [app](App)
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Constraint {
    /// Only load the resource after the other component with the id `id` has been loaded
    After(ComponentId),
}

// TODO: docs
pub trait CloneAs {
    #[must_use]
    fn clone_as(&self, dependent: ComponentIdRef<'_>) -> Self
    where
        Self: Sized;
}

impl<T: CloneAs> CloneAs for Option<T> {
    fn clone_as(&self, dependent: ComponentIdRef<'_>) -> Self
    where
        Self: Sized,
    {
        self.as_ref().map(|val| val.clone_as(dependent))
    }
}

#[doc(hidden)]
pub mod clone_as {
    use crate::app::ComponentIdRef;

    pub trait Specialization<T> {
        fn specialization(&self, dependent: ComponentIdRef<'_>) -> T;
    }

    impl<T: Clone + Send + Sync> Specialization<T> for &T {
        fn specialization(&self, _: ComponentIdRef<'_>) -> T {
            (*self).clone()
        }
    }

    impl<T: super::CloneAs> Specialization<T> for &mut &T {
        fn specialization(&self, dependent: ComponentIdRef<'_>) -> T {
            self.clone_as(dependent)
        }
    }
}
