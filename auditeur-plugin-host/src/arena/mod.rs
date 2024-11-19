use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use auditeur_plugin::auditeur::{
    app::{App, EventSender},
    Message,
};
use log::{debug, trace};
use tokio::try_join;

use crate::{prelude::*, registry};

use self::{
    binaries::Binaries,
    source_list::{LoadedSource, SourceList},
    workspace::{BuildArtifacts, Workspace},
};

mod binaries;
mod manifest;
mod source_list;
mod workspace;

// TODO: serialize and deserialize all current state (State, SourceList, Binaries, etc.)
// TODO: confirm deserialized state with file system
// make sure the file system agrees with the state
// TODO: serialize and change state after every action finishes
// don't change the state immediately once config changes
// instead, modify the state as actions are run
// TODO: runtime config changing
// when config changes, delete all upcoming actions and replace them with the new config

enum Action {
    LoadSource(SourceId),
    RemoveSource(SourceId),
    AddToWorkspace(PluginName),
    Compile(HashSet<PluginName>),
    Load(Vec<Binary>),
}

impl Action {
    /// Returns `true` if the action is [`LoadSource`](Action::LoadSource) and the source is a
    /// [`Builtin`](source::Variant::Builtin).
    fn is_load_binary(&self, state: &State) -> bool {
        if let Self::LoadSource(source) = self {
            if let Some(source::Variant::Builtin(_)) =
                state.sources.get(source).map(|src| src.source.variant())
            {
                return true;
            }
        }
        false
    }

    fn as_compile_mut(&mut self) -> Option<&mut HashSet<PluginName>> {
        if let Self::Compile(v) = self {
            Some(v)
        } else {
            None
        }
    }

