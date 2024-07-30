#![allow(dead_code, unused_variables)]

use std::{
    collections::HashMap,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use auditeur::{app::App, Message};
use cargo_toml::Manifest;
use git2::Repository;
use serde::{Deserialize, Serialize};
use tokio::{fs::File, io::AsyncWriteExt, process::Command};

use crate::{config::BinarySource, prelude::*};

async fn serialize<T: Serialize + Send + 'static>(path: PathBuf, value: T) -> Result<T, Error> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::create(&path)?;
        ron::ser::to_writer(file, &value)
            .map_err(|error| Error::Serialization { error, file: path })?;
        Ok::<_, Error>(value)
    })
    .await
    .expect("serialization shouldn't panic")
}

async fn deserialize<T: for<'de> Deserialize<'de> + Send + 'static>(
    path: PathBuf,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path)?;
        let value = ron::de::from_reader(file)
            .map_err(|error| Error::Deserialization { error, file: path })?;
        Ok(value)
    })
    .await
    .expect("deserialization shouldn't panic")
}

/// Returns `true` if `path` exists and isn't empty
fn path_exists_with_children(path: &Path) -> Result<bool, Error> {
    Ok(path.exists() && path.is_dir() && std::fs::read_dir(path)?.next().is_some())
}

pub struct Built;
pub struct Unbuilt;

pub struct Arena<B> {
    arena_dir: PathBuf,
    output_dir: PathBuf,
    config: config::host::Arena,
    loaded: HashMap<PluginName, LoadedPlugin>,
    binaries: HashMap<PluginName, PathBuf>,
    error_sink: ErrorSink,
    built_tag: PhantomData<B>,
}

impl<B> Arena<B> {
    fn convert<T>(self) -> Arena<T> {
        let Self {
            arena_dir,
            output_dir,
            config,
            loaded,
            binaries,
            error_sink,
            ..
        } = self;
        Arena {
            arena_dir,
            output_dir,
            config,
            loaded,
            binaries,
            error_sink,
            built_tag: PhantomData,
        }
    }
}

impl Arena<Unbuilt> {
    pub async fn open_or_init(config: &config::Host) -> Self {
        let mut error_sink = ErrorSink::new();
        let arena_dir = config.directories().arena_dir();
        let binaries =
            Self::deserialize_binary_file(arena_dir.join("built_binaries.ron"), &mut error_sink)
                .await;
        let mut arena = Self {
            arena_dir,
            output_dir: config.directories().plugin_dir(),
            config: config.arena.clone(),
            loaded: HashMap::new(),
            error_sink,
            binaries,
            built_tag: PhantomData,
        };
        let source_loads = config.plugins.iter().map(|plugin| async {
            arena
                .load_source(plugin)
                .await
                .map_err(|err| (plugin.source.clone(), err))
        });
        // load each source at the same time, then add them in sequence
        // they can't all be added at the same time, as that create multiple mutable references
        for result in futures::future::join_all(source_loads).await {
            match result {
                Ok(loaded_source) => drop(arena.add_loaded_source(loaded_source)),
                Err((source, error)) => arena.error_sink.add_for_source(source, error),
            }
        }
        arena
    }

    async fn deserialize_binary_file(
        file: PathBuf,
        error_sink: &mut ErrorSink,
    ) -> HashMap<PluginName, PathBuf> {
        let binaries = if file.exists() {
            deserialize(file)
                .await
                .map_err(|err| error_sink.add_generic(err))
                .ok()
        } else {
            None
        };
        binaries.unwrap_or_default()
    }

    pub async fn build(mut self) -> Result<Arena<Built>, Self> {
        if let Err(err) = self.create_workspace().await {
            self.error_sink.add_generic(err);
            return Err(self);
        }
        self.build_plugins().await;
        if !self.config.retain_build_artifacts {
            self.remove_target_dir().await;
        }
        // all plugins should be built now, so we can say that we've been built
        Ok(self.convert())
    }

    async fn create_workspace(&self) -> Result<(), Error> {
        let paths = self.loaded.values().map(|plugin| &plugin.path);
        let path_list =
            paths
                .map(|path| path.to_string_lossy())
                .fold('\n'.to_string(), |mut string, path| {
                    string.push_str(&format!("\"{path}\",\n"));
                    string
                });
        let workspace_toml = format!(
            r#"
[workspace]

members = [{path_list}]
resolver = "2"
"#
        );
        let mut file = File::create(self.arena_dir.join("Cargo.toml")).await?;
        file.write_all(workspace_toml.as_bytes()).await?;
        Ok(())
    }

