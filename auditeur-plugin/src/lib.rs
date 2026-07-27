use auditeur::{
    children::HasChildren,
    component::{
        registry::{Loaded, Loader, Registries, Registry},
        Component, ComponentIdPath, IdentifiedComponent, RegistryOf,
    },
    prelude::{anyhow::anyhow, *},
    Error,
};
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
/// This most notably contains a [`LibraryHandle`], which is sometimes necessary to keep the
/// plugin loaded.
#[derive(Clone)]
pub struct PluginContext {
    library_handle: LibraryHandle,
}

impl PluginContext {
    #[cfg(feature = "loading")]
    #[allow(clippy::missing_const_for_fn)]
    fn new(library_handle: LibraryHandle) -> Self {
        Self { library_handle }
    }

    /// Get an owned handle to the plugin's dynamic library
    ///
    /// See [`LibraryHandle`] for more information
    #[must_use]
    pub fn library_handle(&self) -> LibraryHandle {
        self.library_handle.clone()
    }
}

impl Input for PluginContext {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.plugins_mut().get_ancestor_context().await
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

#[derive(Default)]
pub struct Plugin {
    children: Children,
}

impl Plugin {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl HasChildren for Plugin {
    fn with_erased(
        mut self,
        component: auditeur::app::ErasedComponent,
        children: Children,
    ) -> Self {
        self.children = self.children.with_erased(component, children);
        self
    }
}

pub struct PluginComponent {
    id: ComponentIdPath,
    plugin: Plugin,
    context: PluginContext,
}

impl Component<Plugins> for PluginComponent {
    type ComponentInput = Self;
    fn register(component: Self::ComponentInput) -> (<Plugins as Registry>::Loader, Children) {
        let Self {
            id,
            plugin: Plugin { children },
            context,
        } = component;

        (PluginLoader { id, context }, children)
    }
}

#[identified("plugin:plugins")]
pub struct Plugins;
impl RegistryOf for Plugins {
    type Registry = Registries;
}

impl Registry for Plugins {
    type Loader = PluginLoader;
    type Loaded = PluginLoaded;
}

#[doc(hidden)]
pub struct PluginLoader {
    id: ComponentIdPath,
    context: PluginContext,
}

impl Loader for PluginLoader {
    type Loaded = PluginLoaded;
    #[expect(clippy::unused_async_trait_impl)]
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, auditeur::Error> {
        Ok(PluginLoaded {
            context: self.context.clone(),
        })
    }
}

impl IdentifiedComponent for PluginLoader {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId {
            registry: Plugins::id(),
            component: self.id.clone(),
        }
    }
}

#[derive(Any)]
#[doc(hidden)]
pub struct PluginLoaded {
    context: PluginContext,
}

impl Loaded for PluginLoaded {}

pub trait AppExt {
    fn plugins_mut(&mut self) -> AppPluginsMut<'_>;
}

impl AppExt for App {
    fn plugins_mut(&mut self) -> AppPluginsMut<'_> {
        AppPluginsMut { app: self }
    }
}

pub struct AppPluginsMut<'a> {
    app: &'a mut App,
}

impl AppPluginsMut<'_> {
    /// Gets the context of the plugin with the id of `id`.
    ///
    /// # Errors
    ///
    /// - See [`AppComponentsMut::get`](::auditeur::app::component::AppComponentsMut::get).
    pub async fn get_context(self, id: impl Into<ComponentIdPath>) -> Result<PluginContext, Error> {
        self.app
            .components_mut()
            .get::<Plugins, _>(id.into(), async |loaded: &mut PluginLoaded, _: &mut App| {
                Ok(loaded.context.clone())
            })
            .await
            .map(|dependency| dependency.clone())
    }

    /// Gets the context of the nearest ancestor plugin of the current dependent.
    ///
    /// # Errors
    ///
    /// - [`Error::FromComponent`] iff there are no plugins in the ancestry of the current
    ///   dependent.
    /// - See [`AppComponentsMut::get`](::auditeur::app::component::AppComponentsMut::get).
    #[expect(clippy::missing_panics_doc)]
    pub async fn get_ancestor_context(self) -> Result<PluginContext, Error> {
        let map = self.app.map().get().await;

        let mut id = self.app.dependent().dependent_id().component;
        loop {
            if id.registry == Plugins::id() {
                break;
            }

            let component = map.components.get(&id).expect("components should contain id b/c it's either kept alive by the current app handle or a parent of a component kept alive");
            let Some(parent) = &component.parent else {
                return Err(self.app.errors().wrap(anyhow!("[`AppPluginsMut::get_first_ancestor`] should only be called on a descendant of a plugin")));
            };

            id = parent.clone();
        }

        self.get_context(id.into_id().component).await
    }
}
