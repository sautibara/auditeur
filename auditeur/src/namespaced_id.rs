//! An id that is prefixed with a namespace
//!
//! Usually, the namespace should be a more general id that represents many lower-level ids. For
//! example, all builtin [`resource`](mod@crate::resource)s in this crate have ids under the
//! `auditeur` namespace. This is a way to lessen the chances of name collisions while still
//! retaining an identifier that can accurately represent the resource through text
//! (`auditeur:resource_map` is a lot more recognizeable than `2dd42d85-d83d-4663-8fd1-e4e8e88faef5`).

use std::{
    alloc::Layout,
    borrow::Borrow,
    fmt::{Debug, Display},
    str::FromStr,
};

use serde::{
    de::{self, Visitor},
    Deserialize, Serialize,
};
use thiserror::Error;

/// A reference to a [`NamespacedId`]
///
/// To create a [`NamespacedIdRef`] from a static string, use [`ident!`](crate::ident).
///
/// # Examples
///
/// ```
/// # use auditeur::namespaced_id;
/// # fn main() {
/// use auditeur::prelude::*;
///
/// let ident: &'static NamespacedIdRef = ident!("namespace:id");
/// assert_eq!("namespace", ident.namespace());
/// assert_eq!("id", ident.id());
/// # }
/// ```
#[repr(C)]
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::module_name_repetitions)] // Ref is way too general
pub struct NamespacedIdRef {
    namespace_len: usize,
    // `data` holds the entire string form of the id, that is, "{namespace}:{id}"
    // this is so the data can be borrowed as a string without making a new one
    data: str,
}

impl NamespacedIdRef {
    /// Get a reference to the namespace portion of this `NamespacedId`
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.data[..self.namespace_len]
    }

    /// Get a reference to the id portion of this `NamespacedId`
    #[must_use]
    pub fn id(&self) -> &str {
        &self.data[(self.namespace_len + 1)..]
    }

    /// Get a reference to the display form of this `NamespacedId`
    ///
    /// # Examples
    ///
    /// ```
    /// # use auditeur::namespaced_id;
    /// # fn main() {
    /// use auditeur::prelude::*;
    ///
    /// let ident: &'static NamespacedIdRef = ident!("namespace:id");
    /// assert_eq!("namespace:id", ident.as_str());
    /// # }
    /// ```
    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.data
    }

    fn layout(data_len: usize) -> Layout {
        // We have to calculate the layout ourselves because it's Unsized :(
        let array_layout = Layout::array::<u8>(data_len).expect(
            "the lengths of the namespace and id shouldn't overflow a isize when combined.",
        );
        let (layout, _) = Layout::new::<usize>()
            .extend(array_layout)
            .expect("the layout of a NamespacedId shouldn't overflow an isize");
        layout.pad_to_align()
    }
}

impl Borrow<str> for &NamespacedIdRef {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Display for &NamespacedIdRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Debug for &NamespacedIdRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NamespacedIdRef({})", self.as_str())
    }
}

/// An owned simple id that is prefixed with a namespace
///
/// To create a [`NamespacedIdRef`] from a static string, use [`ident!`](crate::ident).
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct NamespacedId {
    id: Box<NamespacedIdRef>,
}

impl NamespacedId {
    /// Create a new [`NamespacedId`] using a `namespace` and `id`.
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur::prelude::*;
    ///
    /// let id = NamespacedId::new("namespace", "id");
    /// assert_eq!("namespace", id.namespace());
    /// assert_eq!("id", id.id());
    /// ```
    #[must_use]
    pub fn new(namespace: &str, id: &str) -> Self {
        let namespace_len = namespace.len();
        let id_len = id.len();
        let len = namespace_len + id_len + 1;

        // SAFETY: we completely initialize the NamespacedIdRef by copying the namespace_len,
        // and providing the data by copying the namespace and id
        unsafe {
            Self::init(len, |wide_ptr| {
                (&raw mut (*wide_ptr).namespace_len).write(namespace_len);

                let data_ptr = (&raw mut (*wide_ptr).data).cast::<u8>();
                std::ptr::copy(namespace.as_ptr(), data_ptr, namespace_len);
                data_ptr.add(namespace_len).write(b':');
                std::ptr::copy(id.as_ptr(), data_ptr.add(namespace_len + 1), id_len);
            })
        }
    }

