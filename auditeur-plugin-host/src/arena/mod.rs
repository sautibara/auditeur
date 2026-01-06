use std::collections::{HashMap, VecDeque};

use auditeur_plugin::auditeur::{
    app::{App, channels::EventSender, resource_map::ResourceMapConnection},
    path_any::PathAnyConst,
};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use crate::{
    arena::source_list::LoadedPluginArtifacts, prelude::*, registry::PluginRegistryConnection,
};

use self::{
    binaries::Binaries,
    loader::Loader,
    source_list::{LoadedSource, SourceList},
    workspace::{BuildArtifacts, Workspace},
};

mod binaries;
mod loader;
mod manifest;
mod source_list;
mod workspace;

// How does this all work??
//
// The Arena attempts to allow the user to declaratively describe plugins to add. Following this
// config, it runs separate actions imperatively to get to that target state (unsurprisingly, this
// philosophy is modeled after cargo, we (I) love cargo). The pipeline is generally separated into
// three areas: loading plugin sources, combining plugins into a cargo workspace, and storing the
// resulting shared libraries. These are handled by the [`SourceList`], the [`Workspace`], and the
// [`Binaries`] types respectively. These each also have their own directory, the path of which is
// also customizeable in the config.
//
// The Arena has an internal state and attempts to make that state align with the config. To do
// this, it takes little steps (defined by [`Action`]) one by one. These actions are separated,
// since the config can be changed at any time; for example, if the user wants to cancel downloading
// a package. When config is changed, the action queue is flushed and the state is "diffed" to
// figure out the actions to perform from now on (which becomes the new action queue). Notably, this
// requires the state to be updated **only after an action finishes** so that it is always an
// accurate representation of what has been done.
//
// State is held in multiple locations, and state "diff"ing happens at different times. This is
// because we may not have all the information to know how to arrive at a future state until we
// finish some of the beginnning actions. Diffing happens once when changing config in
// [`Arena::diff`], and again after loading all sources in [`Action::AddPlugins`]. This is because
// we may have to add the plugins to the workspace in a specific order because of dependencies.
//
// (sorry for writing so much, I wrote it so I would understand it better and figured I'd leave it
// in for the one or two people that will ever read this code).

// TODO: confirm deserialized state with file system
// make sure the file system agrees with the state
// TODO: runtime config changing
// when config changes, delete all upcoming actions and replace them with the new config
// TODO: validate built info after deserializing binaries
// TODO: move SourceId to a UUID

pub trait Opts: PathAnyConst + Send + Sync + Sized + 'static {
    fn default_config(&self) -> config::Host;

    fn static_dependencies(&self) -> &HashMap<SourceId, Source>;

    fn read_config(&self) -> impl Future<Output = Result<config::Host, Error>> + Send + Sync;
    fn write_config(
        &self,
        config: &config::Host,
    ) -> impl Future<Output = Result<(), Error>> + Send + Sync;

    fn read_config_lock(&self) -> impl Future<Output = Result<Lockfile, Error>> + Send + Sync;
    fn write_config_lock(
        &self,
        lock: &Arena<Self>,
    ) -> impl Future<Output = Result<(), Error>> + Send + Sync;
}

#[derive(PathAny)]
pub struct DefaultOpts {
    project_dirs: ProjectDirs,
    static_dependencies: HashMap<SourceId, Source>,
}

