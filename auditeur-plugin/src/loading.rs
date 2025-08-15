use std::{ffi::OsStr, path::PathBuf};

use auditeur::{
    app::App,
    bundle::{self, Bundle},
    namespaced_id::NamespacedId,
};
use stabby::libloading::StabbyLibrary;
use thiserror::Error;

use crate::{LibraryHandle, PluginContext};

type PluginFn = fn(PluginContext) -> Bundle;

/// A plugin and its associated [`Bundle`] that has already been loaded into an [`App`]
pub struct Plugin {
    pub library: LibraryHandle,
    pub bundle: NamespacedId,
    pub init: bundle::SharedInitFn,
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
    /// Also, if this library is unloaded before the plugin is unloaded, it could lead to a
    /// segfault if the plugin attempts to access its unloaded code.
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
        let info = super::LibraryInfo {
            library,
            path: PathBuf::from(path),
        };
        let handle = LibraryHandle::from_library_info(info);
        let main: PluginFn =
            *handle
                .library()
                .get_canaried::<PluginFn>(b"init")
                .map_err(|err| Error::CantLoadInitFunction {
                    path: path.to_string_lossy().to_string(),
                    additional_info: err.to_string(),
                })?;

        let context = PluginContext::new(handle.clone());
        let init = bundle::SharedInitFn::new(move || main(context.clone()));

        let bundle = init.call();
        let bundle_id = bundle.id().to_owned();
        app.add_bundle(bundle);

        Ok(Self {
            library: handle,
            bundle: bundle_id,
            init,
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