    /// Allocate a new [`NamespacedId`] with enough space for data of length `data_len` and initialize
    /// it using the given `initializer`. The initializer is given a non-null pointer to an
    /// uninitialized [`NamespacedIdRef`] and must fully initialize it.
    ///
    /// # Safety
    ///
    /// - `initializer` must completely initialize the [`NamespacedIdRef`].
    unsafe fn init(data_len: usize, initializer: impl FnOnce(*mut NamespacedIdRef)) -> Self {
        // Allocate the correct amount of space for the NamespacedIdRef.
        // SAFETY: the size is at least the size of usize, which is greater than 1.
        let layout = NamespacedIdRef::layout(data_len);
        let ptr = unsafe { std::alloc::alloc(layout) };
        assert!(!ptr.is_null(), "the allocated pointer shouldn't be null");

        // Pointers to NamespacedIdRefs are wide pointers to hold the length of the str.
        // We give the length here :).
        let wide_ptr = std::ptr::slice_from_raw_parts_mut(ptr, data_len);
        // the pointer is guaranteed to have the correct alignment since it was allocated with that
        // alignment.
        #[allow(clippy::cast_ptr_alignment)]
        let wide_ptr = wide_ptr as *mut NamespacedIdRef;

        // According to rust rules, we cannot have a normal reference to data that isn't
        // initialized yet, otherwise undefined behavior happens.
        // Thus, we have to initialize the id through raw pointers before making the box.
        // SAFETY: we own all of this memory, so we can mutate it how we wish.
        initializer(wide_ptr);

        // Now since everything is initialized, we can create the box!
        // SAFETY: everything is initialized, the pointer is aligned and isn't null, and this is
        // only called once.
        let id = unsafe { Box::from_raw(wide_ptr) };
        Self { id }
    }
}

impl ToOwned for NamespacedIdRef {
    type Owned = NamespacedId;

    fn to_owned(&self) -> Self::Owned {
        // SAFETY: we completely initialize the NamespacedIdRef by copying our fields
        unsafe {
            NamespacedId::init(self.data.len(), |wide_ptr| {
                (&raw mut (*wide_ptr).namespace_len).write(self.namespace_len);

                let data_ptr = (&raw mut (*wide_ptr).data).cast::<u8>();
                std::ptr::copy(self.data.as_ptr(), data_ptr, self.data.len());
            })
        }
    }
}

impl From<&NamespacedIdRef> for NamespacedId {
    fn from(reference: &NamespacedIdRef) -> Self {
        reference.to_owned()
    }
}

impl Clone for NamespacedId {
    fn clone(&self) -> Self {
        NamespacedIdRef::to_owned(self)
    }
}

impl std::ops::Deref for NamespacedId {
    type Target = NamespacedIdRef;

    fn deref(&self) -> &NamespacedIdRef {
        &self.id
    }
}

impl Borrow<NamespacedIdRef> for NamespacedId {
    fn borrow(&self) -> &NamespacedIdRef {
        self
    }
}

impl Borrow<str> for NamespacedId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Display for NamespacedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Debug for NamespacedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NamespacedId({})", self.as_str())
    }
}

impl PartialEq<NamespacedIdRef> for NamespacedId {
    fn eq(&self, other: &NamespacedIdRef) -> bool {
        &**self == other
    }
}

impl PartialEq<NamespacedId> for NamespacedIdRef {
    fn eq(&self, other: &NamespacedId) -> bool {
        self == &**other
    }
}

/// An error encountered while parsing a [`NamespacedId`]
#[derive(Clone, Debug, Error)]
pub enum ParseError {
    #[error("expected a non-empty string")]
    Empty,
    #[error("incorrect amount of separator (':') characters found (expected 1, found {found}")]
    IncorrectSeparatorAmount { found: usize },
    #[error("unexpected whitespace found in namespace or id")]
    UnexpectedWhitespace,
}

impl FromStr for NamespacedId {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.split_whitespace().count() > 1 {
            return Err(ParseError::UnexpectedWhitespace);
        }
        let mut iter = s.split(':');
        let Some(namespace) = iter.next() else {
            return Err(ParseError::Empty);
        };
        let Some(id) = iter.next() else {
            return Err(ParseError::IncorrectSeparatorAmount { found: 0 });
        };
        let rest = iter.count();
        if rest > 0 {
            return Err(ParseError::IncorrectSeparatorAmount { found: rest });
        }
        Ok(Self::new(namespace, id))
    }
}

impl Serialize for NamespacedId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NamespacedId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(NamespacedIdVisitor)
    }
}

struct NamespacedIdVisitor;
impl Visitor<'_> for NamespacedIdVisitor {
    type Value = NamespacedId;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a namespaced id string in the form of namespace:id")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(de::Error::custom)
    }
}

/// A struct used in [`ident!`](macro@crate::ident) to coerce a byte array to a static
/// [`NamespacedIdRef`].
#[repr(C)]
#[doc(hidden)]
pub struct Coerce<T: ?Sized> {
    pub namespace_size: usize,
    pub data: T,
}

#[cfg(test)]
mod tests {
    use super::NamespacedId;

    #[test]
    fn deref_to_owned() {
        let id = NamespacedId::new("namespace", "id");

        let deref = &*id;
        assert_eq!("namespace", deref.namespace());
        assert_eq!("id", deref.id());

        let to_owned = deref.to_owned();
        assert_eq!("namespace", to_owned.namespace());
        assert_eq!("id", to_owned.id());
    }

    #[test]
    fn clone() {
        let id = NamespacedId::new("namespace", "id");
        #[allow(clippy::redundant_clone)] // testing the clone method itself
        let clone = id.clone();

        assert_eq!(id.namespace(), clone.namespace());
        assert_eq!(clone.id(), clone.id());

        assert!(!std::ptr::eq(id.namespace(), clone.namespace()));
        assert!(!std::ptr::eq(id.id(), clone.id()));
    }
}