impl DefaultOpts {
    #[must_use]
    pub fn new(project_dirs: ProjectDirs) -> Self {
        Self {
            project_dirs,
            static_dependencies: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_static_dependency(mut self, source: Source) -> Self {
        self.static_dependencies.insert(source.id().clone(), source);
        self
    }
}

impl Opts for DefaultOpts {
    fn default_config(&self) -> config::Host {
        config::Host::defaults(&self.project_dirs)
    }

    fn static_dependencies(&self) -> &HashMap<SourceId, Source> {
        &self.static_dependencies
    }

    async fn read_config(&self) -> Result<config::Host, Error> {
        let file = self.project_dirs.config_dir().join("config.ron");
        crate::deserialize(file).await
    }

    async fn write_config(&self, config: &config::Host) -> Result<(), Error> {
        tokio::fs::create_dir_all(self.project_dirs.config_dir()).await?;
        let file = self.project_dirs.config_dir().join("config.ron");
        crate::serialize(file, config)
    }

    async fn read_config_lock(&self) -> Result<Lockfile, Error> {
        let file = self.project_dirs.config_dir().join("lock.ron");
        crate::deserialize(file).await
    }

    async fn write_config_lock(&self, lock: &Arena<Self>) -> Result<(), Error> {
        tokio::fs::create_dir_all(self.project_dirs.config_dir()).await?;
        let file = self.project_dirs.config_dir().join("lock.ron");
        crate::serialize(file, lock)
    }
}

enum Action {
    /// Add a source to the [`SourceList`]
    Load(SourceId),
    /// Remove a source from the [`SourceList`]
    Remove(SourceId),
    /// Resolve dependencies and find out how to load plugins into the workspace in the correct
    /// order.
    ResolveDependencies,
    /// Copy a plugin's manifest into the workspace.
    AddToWorkspace(PluginName),
    /// Compile all plugins in the workspace.
    CompileAll,
    /// Unload all plugins that aren't meant to be loaded.
    UnloadAll,
    /// Load all plugin objects.
    LoadAll,
}

#[derive(Serialize)]
#[resource("plugin_host:arena")]
pub struct Arena<O: Opts> {
    source_list: SourceList,
    workspace: Workspace,
    binaries: Binaries,
    #[serde(skip)]
    loader: Loader,
    #[serde(skip)]
    actions: VecDeque<Action>,
    #[serde(skip)]
    config: config::Host,
    #[serde(skip)]
    opts: O,
}

#[derive(Deserialize)]
pub struct Lockfile {
    source_list: SourceList,
    workspace: Workspace,
    binaries: Binaries,
}

#[create]
impl<O: Opts> Create for Arena<O> {
    async fn create(#[external] arena: Result<Self>) -> Result<Self> {
        arena
    }
}

impl<O: Opts> Arena<O> {
    /// Open or initialize an [`Arena`] using the specified `config`
    ///
    /// # Errors
    ///
    /// - If the directory structure cannot be created
    pub async fn open_or_init(opts: O) -> (Result<Self, Error>, Vec<Error>) {
        async fn open_or_init<O: Opts>(opts: O) -> Result<Arena<O>, Error> {
            debug!("initializing plugin arena");

            let config = match opts.read_config().await {
                Ok(config) => config,
                Err(Error::Io(io_err)) if io_err.kind() == std::io::ErrorKind::NotFound => {
                    let config = opts.default_config();
                    opts.write_config(&config).await?;
                    config
                }
                Err(err) => {
                    return Err(err);
                }
            };

            let Lockfile {
                source_list,
                workspace,
                binaries,
            } = match opts.read_config_lock().await {
                Ok(lockfile) => {
                    tokio::try_join!(
                        lockfile.source_list.init(),
                        lockfile.workspace.init(),
                        lockfile.binaries.init(),
                    )?;
                    lockfile
                }
                Err(Error::Io(io_err)) if io_err.kind() == std::io::ErrorKind::NotFound => {
                    let (source_list, workspace, binaries) = tokio::try_join!(
                        SourceList::new(config.directories().source_dir()),
                        Workspace::new_from_build_info(
                            config.directories().workspace_dir(),
                            config.arena.retain_build_artifacts,
                        ),
                        Binaries::new(config.directories().binary_dir())
                    )?;
                    Lockfile {
                        source_list,
                        workspace,
                        binaries,
                    }
                }
                Err(err) => {
                    return Err(err);
                }
            };

            let loader = Loader::default();

            let mut arena = Arena {
                source_list,
                workspace,
                binaries,
                loader,
                config,
                actions: VecDeque::new(),
                opts,
            };
            arena.source_diff();
            Ok(arena)
        }

        let mut result = open_or_init(opts).await;
        let errors = if let Ok(arena) = result.as_mut() {
            arena.load_binaries().await.err().unwrap_or_default()
        } else {
            Vec::new()
        };

        (result, errors)
    }

    async fn load_binaries(&mut self) -> Result<(), Vec<Error>> {
        let mut errs = Vec::new();
        let mut other_actions = VecDeque::new();

        for action in self.actions.drain(..) {
            if let Action::Load(id) = &action
                && let Some(source) = self.config.source(id)
                && let source::Variant::Builtin(_) = source.variant()
            {
                let res = self.source_list.load_source(source).await;
                if let Err(err) = res {
                    errs.push(err);
                }
            } else {
                other_actions.push_back(action);
            }
        }

        self.actions = other_actions;

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    fn source_diff(&mut self) {
        self.actions.clear();

        for (source, _) in self.source_list.sources() {
            if !self.config.plugins.contains_key(source)
                && !self.opts.static_dependencies().contains_key(source)
            {
                self.actions.push_back(Action::Remove(source.clone()));
            }
        }

        for source in (self.config.plugins.keys()).chain(self.opts.static_dependencies().keys()) {
            if !self.source_list.contains_source(source) {
                self.actions.push_back(Action::Load(source.clone()));
            }
        }

        self.actions.push_back(Action::ResolveDependencies);
    }

    async fn run_action(
        &mut self,
        action: Action,
        resource_map: &ResourceMapConnection,
        plugin_registry: &PluginRegistryConnection,
        registry: &Registry,
    ) -> Result<(), Vec<Error>> {
        match action {
            Action::Load(source) => {
                self.load_source(&source).await?;
            }
            Action::Remove(source) => {
                self.remove_source(&source).await?;
            }
            Action::ResolveDependencies => {
                self.resolve_dependencies().await?;
            }
            Action::AddToWorkspace(plugin_name) => {
                self.add_to_workspace(&plugin_name).await?;
            }
            Action::CompileAll => {
                self.compile().await?;
            }
            Action::UnloadAll => {
                self.unload(resource_map, registry).await?;
            }
            Action::LoadAll => {
                self.load(plugin_registry).await?;
            }
        }
        self.serialize_lock().await?;
        Ok(())
    }

    async fn serialize_lock(&self) -> Result<(), Error> {
        self.opts.write_config_lock(self).await?;
        Ok(())
    }

    async fn load_source(&mut self, source_id: &SourceId) -> Result<(), Error> {
        let Some(source) = (self.config.source(source_id))
            .or_else(|| self.opts.static_dependencies().get(source_id))
        else {
            unreachable!("arena should not attempt to load a source that's not in the config");
        };

        debug!("loading source '{}'", source);
        let (source, updated) = self.source_list.add_source(source).await?;

        match source {
            LoadedSource::Plugins { names, .. } => {
                if updated.0 || self.config.arena.developer_mode {
                    for name in names {
                        self.workspace.remove(name).await?;
                    }
                }
            }
            LoadedSource::Binary { binary } => {
                let binary = binary.clone();
                let binary = (self.source_list.get_plugin(&binary))
                    .and_then(|plugin| plugin.artifacts.as_binary())
                    .expect("binary should be included as a plugin in the source list");
                self.binaries
                    .insert_copy(&binary.name, &binary.path)
                    .await?;
            }
        }

        Ok(())
    }

    async fn remove_source(&mut self, source_id: &SourceId) -> Result<(), Error> {
        let removed = self.source_list.remove_source(source_id).await?;
        debug!("removed source '{removed:?}'");
        match removed {
            Some(LoadedSource::Plugins { names, .. }) => {
                self.binaries.remove_plugins(names.iter()).await?;
                for name in names {
                    self.workspace.remove(&name).await?;
                }
            }
            Some(LoadedSource::Binary { binary }) => {
                self.binaries.remove(&binary).await?;
            }
            _ => (),
        }

        Ok(())
    }

    async fn resolve_dependencies(&mut self) -> Result<(), Vec<Error>> {
        let mut tracker = SourceTracker::new(self);

        for plugin in self.source_list.plugins().map(|(name, _)| name) {
            tracker.add(plugin);
        }

        let SourceTracker {
            sources_to_add,
            actions,
            errors,
            ..
        } = tracker;

        if sources_to_add.is_empty() {
            self.actions.extend(actions);
            self.actions.push_back(Action::UnloadAll);
            self.actions.push_back(Action::CompileAll);
            self.actions.push_back(Action::LoadAll);
        } else {
            self.add_sources(sources_to_add).await?;
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    async fn add_to_workspace(&mut self, plugin_name: &PluginName) -> Result<(), Error> {
        debug!("adding `{plugin_name}` to the cargo workspace");

        if let Some(plugin) = self.source_list.get_plugin(plugin_name)
            && let LoadedPluginArtifacts::SourceCode { path, config } = &plugin.artifacts
        {
            let dependencies = config.dependencies.keys().collect();
            self.workspace
                .add(
                    path.clone(),
                    &dependencies,
                    plugin_name.clone(),
                    config.export.clone(),
                )
                .await?;
            self.binaries.remove(plugin_name).await?;
        }

        Ok(())
    }

    async fn compile(&mut self) -> Result<(), Vec<Error>> {
        let plugin_names: Vec<&PluginName> = self
            .source_list
            .plugins()
            .map(|(plugin, _)| plugin)
            .filter(|plugin| {
                self.source_list.get_plugin(plugin).is_some_and(|plugin| {
                    !self.opts.static_dependencies().contains_key(&plugin.source)
                })
            })
            .filter(|plugin| self.plugin_enabled(plugin).unwrap_or_default())
            .filter(|plugin| {
                self.workspace
                    .get(plugin)
                    .is_some_and(|crt| crt.export.is_none())
            })
            .filter(|plugin| self.binaries.get(plugin).is_none())
            .collect();

        if plugin_names.is_empty() {
            return Ok(());
        }

        debug!("compiling {plugin_names:?}, this may take a while...");

        let plugin_names = || plugin_names.iter().copied();
        let BuildArtifacts { artifacts, errors } = self.workspace.compile(plugin_names).await?;

        self.binaries.insert_copies(artifacts.into_iter()).await?;

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.into_iter().map(Into::into).collect())
        }
    }

    async fn unload(
        &mut self,
        resource_map: &ResourceMapConnection,
        registry: &Registry,
    ) -> Result<(), Error> {
        let plugins: Vec<_> = self
            .loader
            .loaded()
            .keys()
            .filter(|binary| !self.plugin_enabled(binary).unwrap_or_default())
            .cloned()
            .collect();

        if plugins.is_empty() {
            return Ok(());
        }

        debug!("unloading {plugins:?} from registry");

        self.loader
            .unload_all(plugins, resource_map, registry)
            .await?;
        Ok(())
    }

    async fn load(&mut self, plugin_registry: &PluginRegistryConnection) -> Result<(), Vec<Error>> {
        let binaries: Vec<_> = self
            .enabled_binaries()
            .filter(|binary| !self.loader.loaded().contains_key(&binary.name))
            .cloned()
            .collect();

        if binaries.is_empty() {
            return Ok(());
        }

        debug!("loading {binaries:?} into registry");

        self.loader.load_all(binaries, plugin_registry).await?;
        Ok(())
    }

    /// Returns `Some(true)` if the plugin is enabled, `Some(false)` if it was found and not
    /// enabled, and `None` if it was not found
    #[must_use]
    pub fn plugin_enabled(&self, plugin_name: &PluginName) -> Option<bool> {
        let source_id = self.source_list.get_plugin(plugin_name)?.id();
        if self.opts.static_dependencies().get(source_id).is_some() {
            Some(true)
        } else {
            self.config
                .plugins
                .get(source_id)
                .map(|plugin| plugin.enabled.get(plugin_name))
        }
    }

    /// Returns `Some(true)` if the source is [all enabled](config::host::Enabled::all_enabled),
    /// `Some(false)` if it was found and not enabled, and `None` if it was not found
    #[must_use]
    pub fn source_enabled(&self, source_id: &SourceId) -> Option<bool> {
        if self.opts.static_dependencies().get(source_id).is_some() {
            Some(true)
        } else {
            self.config
                .plugins
                .get(source_id)
                .map(|source| source.enabled.all_enabled())
        }
    }

    pub fn binaries(&self) -> impl Iterator<Item = &Binary> + '_ {
        self.binaries.binaries().values()
    }

    pub fn enabled_binaries(&self) -> impl Iterator<Item = &Binary> + '_ {
        self.binaries()
            .filter(|binary| self.plugin_enabled(&binary.name).unwrap_or_default())
    }

    async fn mutate_config(&mut self, func: impl FnOnce(&mut config::Host)) -> Result<(), Error> {
        (func)(&mut self.config);
        self.source_diff();
        self.opts.write_config(&self.config).await?;
        Ok(())
    }

    async fn add_sources(
        &mut self,
        sources: impl IntoIterator<Item = Source>,
    ) -> Result<(), Error> {
        self.mutate_config(|config| {
            config.plugins.extend(sources.into_iter().map(|source| {
                (
                    source.id().clone(),
                    config::host::PluginGroup {
                        source,
                        enabled: config::host::Enabled::All(false),
                    },
                )
            }));
        })
        .await
    }

    /// # Safety
    ///
    /// The plugin system calls foreign code through the use of [dynamic
    /// linking](auditeur_plugin::LibraryHandle), so it is inherently unsafe - see
    /// [`PluginRegistry::load_plugin_from_file_unchecked`] for more information. However, the
    /// boundary is checked in many ways. The main way that unsafety could come from other plugins
    /// is from them having unsafe code or intentionally going past the safety checks, which isn't
    /// something that can be prevented. Thus, thus function isn't marked unsafe.
    ///
    /// # Errors
    ///
    /// If a plugin can't be loaded
    pub fn add_to_plugin_registry(
        &mut self,
        registry: &mut PluginRegistry,
        app: &mut App,
    ) -> Result<(), Vec<Error>> {
        let binaries: Vec<_> = self.enabled_binaries().cloned().collect();

        if binaries.is_empty() {
            return Ok(());
        }

        debug!("adding {binaries:?} to the registry");

        let mut errors = Vec::new();
        for binary in binaries {
            trace!("adding '{}' to the registry", binary.path.display());
            // SAFETY: these binaries either must be builtin or built from rust projects, so they
            // should be safe to run
            let res = unsafe { registry.load_plugin_from_file_unchecked(&binary) };
            match res {
                Ok((_, bundle)) => {
                    self.loader.add_loaded(binary.name, bundle.id().to_owned());
                    app.add_bundle(bundle);
                }
                Err(err) => errors.push(err),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl<O: Opts> Watcher for Arena<O> {
    type Input = (
        EventSender,
        ResourceMapConnection,
        PluginRegistryConnection,
        Registry,
    );

    async fn run(
        &mut self,
        (sender, resource_map, plugin_registry, registry): &mut Self::Input,
    ) -> Result<()> {
        // TODO: receive actions as well
        while let Some(action) = self.actions.pop_front() {
            let res = self
                .run_action(action, resource_map, plugin_registry, registry)
                .await;
            if let Err(errors) = res {
                sender.send_errors(Self::const_ident(), errors);
            }
        }
        Ok(())
    }
}

struct SourceTracker<'a, O: Opts> {
    arena: &'a Arena<O>,
    // value is if the plugin was enabled
    already_added: HashMap<PluginName, bool>,
    errors: Vec<Error>,
    sources_to_add: Vec<Source>,
    actions: Vec<Action>,
}

impl<'a, O: Opts> SourceTracker<'a, O> {
    fn new(arena: &'a Arena<O>) -> Self {
        Self {
            arena,
            already_added: HashMap::new(),
            errors: Vec::new(),
            sources_to_add: Vec::new(),
            actions: Vec::new(),
        }
    }

    fn add(&mut self, plugin: &PluginName) -> PluginLoadResult {
        if let Some(enabled) = self.already_added.get(plugin).copied() {
            return PluginLoadResult::Success { enabled };
        }

        let mut had_error = false;

        let Some(info) = self.arena.source_list.get_plugin(plugin) else {
            return PluginLoadResult::NotFound;
        };

        let this_enabled = self.arena.plugin_enabled(plugin).unwrap_or_default();

        let LoadedPluginArtifacts::SourceCode { config, .. } = &info.artifacts else {
            return PluginLoadResult::Success {
                enabled: this_enabled,
            };
        };

        for (dependency, dep_info) in &config.dependencies {
            match self.add(dependency) {
                PluginLoadResult::Success { enabled } => {
                    if this_enabled && !enabled {
                        self.errors.push(Error::PluginDependencyNotEnabled {
                            plugin: plugin.clone(),
                            dependency: dependency.clone(),
                        });
                    }
                }
                PluginLoadResult::NotFound => {
                    if self.arena.config.arena.auto_add_dependencies
                        && let Some(source) = dep_info.source.as_ref()
                    {
                        self.sources_to_add.push(source.clone());
                    } else {
                        self.errors.push(Error::PluginDependencyNotFound {
                            plugin: plugin.clone(),
                            dependency: dependency.clone(),
                        });
                    }
                    had_error = true;
                }
                PluginLoadResult::HadError => {
                    had_error = true;
                }
            }
        }

        self.already_added.insert(plugin.clone(), this_enabled);

        if had_error {
            PluginLoadResult::HadError
        } else {
            if !self.arena.workspace.contains(plugin) {
                self.actions.push(Action::AddToWorkspace(plugin.clone()));
            }
            PluginLoadResult::Success {
                enabled: this_enabled,
            }
        }
    }
}

enum PluginLoadResult {
    Success { enabled: bool },
    NotFound,
    HadError,
}
