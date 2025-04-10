//! A replacement for [`std::any::Any`] that plays well with dynamic linking
//!
//! [`std::any::Any`] uses [`TypeId`](std::any::TypeId)s, which are based on information only
//! available at the specific time that objects are compiled - they can (and do) change between
//! different compilations. This becomes problematic when it comes to dynamic linking, as then,
//! objects are usually compiled separately. [`PathAny`], on the other hand, uses [`PathId`]s,
//! which are based on the module path of a type. This doesn't change between compilations, but it
//! does incite other problems.
//!
//! The tradeoff is twofold. For one, since this allows separate compilations to match the same
//! [`PathId`], it could lead to an object expecting one structure of a type (its specific fields,
//! methods, etc) to be given a different structure - code can be changed between compilations. In
//! `auditeur-plugin-host`, this is protected against by compiling all plugins at the same time.
//! Another issue is that, since it rests on information that isn't readily available for every
//! type (its module path), the [`PathAny`] trait must be derived separately for every type that
//! wants to implement it - it can't be automatically implemented for all types as
//! [`Any`](std::any::Any) is.

use bevy_utils_proc_macros::all_tuples;

/// A unique identifier for a type and its generics using module paths
///
/// See the [module documentation](crate::path_any) for why this type is necessary
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PathId(PathIdInner);

/// A separate [`PathId`] enum so that the implementation details are private
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[doc(hidden)]
pub enum PathIdInner {
    Single(&'static str),
    Joined(&'static PathId, &'static PathId),
}

impl PathId {
    #[must_use]
    #[doc(hidden)]
    pub const fn new(path: &'static str) -> Self {
        Self(PathIdInner::Single(path))
    }

    #[must_use]
    #[doc(hidden)]
    pub const fn join(&'static self, other: &'static Self) -> Self {
        Self(PathIdInner::Joined(self, other))
    }
}

/// A trait to emulate dynamic typing which uses [`PathId`]s
///
/// The implementation is separated into [`Self::path_id`] and [`Self::path_id_sized`], as the only
/// way to find the specific [`PathId`] of a type, even without an instance of the type, is through
/// this trait. [`std::any::Any`] is able to use [`TypeId`](std::any::TypeId), which doesn't have
/// this limitation, as the functionality is built into the compiler.
///
/// # Safety
///
/// The [`PathId`] returned by [`Self::path_id`] and [`Self::path_id_sized`] must be the same on
/// the same type and unique between all other types. Otherwise, collisions could lead to faulty
/// memory access.
pub unsafe trait PathAny: 'static {
    fn path_id(&self) -> &'static PathId;
    fn path_id_sized() -> &'static PathId
    where
        Self: Sized;
}

/// [`PathAny`] implemented using an associated constant
///
/// Note: Since the trait has an associated constant, it cannot be used to create trait objects.
/// Use [`PathAny`] instead for this use-case.
///
/// # Safety
///
/// Like [`PathAny`], the [`PathId`] given must be unique from the ids of all other types.
#[allow(clippy::module_name_repetitions)]
pub unsafe trait PathAnyConst: 'static {
    const PATH_ID: PathId;
}

unsafe impl<T: PathAnyConst + 'static> PathAny for T {
    fn path_id(&self) -> &'static PathId {
        &Self::PATH_ID
    }
    fn path_id_sized() -> &'static PathId
    where
        Self: Sized,
    {
        &Self::PATH_ID
    }
}

impl dyn PathAny {
    /// Returns `true` if the type contained within the trait object is the same type as `T`
    pub fn is<T: PathAny>(&self) -> bool {
        self.path_id() == T::path_id_sized()
    }

