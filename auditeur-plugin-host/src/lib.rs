// - have a handle to the shared library
//   - each listener, watcher, and resource automatically keeps the handle
//   - plugins are also given a cloneable handle so they can put it in any trait objects they give
//   other plugins
//     - this should be put in some Context-like struct to allow adding new things in the future

// - when plugins are reloaded their dependencies *have* to be recompiled and reloaded too
//   - this is because the dependencies will expect the dependent to be structured in a certain
//   way, which may change when the plugin is reloaded

// notice that this library *has* to stay loaded as long as the plugin is loaded in the app
// (so pretty much the lifetime of the program)
// the app uses trait objects (such as Box<dyn Runnable>) to allow for external plugins
// but since the trait objects reference code from their library, if the library gets unloaded,
// they break (they can't find the thing that they're referencing, so they segfault)

use std::{ffi::OsStr, sync::Arc};

use auditeur::{app::App, Message};
use libloading::Library;
use stabby::libloading::StabbyLibrary;
use thiserror::Error;

mod wrappers;

#[doc(hidden)]
pub use stabby::export as stabby_export;

type PluginFn<M> = fn(&mut App<M>, PluginContext);

/// A handle to a [shared library](::libloading) that contains a plugin
///
/// # Safety
///
/// Once all handles are dropped, the library containing the plugin will be automatically unloaded.
/// This'll cause future attempts to call code from this plugin to segfault (to try to access memory
/// that isn't owned by the process), as the memory which should hold the code would already be
/// freed by unloading.
///
/// All [resources](mod@auditeur::resource) are automatically given handles to their own plugin by
/// the plugin manager, so they will never unload while being used. However, trait objects given to
/// other plugins may cause undefined behavior if all handles are dropped and the code that the trait
/// object links to is called. To prevent this, handles can be put within the trait objects to
/// ensure that the library lasts at least as long as the object.
// ^ this is some prime yappage
#[derive(Clone)]
pub struct LibraryHandle(Arc<Library>);

impl LibraryHandle {
    /// Load a plugin from a dynamic library at `path` and run its `main` function
    ///
    /// # Safety
    ///
    /// Calling a function from a dynamic library is inherently unsafe, as we're trusting the
    /// it to have the correct signature. If the signature is wrong, it could lead to undefined
    /// behavior.
    ///
    /// # Errors
    ///
    /// - If the library fails to load
    /// - If the library's main function fails to load
    pub unsafe fn load_from_file_unchecked<M: Message>(
        path: impl AsRef<OsStr>,
        app: &mut App<M>,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let library = libloading::Library::new(path)?;
        let handle = Self(Arc::new(library));
        let main: libloading::Symbol<PluginFn<M>> = handle
            .library()
            .get_canaried(b"main")
            .map_err(|err| Error::CantLoadMainFunction {
                path: path.to_string_lossy().to_string(),
                additional_info: err.to_string(),
            })?;
        let context = PluginContext {
            handle: handle.clone(),
        };
        main(app, context);
        Ok(handle)
    }

    pub(crate) fn library(&self) -> &Library {
        &self.0
    }
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
    /// Get an owned handle to the plugin's dynamic library
    ///
    /// See [`LibraryHandle`] for more information
    pub fn handle(&self) -> LibraryHandle {
        self.handle.clone()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An error associated with loading a plugin dynamic library
    #[error("failed to load library: {0}")]
    LibLoading(#[from] libloading::Error),
    /// The plugin loader couldn't find or call the `main` function in a plugin
    #[error("couldn't call the main function in plugin `{path}`: {additional_info}")]
    CantLoadMainFunction {
        path: String,
        additional_info: String,
    },
}
