use std::{
    io::Write as _,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use cargo_toml::{DepsSet, Product};
use futures::Future;
use tokio::io::AsyncWriteExt as _;

use crate::prelude::*;

pub struct Manifest {
    inner: cargo_toml::Manifest,
    path: PathBuf,
    modified: bool,
}

impl Manifest {
    fn load_manifest_blocking(path: &Path) -> Result<cargo_toml::Manifest, Error> {
        cargo_toml::Manifest::from_path(path).map_err(|error| Error::CargoParse {
            error,
            path: path.to_owned(),
        })
    }

    pub fn load_dir(path: impl AsRef<Path>) -> impl Future<Output = Result<Self, Error>> {
        Self::load(path.as_ref().join("Cargo.toml"))
    }

    pub async fn load(path: PathBuf) -> Result<Self, Error> {
        let (manifest, path) = crate::spawn_blocking(move || {
            let manifest = Self::load_manifest_blocking(&path);
            (manifest, path)
        })
        .await;
        Ok(Self {
            inner: manifest?,
            path,
            modified: false,
        })
    }

    pub fn save_blocking(&self) -> Result<(), Error> {
        let serialized =
            toml::ser::to_string_pretty(&self.inner).map_err(|error| Error::TomlSerialization {
                error,
                path: self.path.clone(),
            })?;
        std::fs::File::create(&self.path)?.write_all(serialized.as_bytes())?;
        Ok(())
    }

    pub async fn save(&self) -> Result<(), Error> {
        let manifest = self.inner.clone();
        let serialized = crate::spawn_blocking(move || toml::ser::to_string_pretty(&manifest))
            .await
            .map_err(|error| Error::TomlSerialization {
                error,
                path: self.path.clone(),
            })?;
        tokio::fs::File::create(&self.path)
            .await?
            .write_all(serialized.as_bytes())
            .await?;
        Ok(())
    }

    // TODO: try to make this a relative path rather than an absolute one
    // also, there's a chance that the path is an absolute one, and that must be handled too
    fn move_path(path: &mut String, dest: &Path) {
        *path = dest.join(&path).to_string_lossy().into_owned();
    }

    fn move_products(products: &mut [Product], dest: &Path) {
        for path in products.iter_mut().filter_map(|test| test.path.as_mut()) {
            Self::move_path(path, dest);
        }
    }

    fn move_deps(deps: &mut DepsSet, dest: &Path) {
        for dependency in deps.values_mut() {
            if let Some(path) = dependency.detail_mut().path.as_mut() {
                Self::move_path(path, dest);
            }
        }
    }

    pub fn copy_to_path(mut self, dest: impl AsRef<Path>) -> Self {
        let dest = dest.as_ref();
        // fix all the things!
        // Since we're moving the manifest, we have to move each and every path with it too
        if let Some(path) = self.inner.lib.as_mut().and_then(|lib| lib.path.as_mut()) {
            Self::move_path(path, dest);
        }
        Self::move_products(&mut self.test, dest);
        Self::move_products(&mut self.bin, dest);
        Self::move_products(&mut self.example, dest);
        // tests seem important enough to migrate
        // dependencies definitely do, though
        Self::move_deps(&mut self.inner.dependencies, dest);
        Self::move_deps(&mut self.inner.dev_dependencies, dest);
        Self::move_deps(&mut self.inner.build_dependencies, dest);
        for target in self.inner.target.values_mut() {
            Self::move_deps(&mut target.dependencies, dest);
            Self::move_deps(&mut target.dev_dependencies, dest);
            Self::move_deps(&mut target.build_dependencies, dest);
        }
        for patch in self.inner.patch.values_mut() {
            Self::move_deps(patch, dest);
        }
        // then we can just move the path - the actual file will be saved later
        self.path = dest.join("Cargo.toml");
        // and oh yes this is modified
        self.modified = true;
        self
    }
}

impl Deref for Manifest {
    type Target = cargo_toml::Manifest;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Manifest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.modified = true;
        &mut self.inner
    }
}

impl Drop for Manifest {
    fn drop(&mut self) {
        if !self.modified {
            return;
        }
        let _ = self.save_blocking();
    }
}
