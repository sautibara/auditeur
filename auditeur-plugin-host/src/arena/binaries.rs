use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::prelude::*;

#[derive(Serialize, Deserialize)]
pub struct Binaries {
    path: PathBuf,
    binaries: HashMap<PluginName, PathBuf>,
}

impl Binaries {
    pub async fn new(path: PathBuf) -> Result<Self, Error> {
        let mut this = Self {
            path,
            binaries: HashMap::new(),
        };
        this.init().await?;
        Ok(this)
    }

    pub async fn init(&mut self) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.path).await?;

        // this mimics [`HashSet::retain`], but we can't use it because of the `await`
        let mut binaries_to_remove = HashSet::new();
        for (plugin, path) in &self.binaries {
            if !tokio::fs::try_exists(path).await.is_ok_and(|exists| exists) {
                binaries_to_remove.insert(plugin.clone());
            }
        }

        for plugin in binaries_to_remove {
            if let Some(path) = self.binaries.remove(&plugin) {
                warn!(
                    "lockfile referenced binary for plugin '{plugin}' at '{}', but no file was found there - removing reference",
                    path.display()
                );
            }
        }

        Ok(())
    }

    pub async fn remove(&mut self, name: &PluginName) -> Result<(), Error> {
        if let Some(path) = self.binaries.remove(name) {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    pub async fn clear(&mut self) -> Result<(), Error> {
        for (_, path) in self.binaries.drain() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }

    pub fn get(&self, name: &PluginName) -> Option<&Path> {
        self.binaries.get(name).map(|v| &**v)
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

    pub const fn binaries(&self) -> &HashMap<PluginName, PathBuf> {
        &self.binaries
    }

    pub async fn insert_copies<I>(&mut self, iter: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = (PluginName, PathBuf)> + Send,
        I::IntoIter: Send,
    {
        for (name, path) in iter {
            self.insert_copy(&name, &path).await?;
        }
        Ok(())
    }

    pub async fn insert_copy(&mut self, name: &PluginName, path: &Path) -> Result<(), Error> {
        let Some(file_name) = path.file_stem() else {
            return Err(Error::ArtifactHasNoFilename {
                plugin: name.clone(),
                artifact: path.to_owned(),
            });
        };

        let random_number: u16 = rand::random();

        let mut file_name = file_name.to_os_string();
        file_name.push("-");
        file_name.push(random_number.to_string());
        if let Some(extension) = path.extension() {
            file_name.push(".");
            file_name.push(extension);
        }

        let out_path = self.path.join(file_name);
        tokio::fs::copy(path, &out_path).await?;
        self.binaries.insert(name.clone(), out_path);
        Ok(())
    }
}
