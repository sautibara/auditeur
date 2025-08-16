use std::collections::{HashMap, HashSet, VecDeque};

use auditeur_plugin::auditeur::app::{App, EventSender};
use tokio::try_join;
use tracing::{debug, trace, warn};

use crate::{arena::config_handler::ConfigHandler, prelude::*, registry::PluginRegistryConnection};

use self::{
    binaries::Binaries,
    source_list::{LoadedSource, SourceList},
    workspace::{BuildArtifacts, Workspace},
};

mod binaries;
mod config_handler;
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
// finish some of the beginnning actions. Specifically, state that comes from loading sources is
// held in [`State`], and each of the sections of the Arena (such as the source list or workspace)
// has their own state. Diffing happens once when changing config in [`State::diff`], and again
// after loading all sources in [`Action::AddPlugins`] (which inspects source config that's kept in
// [`State`] once it's loaded...). This is because we may have to add the plugins to the workspace
// in a specific order because of dependencies.
//
// (sorry for writing so much, I wrote it so I would understand it better and figured I'd leave it
// in for the one or two people that will ever read this code).

// TODO: serialize and deserialize all current state (State, SourceList, Binaries, etc.)
// TODO: confirm deserialized state with file system
// make sure the file system agrees with the state
// TODO: serialize and change state after every action finishes
// don't change the state immediately once config changes
// instead, modify the state as actions are run
// TODO: runtime config changing
// when config changes, delete all upcoming actions and replace them with the new config

enum Action {
    Load(SourceId),
    Remove(SourceId),
    AddPlugins,
    AddToWorkspace(PluginName),
    CompileAll,
    LoadAll,
}

#[resource("plugin_host:arena")]
pub struct Arena {
    source_list: SourceList,
    workspace: Workspace,
    binaries: Binaries,
    actions: VecDeque<Action>,
    state: State,
    config: ConfigHandler,
}

#[create]
impl Create for Arena {
    async fn create(#[external] arena: Result<Self>) -> Result<Self> {
        arena
    }
}

