use libloading::Library;
use std::{path::PathBuf, sync::Arc};
use tracing::warn;

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
    library_handle: LibraryHandle,
    runtime_handle: tokio::runtime::Handle,
}

impl PluginContext {
    #[cfg(feature = "loading")]
    fn new(library_handle: LibraryHandle) -> Self {
        // NOTE: This function should only ever be called by the parent process, so the tokio
        // runtime should alredy be loaded.
        let runtime_handle = tokio::runtime::Handle::current();
        Self {
            library_handle,
            runtime_handle,
        }
    }

    /// Get an owned handle to the plugin's dynamic library
    ///
    /// See [`LibraryHandle`] for more information
    #[must_use]
    pub fn library_handle(&self) -> LibraryHandle {
        self.library_handle.clone()
    }

    /// Initialize the plugin runtime within the shared library.
    ///
    /// **NOTE: This is automatically called when using [`#[export]`]**. Repeated calls shouldn't
    /// break the runtime, but they also aren't necessary.
    pub fn init(&self) {
        // We have to initialize the tokio runtime within the plugins, since the runtime is loaded
        // in a thread_local and those don't spread across FFI.
        // WARN: By leaking the EnterGuard, the runtime may never exit...
        // TODO: Find out how to reaquire and drop the EnterGuard when the library is unloaded
        std::mem::forget(self.runtime_handle.enter());
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
/// [`ResourceMap`](auditeur::app::ResourceMap), so they will never unload while
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
/// be forgotten by the main [`ResourceMap`](auditeur::app::ResourceMap), leading to
/// libraries being freed too early.
// TODO: example here
#[derive(Clone)]
#[cfg_attr(not(feature = "loading"), allow(dead_code))]
pub struct LibraryHandle(Arc<LibraryInfo>);

#[cfg_attr(not(feature = "loading"), allow(dead_code))]
impl LibraryHandle {
    fn library(&self) -> &Library {
        &self.0.library
    }

    fn from_library_info(library: LibraryInfo) -> Self {
        Self(Arc::new(library))
    }
}

#[cfg_attr(not(feature = "loading"), allow(dead_code))]
struct LibraryInfo {
    library: Library,
    path: PathBuf,
}

impl Drop for LibraryInfo {
    fn drop(&mut self) {
        warn!("unloading library at '{}'...", self.path.display());
    }
}
