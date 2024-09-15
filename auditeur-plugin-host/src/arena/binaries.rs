use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::prelude::*;

use super::source_list::{LoadedSource, SourceList};

#[derive(Serialize, Deserialize)]
pub struct Binaries {
    path: PathBuf,
    binaries: HashMap<PluginName, PathBuf>,
}

impl Binaries {
    const FILE_NAME: &'static str = "binaries.ron";

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            binaries: HashMap::new(),
        }
    }

    pub async fn try_deserialize_from(path: PathBuf) -> Self {
        if let Ok(binaries) = crate::deserialize(path.join(Self::FILE_NAME)).await {
            Self { path, binaries }
        } else {
            Self::new(path)
        }
    }

    // Self::serialize has a name collision :(
    pub async fn serialize_to_file(&self) -> Result<(), Error> {
        crate::serialize(self.path.join(Self::FILE_NAME), self.binaries.clone()).await
    }

    async fn remove(&mut self, name: &PluginName) -> Result<(), Error> {
        if let Some(path) = self.binaries.remove(name) {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    pub async fn remove_source(&mut self, source: &Source, list: &SourceList) -> Result<(), Error> {
        if let Some(LoadedSource::Plugins { names, .. }) = list.get_source(source) {
            for plugin_name in names {
                self.remove(plugin_name).await?;
            }
        }
        self.serialize_to_file().await?;
        Ok(())
    }

    pub fn unbuilt_plugins<'a>(
        &'a self,
        list: &'a SourceList,
    ) -> impl Iterator<Item = &'a PluginName> {
        list.plugins()
            .keys()
            .filter(|&key| !self.binaries.contains_key(key))
    }

    pub const fn binaries(&self) -> &HashMap<PluginName, PathBuf> {
        &self.binaries
    }

    pub async fn insert_copy(&mut self, name: PluginName, path: PathBuf) -> Result<(), Error> {
        let Some(file_name) = path.file_name() else {
            return Err(Error::ArtifactHasNoFilename {
                plugin: name,
                artifact: path,
            });
        };
        let out_path = self.path.join(file_name);
        tokio::fs::copy(path, &out_path).await?;
        self.binaries.insert(name, out_path);
        self.serialize_to_file().await?;
        Ok(())
    }
}
