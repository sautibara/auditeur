use std::ffi::OsStr;

use auditeur::{app::App, bundle::Bundle, namespaced_id::NamespacedId};
use stabby::libloading::StabbyLibrary;
use thiserror::Error;

use crate::{LibraryHandle, PluginContext};

type PluginFn = fn(PluginContext) -> Bundle;

/// A plugin and its associated [`Bundle`] that has already been loaded into an [`App`]
pub struct Plugin {
    pub library: LibraryHandle,
    pub bundle: NamespacedId,
}

impl Plugin {
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
    pub unsafe fn load_from_file_unchecked(
        path: impl AsRef<OsStr>,
        app: &mut App,
    ) -> Result<Self, Error> {
        let path = path.as_ref();
        let library = libloading::Library::new(path)?;
        let handle = LibraryHandle::from_library(library);
        let main: libloading::Symbol<PluginFn> =
            handle
                .library()
                .get_canaried(b"init")
                .map_err(|err| Error::CantLoadInitFunction {
                    path: path.to_string_lossy().to_string(),
                    additional_info: err.to_string(),
                })?;
        let context = PluginContext::new(handle.clone());
        let bundle = main(context);
        let bundle_id = bundle.id().to_owned();
        app.add_bundle(bundle);
        Ok(Self {
            library: handle,
            bundle: bundle_id,
        })
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// An error associated with loading a plugin dynamic library
    #[error("failed to load library: {0}")]
    LibLoading(#[from] libloading::Error),
    /// The plugin loader couldn't find or call the `main` function in a plugin
    #[error("couldn't call the init function in plugin `{path}`: {additional_info}")]
    CantLoadInitFunction {
        path: String,
        additional_info: String,
    },
}
