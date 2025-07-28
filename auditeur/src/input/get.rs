use std::ops::{Deref, DerefMut};

use crate::prelude::*;

pub struct Get<T: Resource + GetRef>(T::Ref);

impl<T: Resource + GetRef> Input for Get<T> {
    async fn create(registry: &Registry, as_resource: &NamespacedIdRef) -> Result<Self, Error> {
        let reference =
            registry
                .get_as::<T>(as_resource)
                .await
                .ok_or_else(|| Error::DowncastFailed {
                    id: T::const_ident().into(),
                })?;
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
    type Ref: Clone + crate::Any + Send + Sync;
    /// Get a synchronized reference to `Self`
    fn get_ref(&self, registry: &Registry) -> Self::Ref;
}

/// A trait that returns [`Some(R::Ref)`](GetRef::Ref) if the type implements [`GetRef`], or [`None`] otherwise
///
/// This can be automatically implemented for all types using autoderef-based specialization,
/// allowing types that don't know if they even implement [`GetRef`] to try to create a
/// synchronized reference. See [`crate::app::GetRequest`] for why this is necessary.
///
/// To implement the trait, use [`#[resource]`](macro@resource) or
/// [`#[derive(MaybeGetRef)]`](macro@crate::MaybeGetRef)
#[cfg(feature = "runtime-loading")]
pub trait MaybeGetRef {
    fn maybe_get_ref(&self, registry: &Registry) -> Option<Box<dyn CloneAny>>;
}

#[doc(hidden)]
#[cfg(feature = "runtime-loading")]
pub mod maybe_get_ref {
    use crate::app::Registry;

    use super::CloneAny;

    pub trait Specialization {
        fn specialization(&self, registry: &Registry) -> Option<Box<dyn CloneAny>>;
    }

    impl<T> Specialization for &T {
        fn specialization(&self, _: &Registry) -> Option<Box<dyn CloneAny>> {
            None
        }
    }

    impl<T: super::GetRef + crate::Any> Specialization for &mut &T {
        fn specialization(&self, registry: &Registry) -> Option<Box<dyn CloneAny>> {
            Some(Box::new(self.get_ref(registry)))
        }
    }
}

/// A trait that allows for [cloning](Clone) within a trait object and dynamic typing
///
/// This trait is used only in [`MaybeGetRef`] and is implemented for all types that implement
/// [`Clone`] and [`Any`]
#[cfg(feature = "runtime-loading")]
pub trait CloneAny: crate::Any + Send + Sync {
    fn clone_any(&self) -> Box<dyn CloneAny>;
    fn as_any(self: Box<Self>) -> Box<dyn Any>;
}

#[cfg(feature = "runtime-loading")]
impl<T: Clone + crate::Any + Send + Sync> CloneAny for T {
    fn clone_any(&self) -> Box<dyn CloneAny> {
        Box::new(self.clone())
    }

    fn as_any(self: Box<Self>) -> Box<dyn crate::Any> {
        self
    }
}