impl Arena {
    /// Open or initialize an [`Arena`] using the specified `config`
    ///
    /// # Errors
    ///
    /// - If the directory structure cannot be created
    pub async fn open_or_init(config: config::Host) -> (Result<Self, Error>, Vec<Error>) {
        async fn open_or_init(config: config::Host) -> Result<Arena, Error> {
            debug!("initializing plugin arena");
            let (source_list, workspace, binaries) = try_join!(
                SourceList::new(config.directories().source_dir()),
                Workspace::new_from_build_info(
                    config.directories().workspace_dir(),
                    config.arena.retain_build_artifacts,
                ),
                async {
                    let dir = config.directories().binary_dir();
                    if config.arena.developer_mode {
                        Binaries::new(dir).await
                    } else {
                        Binaries::try_deserialize_from(dir).await
                    }
                }
            )?;

            let config = ConfigHandler::new(config);

            let goal_state = config.goal_state();
            let state = goal_state.as_empty();
            let actions = state.diff(&goal_state);

            Ok(Arena {
                source_list,
                workspace,
                binaries,
                actions,
                state,
                config,
            })
        }

        let mut result = open_or_init(config).await;
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

    async fn run_action(
        &mut self,
        action: Action,
        plugin_registry: &mut PluginRegistryConnection,
    ) -> Result<(), Vec<Error>> {
        match action {
            Action::Load(source) => self.load_source(source).await?,
            Action::Remove(source) => {
                let removed = self.source_list.remove_source(&source).await?;
                debug!("removed source '{removed:?}'");
                if let Some(LoadedSource::Plugins { names, .. }) = removed {
                    self.binaries.remove_plugins(names.iter()).await?;
                }
            }
            Action::AddPlugins => {
                let mut tracker = SourceTracker::new(self);

                for plugin in self.state.to_add() {
                    tracker.load(plugin);
                }

                let SourceTracker {
                    sources_to_add,
                    actions,
                    errors,
                    ..
                } = tracker;

                if sources_to_add.is_empty() {
                    self.actions.extend(actions);
                    self.actions.push_back(Action::CompileAll);
                    self.actions.push_back(Action::LoadAll);
                } else {
                    self.add_sources(sources_to_add);
                }

                if !errors.is_empty() {
                    return Err(errors);
                }
            }
            Action::AddToWorkspace(plugin_name) => {
                debug!("adding `{plugin_name}` to the cargo workspace");
                // TODO: check dependency versions
                if let Some(plugin) = self.source_list.get_plugin(&plugin_name) {
                    let dependencies = plugin.config.dependencies.keys().collect();
                    self.workspace
                        .add(plugin.path.clone(), &dependencies, plugin_name.clone())
                        .await?;
                    self.state.mark_added(&plugin.source, &plugin_name);
                    self.state.queue_compile(&plugin.source, plugin_name);
                }
            }
            Action::CompileAll => {
                let plugin_names: Vec<&PluginName> = self
                    .state
                    .to_compile()
                    .filter(|plugin| {
                        self.source_list
                            .get_plugin(plugin)
                            // only compile plugin groups (no static sources)
                            .and_then(|info| self.config.plugin_group(&info.source))
                            .is_some_and(|group| group.enabled.get(plugin))
                    })
                    .collect();

                debug!("compiling {plugin_names:?}, this may take a while...");

                let plugin_names = || plugin_names.iter().copied();
                let BuildArtifacts { artifacts, errors } =
                    self.workspace.compile(plugin_names).await?;

                let binaries = self.binaries.insert_copies(artifacts.into_iter()).await?;
                for binary in binaries {
                    if let Some(plugin) = self.source_list.get_plugin(&binary.name) {
                        self.state.mark_compiled(&plugin.source, &binary.name);
                        self.state.queue_load(&plugin.source, binary);
                    } else {
                        warn!(
                            concat!(
                                "compiled binary for plugin '{}', ",
                                "but that plugin wasn't registered in the source list"
                            ),
                            binary.name
                        );
                    }
                }

                if !errors.is_empty() {
                    return Err(errors.into_iter().map(Into::into).collect());
                }
            }
            Action::LoadAll => {
                // we can update state now because this load can't be cancelled
                let binaries = self
                    .state
                    .extract_to_load_if(|binary| {
                        self.source_list
                            .get_plugin(&binary.name)
                            .is_some_and(|info| {
                                self.config
                                    .plugin_group(&info.source)
                                    .is_some_and(|group| group.enabled.get(&binary.name))
                            })
                    })
                    .collect();

                debug!("loading {binaries:?} into the registry");
                // SAFETY: See Self::add_to_plugin_registry for justification
                unsafe { plugin_registry.load_binaries(binaries).await };
            }
        }
        // TODO: serialize
        Ok(())
    }

    async fn load_source(&mut self, source_id: SourceId) -> Result<(), Error> {
        let Some(source) = self.config.source(&source_id) else {
            // TODO: return an error here
            return Ok(());
        };

        debug!("loading source '{}'", source);
        let (loaded_source, updated) = self.source_list.add_source(source).await?;

        let queue = self.state.add_source(source_id);

        // we don't care about loaded binaries, since we still check them in
        // enabled_binaries
        if let LoadedSource::Plugins { names, .. } = loaded_source {
            if updated.has_updated() || self.config.goal().arena.developer_mode {
                // if the source has updated, we have to reload everything
                queue.add.extend(names.iter().cloned());
            } else {
                // otherwise, we may be able to reuse stuff
                for name in names {
                    if let Some(binary) = self.binaries.get(name) {
                        queue.load.insert(binary.clone());
                    } else if self.workspace.contains(name) {
                        queue.compile.insert(name.clone());
                    } else {
                        queue.add.insert(name.clone());
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns `Some(true)` if the plugin is enabled, `Some(false)` if it was found and not
    /// enabled, and `None` if it was not found
    #[must_use]
    pub fn plugin_enabled(&self, plugin: &PluginName) -> Option<bool> {
        let source_id = self.source_list.get_plugin(plugin)?.id();
        match self.config.source_type(source_id)? {
            config_handler::SourceType::Static(_) => Some(true),
            config_handler::SourceType::Plugin(group) => Some(group.enabled.get(plugin)),
        }
    }

    /// Returns `Some(true)` if the source is [all enabled](config::host::Enabled::all_enabled),
    /// `Some(false)` if it was found and not enabled, and `None` if it was not found
    #[must_use]
    pub fn source_enabled(&self, source: &SourceId) -> Option<bool> {
        match self.config.source_type(source)? {
            config_handler::SourceType::Static(_) => Some(true),
            config_handler::SourceType::Plugin(group) => Some(group.enabled.all_enabled()),
        }
    }

    pub fn binaries(&self) -> impl Iterator<Item = &Binary> + '_ {
        (self.binaries.binaries().values())
            .chain(self.source_list.binaries().map(|(_, binary)| binary))
    }

    pub fn enabled_binaries(&self) -> impl Iterator<Item = &Binary> + '_ {
        (self
            .binaries
            .binaries()
            .values()
            .filter(|binary| self.plugin_enabled(&binary.name).unwrap_or_default()))
        .chain(
            self.source_list
                .binaries()
                .filter(|(source_id, _)| self.source_enabled(source_id).unwrap_or_default())
                .map(|(_, binary)| binary),
        )
    }

    fn mutate_config(&mut self, func: impl FnOnce(&mut config::Host)) {
        self.config.mutate(func);
        self.actions.clear();
        let goal_state = self.config.goal_state();
        let actions = self.state.diff(&goal_state);
        self.actions.extend(actions);
    }

    fn add_sources(&mut self, sources: impl IntoIterator<Item = Source>) {
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
        });
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
        &self,
        registry: &mut PluginRegistry,
        app: &mut App,
    ) -> Result<(), Error> {
        for binary in self.enabled_binaries() {
            trace!("adding '{}' to the registry", binary.path.display());
            // SAFETY: these binaries either must be builtin or built from rust projects, so they
            // should be safe to run
            unsafe { registry.load_plugin_from_file_unchecked(binary, app) }?;
        }
        Ok(())
    }
}

impl Watcher for Arena {
    type Input = (EventSender, PluginRegistryConnection);

    async fn run(&mut self, (sender, plugin_registry): &mut Self::Input) -> Result<()> {
        // TODO: receive actions as well
        while let Some(action) = self.actions.pop_front() {
            let res = self.run_action(action, plugin_registry).await;
            if let Err(errors) = res {
                sender.send_errors(Self::const_ident(), errors);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct SourceQueue {
    pub add: HashSet<PluginName>,
    pub compile: HashSet<PluginName>,
    pub load: HashSet<Binary>,
}

// TODO: serialize and deserialize this
struct State {
    // basic config
    pub directories: config::host::Directories,
    // loading sources
    pub sources: HashMap<SourceId, SourceQueue>,
}

impl State {
    #[must_use]
    fn from_config(config: &config::Host) -> Self {
        let mut state = Self {
            directories: config.directories.clone(),
            sources: HashMap::new(),
        };
        for source in (config.plugins.keys()).chain(config.static_dependencies.keys()) {
            state.add_source(source.clone());
        }
        state
    }

    fn add_source(&mut self, source: SourceId) -> &mut SourceQueue {
        self.sources.entry(source).or_default()
    }

    // TODO: handle directory changes
    fn diff(&self, next: &Self) -> VecDeque<Action> {
        let mut actions = VecDeque::new();

        for source in self.sources.keys() {
            if !next.sources.contains_key(source) {
                actions.push_back(Action::Remove(source.clone()));
            }
        }

        for source in next.sources.keys() {
            if !self.sources.contains_key(source) {
                actions.push_back(Action::Load(source.clone()));
            }
        }

        actions.push_back(Action::AddPlugins);

        actions
    }

    fn as_empty(&self) -> Self {
        Self {
            directories: self.directories.clone(),
            sources: HashMap::new(),
        }
    }

    fn source_queues(&self) -> impl Iterator<Item = &SourceQueue> {
        self.sources.values()
    }

    fn source_queues_mut(&mut self) -> impl Iterator<Item = &mut SourceQueue> {
        self.sources.values_mut()
    }

    fn source_queue_mut(&mut self, source: &SourceId) -> Option<&mut SourceQueue> {
        self.sources.get_mut(source)
    }

    fn to_add(&self) -> impl Iterator<Item = &PluginName> {
        self.source_queues().flat_map(|queue| &queue.add)
    }

    fn to_compile(&self) -> impl Iterator<Item = &PluginName> {
        self.source_queues().flat_map(|queue| &queue.compile)
    }

    fn extract_to_load_if<F>(&mut self, func: F) -> impl Iterator<Item = Binary> + use<'_, F>
    where
        F: Fn(&Binary) -> bool + Clone + Copy,
    {
        self.source_queues_mut()
            .flat_map(move |queue| queue.load.extract_if(func))
    }

    fn mark_added(&mut self, source: &SourceId, plugin: &PluginName) {
        if let Some(queue) = self.source_queue_mut(source) {
            queue.add.remove(plugin);
        }
    }

    fn mark_compiled(&mut self, source: &SourceId, plugin: &PluginName) {
        if let Some(queue) = self.source_queue_mut(source) {
            queue.compile.remove(plugin);
        }
    }

    fn queue_compile(&mut self, source: &SourceId, plugin: PluginName) {
        if let Some(queue) = self.source_queue_mut(source) {
            queue.compile.insert(plugin);
        }
    }

    fn queue_load(&mut self, source: &SourceId, binary: Binary) {
        if let Some(queue) = self.source_queue_mut(source) {
            queue.load.insert(binary);
        }
    }
}

struct SourceTracker<'a> {
    arena: &'a Arena,
    // value is if the plugin was enabled
    already_added: HashMap<PluginName, bool>,
    errors: Vec<Error>,
    sources_to_add: Vec<Source>,
    actions: Vec<Action>,
}

impl<'a> SourceTracker<'a> {
    fn new(arena: &'a Arena) -> Self {
        Self {
            arena,
            already_added: HashMap::new(),
            errors: Vec::new(),
            sources_to_add: Vec::new(),
            actions: Vec::new(),
        }
    }

    fn load(&mut self, plugin: &PluginName) -> PluginLoadResult {
        if let Some(enabled) = self.already_added.get(plugin).copied() {
            return PluginLoadResult::Success { enabled };
        }

        let mut had_error = false;

        let Some(info) = self.arena.source_list.get_plugin(plugin) else {
            return PluginLoadResult::NotFound;
        };

        let Some(source) = self.arena.config.source_type(info.id()) else {
            // this may happen if the plugin was once in the config but was later removed
            return PluginLoadResult::NotFound;
        };

        let this_enabled = match source {
            config_handler::SourceType::Static(_) => true,
            config_handler::SourceType::Plugin(group) => group.enabled.get(plugin),
        };

        for (dependency, dep_info) in &info.config.dependencies {
            match self.load(dependency) {
                PluginLoadResult::Success { enabled } => {
                    if this_enabled && !enabled {
                        self.errors.push(Error::PluginDependencyNotEnabled {
                            plugin: plugin.clone(),
                            dependency: dependency.clone(),
                        });
                    }
                }
                PluginLoadResult::NotFound => {
                    if self.arena.config.goal().arena.auto_add_dependencies
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
            self.actions.push(Action::AddToWorkspace(plugin.clone()));
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
