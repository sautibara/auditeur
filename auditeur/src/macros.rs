//! A separate module for imported macros just to clean up `lib.rs`

#[allow(unused_imports)] // used for doc links
use crate::prelude::*;

/// Returns a parsed static reference to a [`NamespacedIdRef`].
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::namespaced_id;
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// assert_eq!(
///     ident!("namespace:id"),
///     &NamespacedId::new("namespace", "id"),
/// );
/// # }
/// ```
pub use auditeur_macros::ident;

/// Implements [`Resource`] for any type.
///
/// This automatically implements [`Identifiable`], [`Any`], and if necessary, [`MaybeGetRef`].
///
/// # Examples
///
/// ```
/// # // see the doctest for bundle::Supplier for why this is necessary
/// # use auditeur::{namespaced_id, resource, path_any, input};
/// # pub fn main() {
/// use auditeur::prelude::*;
///
/// #[resource("test:useful_resource")]
/// struct UsefulResource;
///
/// fn assert_is_resource<R: auditeur::resource::Resource>(_: R) {}
///
/// assert_eq!(UsefulResource.ident(), ident!("test:useful_resource"));
/// assert_is_resource(UsefulResource);
/// # }
/// ```
pub use auditeur_macros::resource;

pub use auditeur_macros::identifiable;
pub use auditeur_macros::message;

pub use auditeur_macros::create;
pub use auditeur_macros::Create;

pub use auditeur_macros::Input;

// these two are used in [`macro@listener`]
#[doc(hidden)]
pub use auditeur_macros::expand_input_specifier;
#[doc(hidden)]
pub use auditeur_macros::expand_listener;

pub use auditeur_macros::has_input;
pub use auditeur_macros::listener;
pub use auditeur_macros::listener_component;
pub use auditeur_macros::Runnable;

/// Implements [`GetRef`](crate::input::get::GetRef) using [`Clone`].
///
/// # Examples
///
/// ```
/// # // see the doc test for bundle::Supplier for why this is necessary
/// # use auditeur::{input, path_any};
/// # pub fn main() {
/// use auditeur::prelude::*;
/// use auditeur::input::get::GetRef;
/// use std::sync::Arc;
///
/// #[derive(Clone, GetRef, PathAny)]
/// pub struct Shared(Arc<usize>);
///
/// let shared: Shared = Shared(Arc::new(0));
/// let reference: Shared = shared.get_ref();
///
/// assert!(Arc::ptr_eq(&shared.0, &reference.0));
/// # }
/// ```
pub use auditeur_macros::GetRef;

/// Automatically provides an implementation of [`MaybeGetRef`](crate::input::get::MaybeGetRef).
/// Usually, though, this is already implemented by [`resource`](macro@resource) or one of its
/// derivatives.
///
/// See [`MaybeGetRef`](crate::input::get::MaybeGetRef) for more information on why this is necessary
#[cfg(feature = "runtime-loading")]
pub use auditeur_macros::MaybeGetRef;

/// Automatically provides an implementation of [`PathAny`] by implementing
/// [`PathAnyConst`](crate::path_any::PathAnyConst)
#[cfg(feature = "path-any")]
pub use auditeur_macros::PathAny;
