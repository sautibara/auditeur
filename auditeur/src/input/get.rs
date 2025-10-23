use std::ops::{Deref, DerefMut};

#[cfg(feature = "runtime-loading")]
use crate::input::CloneAs;
use crate::prelude::*;

pub struct Get<T: Resource + GetRef>(T::Ref);

impl<T: Resource + GetRef> Input for Get<T> {
    async fn create(registry: &Registry, dependent: ComponentIdRef<'_>) -> Result<Self, Error> {
        let reference = registry.get_as::<T>(dependent).await?;
        Ok(Self(reference))
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_resource(T::const_ident())
    }
}

impl<T: Resource + GetRef> Deref for Get<T> {
    type Target = T::Ref;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Resource + GetRef> DerefMut for Get<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A type that allows getting a synchronized reference to the original type
///
/// Most of the time, this is implemented using the [derive macro](crate::GetRef). The macro falls
/// back to [`Clone`], which works for special pointers like [`Arc<_>`](std::sync::Arc).
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::{input, path_any, app};
/// # pub fn main() {
/// use auditeur::prelude::*;
/// use auditeur::input::get::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef, PathAny)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let registry = Registry::new(0);
/// let reference: Shared = shared.get_ref(&registry);
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
#[allow(clippy::module_name_repetitions)] // no
pub trait GetRef {
    /// A synchronized reference to the original [`Resource`]
    type Ref: ResourceRef;
    /// Get a synchronized reference to `Self`
    fn get_ref(
        &self,
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> impl Future<Output = Result<Self::Ref, crate::Error>> + Send + Sync;
}

/// A trait that returns [`Some(R::Ref)`](GetRef::Ref) if the type implements [`GetRef`], or [`None`] otherwise
///
/// This can be automatically implemented for all types using autoderef-based specialization,
/// allowing types that don't know if they even implement [`GetRef`] to try to create a
/// synchronized reference. See [`crate::app::event::GetRequest`] for why this is necessary.
///
/// To implement the trait, use [`#[resource]`](macro@resource) or
/// [`#[derive(MaybeGetRef)]`](macro@crate::MaybeGetRef)
#[cfg(feature = "runtime-loading")]
#[async_trait]
pub trait MaybeGetRef {
    async fn maybe_get_ref(
        &self,
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Box<dyn ResourceRef>, crate::Error>;
}

#[doc(hidden)]
#[cfg(feature = "runtime-loading")]
pub mod maybe_get_ref {
    use crate::app::{ComponentIdRef, Registry};

    use super::ResourceRef;

    pub trait Specialization {
        fn specialization(
            &self,
            registry: &Registry,
            dependent: ComponentIdRef<'_>,
        ) -> impl Future<Output = Result<Box<dyn ResourceRef>, crate::Error>> + Send + Sync;
    }

    impl<T: Send + Sync> Specialization for &T {
        async fn specialization(
            &self,
            _: &Registry,
            _: ComponentIdRef<'_>,
        ) -> Result<Box<dyn ResourceRef>, crate::Error> {
            Err(crate::Error::ResourceNoGetRef)
        }
    }

    impl<T: super::GetRef + crate::Any + Send + Sync> Specialization for &mut &T {
        async fn specialization(
            &self,
            registry: &Registry,
            dependent: ComponentIdRef<'_>,
        ) -> Result<Box<dyn ResourceRef>, crate::Error> {
            self.get_ref(registry, dependent)
                .await
                .map(|value| Box::new(value) as Box<dyn ResourceRef>)
        }
    }
}

// TODO: move to dependency and generalize (no feature gate, allow any component)

/// A trait that allows for [cloning](Clone) within a trait object and dynamic typing
///
/// This trait is used only in [`MaybeGetRef`] and is implemented for all types that implement
/// [`Clone`] and [`Any`](crate::Any).
#[cfg(feature = "runtime-loading")]
pub trait ResourceRef: CloneAs + crate::Any + Send + Sync {
    #[must_use]
    fn clone_as_boxed(&self, dependent: ComponentIdRef<'_>) -> Box<dyn ResourceRef>;
}

#[cfg(feature = "runtime-loading")]
impl<T: CloneAs + crate::Any + Send + Sync> ResourceRef for T {
    fn clone_as_boxed(&self, dependent: ComponentIdRef<'_>) -> Box<dyn ResourceRef> {
        Box::new(self.clone_as(dependent))
    }
}
