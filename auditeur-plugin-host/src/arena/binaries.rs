use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::prelude::*;

#[derive(Serialize, Deserialize)]
pub struct Binaries {
    path: PathBuf,
    binaries: HashMap<PluginName, Binary>,
}

impl Binaries {
    pub async fn new(path: PathBuf) -> Result<Self, Error> {
        let this = Self {
            path,
            binaries: HashMap::new(),
        };
        this.init().await?;
        Ok(this)
    }

    pub async fn init(&self) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.path).await?;
        Ok(())
    }

    pub async fn remove(&mut self, name: &PluginName) -> Result<(), Error> {
        if let Some(binary) = self.binaries.remove(name) {
            tokio::fs::remove_file(binary.path).await?;
        }
        Ok(())
    }

    pub fn get(&self, name: &PluginName) -> Option<&Binary> {
        self.binaries.get(name)
    }

    pub async fn remove_plugins(
        &mut self,
        plugins: impl Iterator<Item = &PluginName> + Send,
    ) -> Result<(), Error> {
        for plugin_name in plugins {
            self.remove(plugin_name).await?;
        }
        Ok(())
    }

    pub const fn binaries(&self) -> &HashMap<PluginName, Binary> {
        &self.binaries
    }

    pub async fn insert_copies<I>(&mut self, iter: I) -> Result<Vec<Binary>, Error>
    where
        I: IntoIterator<Item = (PluginName, PathBuf)> + Send,
        I::IntoIter: Send,
    {
        let mut libraries = Vec::new();
        for (name, path) in iter {
            let binary = self.insert_copy(&name, &path).await?.to_owned();
            libraries.push(binary);
        }
        Ok(libraries)
    }

    pub async fn insert_copy(&mut self, name: &PluginName, path: &Path) -> Result<&Binary, Error> {
        let Some(file_name) = path.file_name() else {
            return Err(Error::ArtifactHasNoFilename {
                plugin: name.clone(),
                artifact: path.to_owned(),
            });
        };
        let out_path = self.path.join(file_name);
        tokio::fs::copy(path, &out_path).await?;
        self.binaries.insert(
            name.clone(),
            Binary {
                path: out_path,
                name: name.clone(),
            },
        );
        Ok(&self.binaries[name])
    }
}
