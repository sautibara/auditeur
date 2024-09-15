use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{prelude::*, source::Updated};

#[derive(Clone, Debug)]
pub enum Artifacts {
    Plugins {
        source: Arc<Source>,
        path: PathBuf,
        plugins: Vec<LoadedPlugin>,
        updated: Updated,
    },
    Binary {
        source: Arc<Source>,
        binary: source::Binary,
    },
}

#[derive(Clone, Debug)]
pub enum LoadedSource {
    Plugins {
        path: PathBuf,
        names: Vec<PluginName>,
    },
    Binary {
        binary: source::Binary,
    },
}

#[derive(Clone, Debug)]
pub struct LoadedPlugin {
    path: PathBuf,
    source: Arc<Source>,
    config: config::plugin::Single,
}

impl LoadedPlugin {
    pub const fn name(&self) -> &PluginName {
        self.config.name()
    }
}

pub struct SourceList {
    path: PathBuf,
    binaries: HashMap<PluginName, PathBuf>,
    sources: HashMap<Arc<Source>, LoadedSource>,
    plugins: HashMap<PluginName, LoadedPlugin>,
}

impl SourceList {
    pub async fn new(path: impl Into<PathBuf> + Send) -> Result<Self, Error> {
        let path = path.into();
        tokio::fs::create_dir_all(&path).await?;
        Ok(Self {
            path,
            binaries: HashMap::new(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
        })
    }

    pub async fn add_source(
        &mut self,
        source: Arc<Source>,
    ) -> Result<(&LoadedSource, Updated), Error> {
        let artifacts = self.load_source(source).await?;
        let updated = self.add_artifacts(artifacts);
        Ok(updated)
    }

    pub async fn load_source(&self, source: Arc<Source>) -> Result<Artifacts, Error> {
        match source.load_into(&self.path).await? {
            source::Artifact::Path { path, updated } => Ok(Artifacts::Plugins {
                plugins: Self::scan_workspace(path.clone(), &source).await?,
                source,
                updated,
                path,
            }),
            source::Artifact::Binary(binary) => Ok(Artifacts::Binary { binary, source }),
        }
    }

    pub fn add_artifacts(&mut self, artifacts: Artifacts) -> (&LoadedSource, Updated) {
        match artifacts {
            Artifacts::Plugins {
                plugins,
                path,
                source,
                updated,
            } => {
                let names = plugins
                    .iter()
                    .map(|plugin| plugin.config.name().clone())
                    .collect();
                for plugin in plugins {
                    self.plugins.insert(plugin.name().clone(), plugin);
                }
                let source = self.add_loaded_source(source, LoadedSource::Plugins { names, path });
                (source, updated)
            }
            Artifacts::Binary { source, binary } => {
                self.binaries
                    .insert(binary.name.clone(), binary.path.clone());
                let source = self.add_loaded_source(source, LoadedSource::Binary { binary });
                (source, Updated(false))
            }
        }
    }

    fn add_loaded_source(
        &mut self,
        source: Arc<Source>,
        loaded_source: LoadedSource,
    ) -> &LoadedSource {
        // using the entry api so that we can retain an immutable reference to the loaded source
        // Entry::insert_entry would be wonderful here, but it's not stabilized :(
        self.sources.remove(&source);
        self.sources.entry(source).or_insert(loaded_source)
    }

    async fn scan_workspace(
        path: PathBuf,
        source: &Arc<Source>,
    ) -> Result<Vec<LoadedPlugin>, Error> {
        let mut plugins = vec![];
        // avoid recursion by using a queue
        // (recursion with async is iffy - it requires [`Box`]ing futures)
        let mut paths = vec![path];
        while let Some(path) = paths.pop() {
            let config_path = path.join("plugin.ron");
            if !config_path.exists() {
                return Err(Error::PluginFileNotFound(config_path));
            }
            let config = crate::deserialize::<config::Plugin>(config_path).await?;
            match config {
                config::Plugin::Single(config) => plugins.push(LoadedPlugin {
                    source: source.clone(),
                    config,
                    path,
                }),
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
}