    /// Downcast the trait object to the type `T`, returning None if it not the correct type
    pub fn downcast_ref<T: PathAny>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(self.downcast_ref_unchecked()) }
        } else {
            None
        }
    }

    /// Downcast the trait object to the type `T` without checking if it's the same type
    ///
    /// # Safety
    ///
    /// The caller must guarantee that the trait object is of the type `T`
    pub unsafe fn downcast_ref_unchecked<T: PathAny>(&self) -> &T {
        debug_assert!(self.is::<T>());
        // SAFETY: caller guarantees that T is the correct type
        unsafe { &*std::ptr::from_ref::<dyn PathAny>(self).cast::<T>() }
    }
}

impl dyn PathAny + Send + Sync {
    /// Returns `true` if the type contained within the trait object is the same type as `T`
    pub fn is<T: PathAny>(&self) -> bool {
        self.path_id() == T::path_id_sized()
    }

    /// Downcast the trait object to the type `T`, returning None if it not the correct type
    pub fn downcast_ref<T: PathAny>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(self.downcast_ref_unchecked()) }
        } else {
            None
        }
    }

    /// Downcast the trait object to the type `T` without checking if it's the same type
    ///
    /// # Safety
    ///
    /// The caller must guarantee that the trait object is of the type `T`
    pub unsafe fn downcast_ref_unchecked<T: PathAny>(&self) -> &T {
        debug_assert!(self.is::<T>());
        // SAFETY: caller guarantees that T is the correct type
        unsafe { &*std::ptr::from_ref::<dyn PathAny>(self).cast::<T>() }
    }
}

/// Attempt to downcast the box to a concrete type.
///
/// # Errors
///
/// - Returns the original `object` if the downcast failed
pub fn downcast<T: PathAny>(object: Box<dyn PathAny>) -> Result<Box<T>, Box<dyn PathAny>> {
    if object.is::<T>() {
        unsafe { Ok(downcast_unchecked::<T>(object)) }
    } else {
        Err(object)
    }
}

/// Downcasts the box to a concrete type.
///
/// For a safe alternative see [`downcast`].
///
/// # Safety
///
/// The object must be an instance of `T`
#[must_use]
pub unsafe fn downcast_unchecked<T: PathAny>(object: Box<dyn PathAny>) -> Box<T> {
    debug_assert!(object.is::<T>());
    unsafe {
        let raw: *mut dyn PathAny = Box::into_raw(object);
        Box::from_raw(raw.cast::<T>())
    }
}

macro_rules! impl_any {
    ($($T:ident),*) => {
        unsafe impl<$($T: PathAnyConst),*> PathAnyConst for ($($T,)*) {
            const PATH_ID: PathId = PathId::new("std::tuple")
                $(.join(&$T::PATH_ID))*;
        }
    };
}

all_tuples!(impl_any, 0, 15, T);

mod tokio_sync_broadcast {
    use super::PathAnyConst;
    use auditeur_macros::implement_path_any_external;
    use tokio::sync::broadcast::Sender;

    implement_path_any_external!(Sender, <T: 'static + PathAnyConst>);
}

mod tokio_sync_mpsc {
    use super::PathAnyConst;
    use auditeur_macros::implement_path_any_external;
    use tokio::sync::mpsc::Receiver;
    use tokio::sync::mpsc::Sender;

    implement_path_any_external!(Sender, <T: 'static + PathAnyConst>);
    implement_path_any_external!(Receiver, <T: 'static + PathAnyConst>);
}

mod std_sync {
    use super::PathAnyConst;
    use auditeur_macros::implement_path_any_external;
    use std::sync::Arc;

    implement_path_any_external!(Arc, <T: 'static + PathAnyConst>);
}

mod tokio_sync_mutex {
    use super::PathAnyConst;
    use auditeur_macros::implement_path_any_external;
    use tokio::sync::Mutex;

    implement_path_any_external!(Mutex, <T: 'static + PathAnyConst>);
}

mod std_option {
    use super::PathAnyConst;
    use auditeur_macros::implement_path_any_external;

    implement_path_any_external!(Option, <T: 'static + PathAnyConst>);
}