    async fn build_plugins(&mut self) {
        let plugin_builds = self
            .loaded
            .values_mut()
            .filter(|plugin| self.binaries.get(plugin.name()).is_none())
            .map(|plugin| async {
                (
                    plugin.name().clone(),
                    Self::build_plugin(plugin, &self.arena_dir, self.config.retain_build_artifacts)
                        .await,
                )
            });
        for (name, status) in futures::future::join_all(plugin_builds).await {
            match status {
                Ok(binary) => drop(self.binaries.insert(name, binary)),
                Err(error) => self.error_sink.add_for_plugin(name, error),
            }
        }
    }

    async fn build_plugin(
        plugin: &mut LoadedPlugin,
        arena_dir: &Path,
        retain_target: bool,
    ) -> Result<PathBuf, Error> {
        let output = Command::new("cargo")
            .arg("build") // FIXME: check for --release
            .current_dir(&plugin.path)
            .env("CARGO_INCREMENTAL", if retain_target { "1" } else { "0" })
            .output()
            .await?;
        if output.status.success() {
            // FIXME: check for windows .dll and --release
            // FIXME: use cargo.toml name instead of plugin name
            Ok(arena_dir.join(format!("target/debug/lib{}.so", plugin.name().0)))
        } else {
            Err(Error::BuildFailed {
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }

    async fn remove_target_dir(&mut self) {
        let target_dir_path = self.arena_dir.join("target");
        let res = tokio::fs::remove_dir_all(&target_dir_path).await;
        if let Err(err) = res {
            self.error_sink.add_generic(Error::Io(err));
        }
    }

    async fn link_workspace(&mut self) -> Result<(), Error> {
        for plugin in self.loaded.values() {
            if let Err(err) = Self::link_plugin(plugin, &self.loaded).await {
                self.error_sink.add_for_plugin(plugin.name().clone(), err);
            }
        }
        Ok(())
    }

    async fn link_plugin(
        plugin: &LoadedPlugin,
        loaded: &HashMap<PluginName, LoadedPlugin>,
    ) -> Result<(), Error> {
        let path = plugin.path.join("Cargo.toml");
        let mut manifest = Manifest::from_path(&path).map_err(|error| Error::CargoParse {
            error,
            path: path.clone(),
        })?;
        // TODO: respect version
        for dependency in plugin.info.dependencies.keys() {
            let dependency =
                loaded
                    .get(dependency)
                    .ok_or_else(|| Error::UnsatisfiedDependency {
                        for_plugin: plugin.name().clone(),
                        dependency: dependency.clone(),
                    })?;
            manifest.dependencies.insert(
                // FIXME: use cargo name
                dependency.name().clone().0,
                cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                    path: Some(path.to_string_lossy().into_owned()),
                    ..Default::default()
                })),
            );
            let mut file = File::create(&path).await?;
            let serialized = toml::ser::to_string_pretty(&manifest).map_err(|error| {
                Error::TomlSerialization {
                    error,
                    path: path.clone(),
                }
            })?;
            file.write_all(serialized.as_bytes()).await?;
        }
        Ok(())
    }
}

impl Arena<Built> {
    pub fn binaries(&self) -> &HashMap<PluginName, PathBuf> {
        &self.binaries
    }

    pub(crate) unsafe fn add_binaries_to_registry<M: Message>(
        &mut self,
        registry: &mut PluginRegistry<M>,
        app: &mut App<M>,
    ) {
        for (name, binary) in self
            .binaries
            .iter()
            .filter(|(name, binary)| self.loaded.get(name).is_some_and(|plugin| plugin.enabled))
        {
            let res = unsafe {
                registry.load_plugin_from_file_unchecked(self.output_dir.join(binary), app)
            };
            if let Err(err) = res {
                self.error_sink.add_generic(err);
            }
        }
    }

    pub fn downgrade(self) -> Arena<Unbuilt> {
        self.convert()
    }
}

impl<B> Arena<B> {
    pub fn errors(&self) -> &ErrorSink {
        &self.error_sink
    }

    pub fn finish(self) -> ErrorSink {
        self.error_sink
    }
}

enum LoadedSource {
    Plugins(Vec<LoadedPlugin>, Updated),
    Binary(BinarySource),
}

impl Arena<Unbuilt> {
    pub async fn add_source(&mut self, source: &config::host::Plugin) -> Result<Updated, Error> {
        let loaded = self.load_source(source).await?;
        Ok(self.add_loaded_source(loaded))
    }