    fn as_load_mut(&mut self) -> Option<&mut Vec<Binary>> {
        if let Self::Load(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

#[resource("plugin_host:arena")]
pub struct Arena<M: Message> {
    plugin_registry: registry::Handle<M>,
    source_list: SourceList,
    workspace: Workspace,
    binaries: Binaries,
    actions: VecDeque<Action>,
    state: State,
}

#[create]
impl<M: Message> Create<M> for Arena<M> {
    async fn create(#[external] arena: Result<Self>) -> Result<Self> {
        arena
    }
}

impl<M: Message> Arena<M> {
    /// Open or initialize an [`Arena`] using the specified `config`
    ///
    /// # Errors
    ///
    /// - If the directory structure cannot be created
    pub async fn open_or_init(
        config: config::Host,
        plugin_registry: registry::Handle<M>,
    ) -> (Result<Self, Error>, Vec<Error>) {
        async fn open_or_init<M: Message>(
            config: config::Host,
            plugin_registry: registry::Handle<M>,
        ) -> Result<Arena<M>, Error> {
            debug!("initializing plugin arena");
            let (source_list, workspace, binaries) = try_join!(
                SourceList::new(config.directories().source_dir()),
                Workspace::new_from_build_info(
                    config.directories().workspace_dir(),
                    config.arena.retain_build_artifacts,
                ),
                Binaries::try_deserialize_from(config.directories().binary_dir())
            )?;

            let state = State::from_config(config);
            let actions = state.diff(&state.as_empty());

            Ok(Arena {
                plugin_registry,
                source_list,
                workspace,
                binaries,
                actions,
                state,
            })
        }

        let mut result = open_or_init(config, plugin_registry).await;
        let errors = if let Ok(arena) = result.as_mut() {
            arena.load_binaries().await.err().unwrap_or_default()
        } else {
            Vec::new()
        };

        (result, errors)
    }

    async fn load_binaries(&mut self) -> Result<(), Vec<Error>> {
        // TODO: replace with Vec::extract_if once it's stabilized
        let (load_binary_actions, rest) = std::mem::take(&mut self.actions)
            .into_iter()
            .partition(|action| action.is_load_binary(&self.state));
        self.actions = rest;

        for load_binary in load_binary_actions {
            self.run_action(load_binary).await?;
        }
        Ok(())
    }

    async fn run_action(&mut self, action: Action) -> Result<(), Vec<Error>> {
        match action {
            Action::LoadSource(source) => self.load_source(&source).await?,
            Action::RemoveSource(source) => {
                let removed = self.source_list.remove_source(&source).await?;
                debug!("removed source '{removed:?}'");
                if let Some(LoadedSource::Plugins { names, .. }) = removed {
                    self.binaries.remove_plugins(names.iter()).await?;
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
                    self.queue_compile(plugin_name);
                }
            }
            Action::Compile(plugin_names) => {
                // TODO: some sort of arena messaging thing for this kind of thing
                debug!("compiling {plugin_names:?}, this may take a while...");
                let BuildArtifacts { artifacts, errors } =
                    self.workspace.compile(&plugin_names).await?;
                // queue loading each new plugin
                let paths = self.binaries.insert_copies(artifacts.into_iter()).await?;
                self.queue_loads(paths);
                if !errors.is_empty() {
                    return Err(errors.into_iter().map(Error::CargoMessage).collect());
                }
            }
            Action::Load(paths) => {
                // debug!("loading {paths:?} into the registry");
                // SAFETY: See Self::add_to_plugin_registry for justification
                unsafe { self.plugin_registry.send(paths) };
            }
        }
        // TODO: serialize
        Ok(())
    }

    async fn load_source(&mut self, source: &SourceId) -> Result<(), Error> {
        let Some(source) = self.state.sources.get(source) else {
            return Ok(());
        };

        debug!("loading source '{}'", source.source);
        let (loaded_source, updated) = self.source_list.add_source(&source.source).await?;

        // we don't care about loaded binaries, since we still check them in
        // enabled_binaries
        if let LoadedSource::Plugins { names, .. } = loaded_source {
            if updated.has_updated() {
                // if the source has updated, we have to reload everything
                let actions = names
                    .iter()
                    .map(|name| Action::AddToWorkspace(name.clone()));
                self.actions.extend(actions);
            } else {
                // otherwise, we can reuse stuff
                for name in names.clone() {
                    if let Some(binary) = self.binaries.get(&name) {
                        self.queue_load(binary.clone());
                    } else if self.workspace.contains(&name) {
                        self.queue_compile(name);
                    } else {
                        self.actions.push_back(Action::AddToWorkspace(name));
                    }
                }
            }
        }
        Ok(())
    }

    fn queue_compile(&mut self, plugin: PluginName) {
        if let Some(action) = self.actions.iter_mut().find_map(Action::as_compile_mut) {
            action.insert(plugin);
        } else {
            let mut set = HashSet::new();
            set.insert(plugin);
            self.actions.push_back(Action::Compile(set));
        }
    }

    fn queue_loads(&mut self, paths: impl IntoIterator<Item = Binary>) {
        if let Some(action) = self.actions.iter_mut().find_map(Action::as_load_mut) {
            action.extend(paths);
        } else {
            let vec: Vec<_> = paths.into_iter().collect();
            if !vec.is_empty() {
                self.actions.push_back(Action::Load(vec));
            }
        }
    }

    fn queue_load(&mut self, path: Binary) {
        if let Some(action) = self.actions.iter_mut().find_map(Action::as_load_mut) {
            action.push(path);
        } else {
            let vec: Vec<_> = vec![path];
            self.actions.push_back(Action::Load(vec));
        }
    }

    /// Returns `Some(true)` if the plugin is enabled, `Some(false)` if it was found and not
    /// enabled, and `None` if it was not found
    #[must_use]
    pub fn plugin_enabled(&self, plugin: &PluginName) -> Option<bool> {
        let source_id = self.source_list.get_plugin(plugin)?.id();
        let source = self.state.sources.get(source_id)?;
        let enabled = source.enabled.get(plugin);
        Some(enabled)
    }

    /// Returns `Some(true)` if the source is [all enabled](config::host::Enabled::all_enabled),
    /// `Some(false)` if it was found and not enabled, and `None` if it was not found
    #[must_use]
    pub fn source_enabled(&self, source: &SourceId) -> Option<bool> {
        let source = self.state.sources.get(source)?;
        let enabled = source.enabled.all_enabled();
        Some(enabled)
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
        registry: &mut PluginRegistry<M>,
        app: &mut App<M>,
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

#[async_trait]
impl<M: Message> Watcher<M> for Arena<M> {
    type Input = EventSender;

    async fn run(&mut self, sender: &mut Self::Input) -> Result<()> {
        // TODO: receive actions as well
        while let Some(action) = self.actions.pop_front() {
            let res = self.run_action(action).await;
            if let Err(errors) = res {
                sender.send_errors(Self::const_ident(), errors);
            }
        }
        Ok(())
    }
}

// TODO: serialize and deserialize this
struct State {
    pub directories: config::host::Directories,
    pub arena: config::host::Arena,
    pub statics: HashSet<SourceId>,
    pub sources: HashMap<SourceId, SourceDefinition>,
}

impl State {
    #[must_use]
    fn from_config(config: config::Host) -> Self {
        let mut state = Self {
            directories: config.directories,
            arena: config.arena,
            sources: HashMap::new(),
            statics: HashSet::new(),
        };
        for source in config.plugins {
            state.add_plugin(source);
        }
        for source in config.static_dependencies {
            state.add_static_source(source);
        }
        state
    }

    fn add_plugin(&mut self, plugin: config::host::Plugin) -> &SourceId {
        let dependencies = plugin
            .dependencies
            .into_iter()
            .map(|dep| self.add_plugin(dep).clone())
            .collect();
        let definition = self
            .sources
            .entry(plugin.source.id().clone())
            .or_insert_with(move || SourceDefinition {
                enabled: plugin.enabled,
                source: Arc::new(plugin.source),
                dependencies,
            });
        // TODO: merge sources, if necessary
        definition.source.id()
    }

    fn add_static_source(&mut self, source: Source) {
        self.statics.insert(source.id().clone());
        self.sources.insert(
            source.id().clone(),
            SourceDefinition {
                enabled: config::host::Enabled::All(true),
                source: Arc::new(source),
                dependencies: Vec::new(),
            },
        );
    }

    // TODO: replace with generator once they get stabilized
    // TODO: handle directory changes
    fn diff(&self, prev: &Self) -> VecDeque<Action> {
        let mut actions = VecDeque::new();

        for source in prev.sources.keys() {
            if self.sources.get(source).is_none() {
                actions.push_back(Action::RemoveSource(source.clone()));
            }
        }

        // this makes sure that all dependencies are loaded in the correct order
        let mut tracker = SourceTracker::new(&self.sources, &self.statics);
        for source in self.sources.keys() {
            tracker.insert(source.clone(), prev.sources.get(source).is_some());
        }
        tracker.load(|source| actions.push_back(Action::LoadSource(source.clone())));

        actions
    }

    fn as_empty(&self) -> Self {
        Self {
            directories: self.directories.clone(),
            arena: self.arena.clone(),
            sources: HashMap::new(),
            statics: HashSet::new(),
        }
    }
}

struct SourceTracker<'sd> {
    source_is_loaded: HashMap<SourceId, bool>,
    sources: &'sd HashMap<SourceId, SourceDefinition>,
    statics: &'sd HashSet<SourceId>,
}

impl<'sd> SourceTracker<'sd> {
    fn new(
        sources: &'sd HashMap<SourceId, SourceDefinition>,
        statics: &'sd HashSet<SourceId>,
    ) -> Self {
        let source_is_loaded = sources.keys().map(|key| (key.clone(), false)).collect();
        Self {
            source_is_loaded,
            sources,
            statics,
        }
    }

    fn insert(&mut self, source: SourceId, loaded: bool) {
        self.source_is_loaded.insert(source, loaded);
    }

    fn load<CB: FnMut(&SourceId)>(mut self, mut callback: CB) {
        // static sources are given priority
        for source in self.statics {
            self.load_source(source, &mut callback);
        }
        for source in self.sources.keys() {
            self.load_source(source, &mut callback);
        }
    }

    fn load_source<CB: FnMut(&SourceId)>(&mut self, source: &SourceId, callback: &mut CB) {
        if self.source_is_loaded[source] {
            return;
        }
        let definition = &self.sources[source];
        for dependency in &definition.dependencies {
            // NOTE: we don't need to check if the dependency is loaded here, because
            // if a dependency is specified, it's always added to the list
            self.load_source(dependency, callback);
        }
        (callback)(source);
        *self.source_is_loaded.get_mut(source).unwrap() = true;
    }
}

struct SourceDefinition {
    pub enabled: config::host::Enabled,
    pub source: Arc<Source>,
    pub dependencies: Vec<SourceId>,
}
