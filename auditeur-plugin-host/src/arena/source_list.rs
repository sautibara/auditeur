use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{prelude::*, source::Version};

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LoadedSource {
    Plugins {
        path: PathBuf,
        names: Vec<PluginName>,
        version: Version,
    },
    Binary {
        binary: Binary,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadedPlugin {
    pub source: SourceId,
    pub artifacts: LoadedPluginArtifacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LoadedPluginArtifacts {
    SourceCode {
        path: PathBuf,
        config: config::plugin::Plugin,
    },
    Binary {
        binary: Binary,
    },
}

impl LoadedPlugin {
    pub const fn name(&self) -> &PluginName {
        match &self.artifacts {
            LoadedPluginArtifacts::SourceCode { config, .. } => config.name(),
            LoadedPluginArtifacts::Binary { binary } => &binary.name,
        }
    }

    pub const fn id(&self) -> &SourceId {
        &self.source
    }
}

#[derive(Serialize, Deserialize)]
pub struct SourceList {
    path: PathBuf,
    sources: HashMap<SourceId, LoadedSource>,
    plugins: HashMap<PluginName, LoadedPlugin>,
}

impl SourceList {
    pub async fn new(path: impl Into<PathBuf> + Send) -> Result<Self, Error> {
        let this = Self {
            path: path.into(),
            sources: HashMap::new(),
            plugins: HashMap::new(),
        };
        this.init().await?;
        Ok(this)
    }

    pub async fn init(&self) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.path)
            .await
            .map_err(Into::into)
    }

    #[allow(dead_code)] // cope
    pub fn get(&self, id: &SourceId) -> Option<&LoadedSource> {
        self.sources.get(id)
    }

    pub fn version(&self, id: &SourceId) -> Option<Version> {
        self.get(id).map(|source| match source {
            LoadedSource::Plugins { version, .. } => version.clone(),
            LoadedSource::Binary { binary } => Version::Versioned {
                version: binary.version.clone(),
            },
        })
    }

    pub fn get_plugin(&self, plugin: &PluginName) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin)
    }

    pub fn plugins(&self) -> impl Iterator<Item = (&PluginName, &LoadedPlugin)> {
        self.plugins.iter()
    }

    pub fn sources(&self) -> impl Iterator<Item = (&SourceId, &LoadedSource)> {
        self.sources.iter()
    }

    pub fn contains_source(&self, source: &SourceId) -> bool {
        self.sources.contains_key(source)
    }

    pub async fn remove_source(
        &mut self,
        source: &SourceId,
    ) -> Result<Option<LoadedSource>, Error> {
        let source = self.sources.remove(source);
        if let Some(LoadedSource::Plugins { path, names, .. }) = &source {
            trace!("removing directory '{}'", path.display());
            tokio::fs::remove_dir_all(path).await?;

            for name in names {
                self.plugins.remove(name);
            }
        }
        Ok(source)
    }

    pub async fn add_source(
        &mut self,
        source: &Source,
        version: Option<&Version>,
        auto_update: bool,
    ) -> Result<(&LoadedSource, Updated), Error> {
        let artifacts = self.load_source(source, version, auto_update).await?;
        let updated = self.add_artifacts(artifacts);
        Ok(updated)
    }

    pub async fn load_source(
        &self,
        source: &Source,
        version: Option<&Version>,
        auto_update: bool,
    ) -> Result<Artifacts, Error> {
        match source.load_into(&self.path, version, auto_update).await? {
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
                let names = plugins.iter().map(|plugin| plugin.name().clone()).collect();
                for plugin in plugins {
                    self.plugins.insert(plugin.name().clone(), plugin);
                }
                let version = match &updated {
                    Updated::True { new_version } => new_version.clone(),
                    Updated::False { retained_version } => retained_version.clone(),
                };
                let source = self.add_loaded_source(
                    &source,
                    LoadedSource::Plugins {
                        names,
                        path,
                        version,
                    },
                );
                (source, updated)
            }
            Artifacts::Binary { source, binary } => {
                let version = binary.version.clone();
                self.plugins.insert(
                    binary.name.clone(),
                    LoadedPlugin {
                        source: source.clone(),
                        artifacts: LoadedPluginArtifacts::Binary {
                            binary: binary.clone(),
                        },
                    },
                );
                let loaded_source =
                    self.add_loaded_source(&source, LoadedSource::Binary { binary });
                (
                    loaded_source,
                    Updated::False {
                        retained_version: Version::Versioned { version },
                    },
                )
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
                        artifacts: LoadedPluginArtifacts::SourceCode {
                            path: offset,
                            config,
                        },
                    });
                }
            } else {
                break;
            }
        }
        Ok(plugins)
    }
}
