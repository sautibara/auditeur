use bevy_utils_proc_macros::all_tuples;

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct PathId(PathIdInner);

// Separate the enum so that the implementation details are private
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
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

/// A trait to emulate dynamic typing similar to [`Any`](std::any::Any), that plays well with FFI
///
/// Instead of integers provided by the compiler used in [`std::any`], this uses the module paths
/// of each type and their generics. However, this requires implementing the trait for every type
/// that wants to use it, which brings some unsafety and issues with ergonomics.
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
/// Use [`PathAny`] instead.
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
        unsafe { &*(self as *const dyn PathAny).cast::<T>() }
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
