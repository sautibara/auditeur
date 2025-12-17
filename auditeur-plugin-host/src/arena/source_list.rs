use std::{collections::HashMap, path::PathBuf};

use crate::prelude::*;

#[derive(Clone, Debug)]
pub enum Artifacts {
    Plugins {
        source: SourceId,
        path: PathBuf,
        plugins: Vec<LoadedPlugin>,
        updated: Updated,
    },
    Binary {
        source: SourceId,
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

impl LoadedSource {
    pub const fn as_binary(&self) -> Option<&source::Binary> {
        if let Self::Binary { binary } = self {
            Some(binary)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadedPlugin {
    pub path: PathBuf,
    pub source: SourceId,
    pub config: config::plugin::Plugin,
}

impl LoadedPlugin {
    pub const fn name(&self) -> &PluginName {
        self.config.name()
    }

    pub const fn id(&self) -> &SourceId {
        &self.source
    }
}

pub struct SourceList {
    path: PathBuf,
    sources: HashMap<SourceId, LoadedSource>,
    plugins: HashMap<PluginName, LoadedPlugin>,
}

impl SourceList {
    pub async fn new(path: impl Into<PathBuf> + Send) -> Result<Self, Error> {
        let path = path.into();
        tokio::fs::create_dir_all(&path).await?;
        Ok(Self {
            path,
            sources: HashMap::new(),
            plugins: HashMap::new(),
        })
    }

    pub fn get_plugin(&self, plugin: &PluginName) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin)
    }

    pub fn binaries(&self) -> impl Iterator<Item = (&SourceId, &Binary)> + '_ {
        self.sources
            .iter()
            .filter_map(|(id, source)| source.as_binary().map(|binary| (id, binary)))
    }

    pub async fn remove_source(
        &mut self,
        source: &SourceId,
    ) -> Result<Option<LoadedSource>, Error> {
        let source = self.sources.remove(source);
        if let Some(LoadedSource::Plugins { path, .. }) = &source {
            tokio::fs::remove_dir(path).await?;
        }
        Ok(source)
    }

    pub async fn add_source(&mut self, source: &Source) -> Result<(&LoadedSource, Updated), Error> {
        let artifacts = self.load_source(source).await?;
        let updated = self.add_artifacts(artifacts);
        Ok(updated)
    }

    pub async fn load_source(&self, source: &Source) -> Result<Artifacts, Error> {
        match source.load_into(&self.path).await? {
            source::Artifact::Path { path, updated } => Ok(Artifacts::Plugins {
                plugins: Self::scan_workspace(path.clone(), source).await?,
                source: source.id().clone(),
                updated,
                path,
            }),
            source::Artifact::Binary(binary) => Ok(Artifacts::Binary {
                source: source.id().clone(),
                binary,
            }),
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
                let source = self.add_loaded_source(&source, LoadedSource::Plugins { names, path });
                (source, updated)
            }
            Artifacts::Binary { source, binary } => {
                let source = self.add_loaded_source(&source, LoadedSource::Binary { binary });
                (source, Updated(false))
            }
        }
    }

    fn add_loaded_source(
        &mut self,
        source: &SourceId,
        loaded_source: LoadedSource,
    ) -> &LoadedSource {
        // using the entry api so that we can retain an immutable reference to the loaded source
        // Entry::insert_entry would be wonderful here, but it's not stabilized :(
        self.sources.remove(source);
        self.sources.entry(source.clone()).or_insert(loaded_source)
    }

    async fn scan_workspace(path: PathBuf, source: &Source) -> Result<Vec<LoadedPlugin>, Error> {
        let mut plugins = vec![];
        // avoid recursion by using a queue
        // (recursion with async is iffy - it requires [`Box`]ing futures)
        let mut paths = vec![path];
        let mut configs = vec![];
        loop {
            if let Some(path) = paths.pop() {
                let config_path = path.join("plugin.ron");
                if !config_path.exists() {
                    return Err(Error::PluginFileNotFound(config_path));
                }
                let mut config = crate::deserialize::<config::Plugin>(config_path).await?;
                config.offset = path.join(config.offset);
                configs.push(config);
            } else if let Some(config) = configs.pop() {
                println!("{config:?}");
                let config::Plugin {
                    offset,
                    children,
                    plugin,
                } = config;
                for child in children {
                    match child {
                        config::plugin::Child::Path(path) => {
                            paths.push(offset.join(path));
                        }
                        config::plugin::Child::Inline(mut config) => {
                            config.offset = offset.join(config.offset);
                            configs.push(config);
                        }
                    }
                }
                if let Some(config) = plugin {
                    plugins.push(LoadedPlugin {
                        source: source.id().clone(),
                        config,
                        path: offset,
                    });
                }
            } else {
                break;
            }
        }
        Ok(plugins)
    }
}
