use std::{
    borrow::Borrow,
    fmt::{Debug, Display},
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::prelude::*;

pub mod git;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    // these are the top three git hosts
    // TODO: generic git source
    // TODO: choosing commit / version
    Github(git::Shorthand),
    Gitlab(git::Shorthand),
    Bitbucket(git::Shorthand),
    Path(PathBuf),
    Builtin(Binary),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binary {
    pub path: PathBuf,
    pub name: PluginName,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(from = "Variant", into = "Variant")]
pub struct Source {
    variant: Variant,
    id: OwnedId,
}

impl Source {
    pub fn new_git(user: impl Into<String>, repo: impl Into<String>, host: git::Host) -> Self {
        let shorthand = git::Shorthand {
            user: user.into(),
            repo: repo.into(),
        };
        let variant = match host {
            git::Host::Github => Variant::Github(shorthand),
            git::Host::Gitlab => Variant::Gitlab(shorthand),
            git::Host::Bitbucket => Variant::Bitbucket(shorthand),
        };
        variant.into()
    }

    #[must_use]
    pub const fn variant(&self) -> &Variant {
        &self.variant
    }

    #[must_use]
    pub const fn id(&self) -> &OwnedId {
        &self.id
    }

    /// Load this source into the directory `parent_dir`
    ///
    /// # Errors
    ///
    /// - If cloning a git repo fails
    /// - If there is an error with the filesystem
    pub async fn load_into(&self, parent_dir: &Path) -> Result<Artifact, Error> {
        trace!("  loading source '{self}'");
        let artifact = match &self.variant {
            Variant::Github(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Github), parent_dir).await?
            }
            Variant::Gitlab(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Gitlab), parent_dir).await?
            }
            Variant::Bitbucket(shorthand) => {
                Self::clone_git_into(shorthand.with_host(git::Host::Bitbucket), parent_dir).await?
            }
            Variant::Builtin(binary) => Artifact::Binary(binary.clone()),
            Variant::Path(path) => Self::load_path_into(path, parent_dir).await?,
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
        match &self.variant {
            Variant::Github(shorthand) => write!(f, r#"( github: "{shorthand}" )"#),
            Variant::Gitlab(shorthand) => write!(f, r#"( gitlab: "{shorthand}" )"#),
            Variant::Bitbucket(shorthand) => write!(f, r#"( bitbucket: "{shorthand}" )"#),
            Variant::Path(path) => write!(f, r#"( path: "{}" )"#, path.display()),
            Variant::Builtin(Binary { path, name }) => {
                write!(
                    f,
                    r#"( builtin: ( path: "{}", name: {name} ) )"#,
                    path.display()
                )
            }
        }
    }
}

impl From<Variant> for Source {
    fn from(variant: Variant) -> Self {
        let id = match &variant {
            Variant::Github(shorthand) => OwnedId::create(0, shorthand.as_bytes()),
            Variant::Gitlab(shorthand) => OwnedId::create(1, shorthand.as_bytes()),
            Variant::Bitbucket(shorthand) => OwnedId::create(2, shorthand.as_bytes()),
            Variant::Path(path) => {
                let bytes = path.as_os_str().as_encoded_bytes();
                OwnedId::create(3, bytes.iter().copied())
            }
            Variant::Builtin(binary) => {
                let bytes = binary.path.as_os_str().as_encoded_bytes();
                OwnedId::create(4, bytes.iter().copied())
            }
        };
        Self { variant, id }
    }
}

impl From<Source> for Variant {
    fn from(value: Source) -> Self {
        value.variant
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "OwnedIdBase64", try_from = "OwnedIdBase64")]
#[repr(transparent)]
pub struct OwnedId(Arc<[u8]>);

impl OwnedId {
    fn new(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }

    fn create(discriminant: u8, bytes: impl IntoIterator<Item = u8>) -> Self {
        let mut vec = vec![discriminant];
        vec.extend(bytes);
        Self::new(vec)
    }
}

impl Debug for OwnedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", &**self)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct OwnedIdBase64(String);

impl From<&Id> for OwnedIdBase64 {
    fn from(value: &Id) -> Self {
        Self(BASE64_URL_SAFE.encode(&value.0))
    }
}

impl From<OwnedId> for OwnedIdBase64 {
    fn from(value: OwnedId) -> Self {
        Self::from(&*value)
    }
}

impl TryFrom<OwnedIdBase64> for OwnedId {
    type Error = base64::DecodeError;

    fn try_from(value: OwnedIdBase64) -> std::result::Result<Self, Self::Error> {
        let bytes = BASE64_URL_SAFE.decode(value.0)?;
        let Some(first) = bytes.first() else {
            return Err(base64::DecodeError::InvalidLength(0));
        };
        if *first > 4 {
            return Err(base64::DecodeError::InvalidByte(0, *first));
        }
        Ok(Self(Arc::from(bytes)))
    }
}

#[repr(transparent)]
pub struct Id([u8]);

impl Deref for OwnedId {
    type Target = Id;

    fn deref(&self) -> &Self::Target {
        // SAFETY: OwnedId is #[repr(transparent)] as Arc<[u8]>
        let bytes: &Arc<[u8]> = unsafe { &*std::ptr::from_ref::<Self>(self).cast() };
        // coerce the &Arc<[u8]> into a &[u8]
        let byte_slice: &[u8] = bytes;
        // SAFETY: Id is #[repr(transparent)] as [u8]
        let shared_id: &Id = unsafe { &*(std::ptr::from_ref::<[u8]>(byte_slice) as *const Id) };

        shared_id
    }
}

impl Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let variant = match self.0[0] {
            0 => "Github",
            1 => "Gitlab",
            2 => "Bitbucket",
            3 => "Path",
            4 => "Builtin",
            _ => unreachable!("These are the only variants possible now."),
        };
        write!(
            f,
            "source::Id::{variant}({})",
            String::from_utf8_lossy(&self.0[1..])
        )
    }
}

impl Borrow<Id> for OwnedId {
    fn borrow(&self) -> &Id {
        self
    }
}

impl Borrow<Id> for Source {
    fn borrow(&self) -> &Id {
        &self.id
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
