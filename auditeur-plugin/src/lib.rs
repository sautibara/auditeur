use libloading::Library;
use std::sync::Arc;

pub use auditeur;
pub use auditeur_plugin_macros::export;
#[doc(hidden)]
pub use stabby_macros::export as stabby_export;

#[cfg(feature = "loading")]
pub mod loading;

pub mod prelude {
    pub use crate::{export, LibraryHandle, PluginContext};
    pub use auditeur::prelude::*;
}

/// Additional context given to a plugin during load
///
/// This most notably contains a [`LibraryHandle`], which is sometimes necessary to keep the plugin
/// loaded.
#[derive(Clone)]
pub struct PluginContext {
    handle: LibraryHandle,
}

impl PluginContext {
    fn new(handle: LibraryHandle) -> Self {
        Self { handle }
    }

    /// Get an owned handle to the plugin's dynamic library
    ///
    /// See [`LibraryHandle`] for more information
    pub fn handle(&self) -> LibraryHandle {
        self.handle.clone()
    }
}

/// A handle to a [shared library](::libloading) that contains a plugin
///
/// # Safety
///
/// Once all handles are dropped, the library containing the plugin will be automatically unloaded.
/// This'll cause future attempts to call code from this plugin to segfault (to try to access memory
/// that isn't owned by the process), as the memory which should hold the code would already be
/// freed by unloading.
///
/// All [resources](mod@auditeur::resource)'s lifetimes are automatically handled by the
/// [`ResourceMap`](auditeur::resource::builtin::ResourceMap), so they will never unload while
/// being used. However, trait objects given to other plugins may cause undefined behavior if all
/// handles are dropped and the code that the trait object links to is called. To prevent this,
/// handles can be put within the trait objects to ensure that the library lasts at least as long
/// as the object.
// ^ this is some prime yappage
///
/// # Common Pitfalls
///
/// ## Exiting too Early
///
/// If a listener or watcher sends resources to other threads and then exits, those references will
/// be forgotten by the main [`ResourceMap`](crate::resource::builtin::ResourceMap), leading to
/// libraries being freed too early.
// TODO: example here
#[derive(Clone)]
pub struct LibraryHandle(Arc<Library>);

impl LibraryHandle {
    #[doc(hidden)]
    pub fn library(&self) -> &Library {
        &self.0
    }

    #[doc(hidden)]
    pub fn from_library(library: Library) -> Self {
        Self(Arc::new(library))
    }
}