    async fn load_source(&self, plugin_spec: &config::host::Plugin) -> Result<LoadedSource, Error> {
        let (path, updated) = match &plugin_spec.source {
            Source::Github(GitShorthand { user, repo }) => {
                self.load_git_source(&format!("https://github.com/{user}/{repo}"), repo)
                    .await
            }
            Source::Gitlab(GitShorthand { user, repo }) => {
                self.load_git_source(&format!("https://gitlab.com/{user}/{repo}"), repo)
                    .await
            }
            Source::Bitbucket(GitShorthand { user, repo }) => {
                self.load_git_source(&format!("https://bitbucket.org/{user}/{repo}"), repo)
                    .await
            }
            Source::Builtin(binary) => {
                // there's no more processing that needs to be done, so return
                return Ok(LoadedSource::Binary(binary.clone()));
            }
            Source::Path(path) => self.load_path_source(path).await,
        }?;
        let plugins = Self::load_path(path).await?;
        // workspaces may return multiple plugins, so we have to insert all of them
        let plugins = plugins
            .into_iter()
            .map(|mut plugin| {
                plugin.enabled = plugin_spec.enabled.get(plugin.name());
                plugin
            })
            .collect();
        Ok(LoadedSource::Plugins(plugins, updated))
    }

    fn add_loaded_source(&mut self, res: LoadedSource) -> Updated {
        match res {
            LoadedSource::Binary(binary) => {
                self.binaries.insert(binary.name, binary.path);
                Updated(false)
            }
            LoadedSource::Plugins(plugins, updated) => {
                // if the source updates, then all binaries should get invalidated
                if updated.has_updated() {
                    for plugin in &plugins {
                        self.binaries.remove(plugin.name());
                    }
                }
                self.loaded.extend(
                    plugins
                        .into_iter()
                        .map(|plugin| (plugin.name().clone(), plugin)),
                );
                updated
            }
        }
    }

    async fn load_path(path: PathBuf) -> Result<Vec<LoadedPlugin>, Error> {
        let mut plugins = vec![];
        // avoid recursion by using a queue
        // (recursion with async is iffy - it requires [`Box`]ing futures)
        let mut paths = vec![path];
        while let Some(path) = paths.pop() {
            let info = deserialize::<config::Plugin>(path.join("plugin.ron")).await?;
            match info {
                config::Plugin::Single(info) => plugins.push(Self::load_plugin(path, info).await?),
                config::Plugin::Workspace(workspace) => {
                    // while we could try to load all of the paths in a workspace at the same time
                    // in an aim to improve performance, it's needlessly complex and there's a good
                    // chance that it wouldn't improve performance by much anyways: we're mostly
                    // constrainted by interacting with the disk (which can't be multithreaded) and
                    // this is a fairly quick thing to do in the scheme of the whole application
                    // anyways
                    paths.extend(workspace.paths.iter().map(|child| path.join(child)));
                }
            }
        }
        Ok(plugins)
    }

    async fn load_plugin(
        path: PathBuf,
        info: config::plugin::Single,
    ) -> Result<LoadedPlugin, Error> {
        Ok(LoadedPlugin {
            path,
            info,
            enabled: false,
        })
    }

    async fn load_git_source(
        &self,
        url: &str,
        repo_name: &str,
    ) -> Result<(PathBuf, Updated), Error> {
        let path = self.arena_dir.join(repo_name);
        let updated = if !path_exists_with_children(&path)? {
            // this is a dumb amount of cloning required due to the 'static bound on spawn_blocking
            // - at times like these I wish I was using some type of structured concurrency
            let path = path.to_owned();
            let repo_url = url.to_owned();
            let error_url = url.to_owned();
            tokio::task::spawn_blocking(move || Repository::clone_recurse(&repo_url, &path))
                .await
                .expect("repository clone shouldn't panic")
                .map_err(|error| Error::Git {
                    error,
                    url: error_url,
                })?;
            true
        } else {
            // TODO: check for updates
            false
        };
        Ok((path, Updated(updated)))
    }

    async fn load_path_source(&self, source: &Path) -> Result<(PathBuf, Updated), Error> {
        if !source.exists() || !source.is_dir() {
            return Err(Error::PathNotFound(source.to_owned()));
        }
        let path = source
            .file_name()
            .ok_or_else(|| Error::NoComponentsInPath(source.to_owned()))?;
        let path = self.arena_dir.join(path);
        let updated = if !path_exists_with_children(&path)? {
            tokio::fs::copy(source, &path).await?;
            true
        } else {
            // TODO: check for updates
            false
        };
        Ok((path, Updated(updated)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Updated(pub bool);

impl Updated {
    pub fn has_updated(&self) -> bool {
        let Updated(updated) = self;
        *updated
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct LoadedPlugin {
    // only store the name here for ease of access
    info: config::plugin::Single,
    path: PathBuf,
    enabled: bool,
}

impl LoadedPlugin {
    fn name(&self) -> &PluginName {
        &self.info.plugin.name
    }
}
