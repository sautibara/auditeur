use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::prelude::*;

pub struct Binaries {
    path: PathBuf,
    binaries: HashMap<PluginName, Binary>,
}

impl Binaries {
    const FILE_NAME: &'static str = "binaries.ron";

    pub async fn new(path: PathBuf) -> Result<Self, Error> {
        tokio::fs::create_dir_all(&path).await?;
        Ok(Self {
            path,
            binaries: HashMap::new(),
        })
    }

    pub async fn try_deserialize_from(path: PathBuf) -> Result<Self, Error> {
        match crate::deserialize(path.join(Self::FILE_NAME)).await {
            Ok(binaries) => {
                tokio::fs::create_dir_all(&path).await?;
                Ok(Self { path, binaries })
            }
            // TODO: report errors
            Err(_) => Self::new(path).await,
        }
    }

    pub async fn serialize(&self) -> Result<(), Error> {
        crate::serialize(self.path.join(Self::FILE_NAME), self.binaries.clone()).await
    }

    async fn remove(&mut self, name: &PluginName) -> Result<(), Error> {
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
        self.serialize().await?;
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
        self.serialize().await?;
        Ok(&self.binaries[name])
    }
}
