use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use log::trace;
use serde::{Deserialize, Serialize};

use crate::prelude::*;

pub mod git;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binary {
    pub path: PathBuf,
    pub name: PluginName,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    // these are the top three git hosts
    // TODO: generic git source
    // TODO: choosing commit / version
    Github(git::Shorthand),
    Gitlab(git::Shorthand),
    Bitbucket(git::Shorthand),
    Path(PathBuf),
    Builtin(Binary),
}

impl Source {
    pub fn new_git(user: impl Into<String>, repo: impl Into<String>, host: git::Host) -> Self {
        let shorthand = git::Shorthand {
            user: user.into(),
            repo: repo.into(),
        };
        match host {
            git::Host::Github => Self::Github(shorthand),
            git::Host::Gitlab => Self::Gitlab(shorthand),
            git::Host::Bitbucket => Self::Bitbucket(shorthand),
        }
    }

    /// Load this source into the directory `parent_dir`
    ///
    /// # Errors
    ///
    /// - If cloning a git repo fails
    /// - If there is an error with the filesystem
    pub async fn load_into(&self, parent_dir: &Path) -> Result<Artifact, Error> {
        trace!("  loading source '{self}'");
        let artifact = match self {
            Self::Github(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Github), parent_dir).await?
            }
            Self::Gitlab(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Gitlab), parent_dir).await?
            }
            Self::Bitbucket(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Bitbucket), parent_dir).await?
            }
            Self::Builtin(binary) => Artifact::Binary(binary.clone()),
            Self::Path(path) => Self::load_path_into(path, parent_dir).await?,
        };
        Ok(artifact)
    }

    async fn load_path_into(source: &Path, parent_dir: &Path) -> Result<Artifact, Error> {
        if !source.exists() || !source.is_dir() {
            return Err(Error::PathNotFound(source.to_owned()));
        }
        let subdirectory = source
            .file_name()
            .ok_or_else(|| Error::NoComponentsInPath(source.to_owned()))?;
        let path = parent_dir.join(subdirectory);
        let updated = if crate::path_exists_with_children(&path)? {
            // TODO: check for updates
            Updated(false)
        } else {
            trace!("copying '{}' to '{}'", source.display(), path.display());
            tokio::fs::copy(source, &path).await?;
            Updated(true)
        };
        Ok(Artifact::Path { path, updated })
    }

    async fn clone_git_into(
        git_shorthand: git::HostedShorthandRef<'_>,
        parent_dir: &Path,
    ) -> Result<Artifact, Error> {
        let path = parent_dir.join(git_shorthand.repo());
        let updated = if crate::path_exists_with_children(&path)? {
            // TODO: check for updates
            false
        } else {
            // this is a dumb amount of cloning required due to the 'static bound on spawn_blocking
            // - at times like these I wish I was using some type of structured concurrency
            let repo = git_shorthand.to_owned();
            let path = path.clone();
            trace!("cloning '{repo}' to '{}''", path.display());
            crate::spawn_blocking(move || repo.clone_into(&path)).await?;
            true
        };
        Ok(Artifact::Path {
            path,
            updated: Updated(updated),
        })
    }
}

impl Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github(shorthand) => write!(f, r#"( github: "{shorthand}" )"#),
            Self::Gitlab(shorthand) => write!(f, r#"( gitlab: "{shorthand}" )"#),
            Self::Bitbucket(shorthand) => write!(f, r#"( bitbucket: "{shorthand}" )"#),
            Self::Path(path) => write!(f, r#"( path: "{}" )"#, path.display()),
            Self::Builtin(Binary { path, name }) => {
                write!(
                    f,
                    r#"( builtin: ( path: "{}", name: {name} ) )"#,
                    path.display()
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Updated(pub bool);

impl Updated {
    #[must_use]
    pub const fn has_updated(&self) -> bool {
        let Self(updated) = self;
        *updated
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Artifact {
    Path { path: PathBuf, updated: Updated },
    Binary(Binary),
}
