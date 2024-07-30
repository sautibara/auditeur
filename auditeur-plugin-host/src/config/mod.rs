use std::{fmt::Display, path::PathBuf, str::FromStr};

use serde::{de::Visitor, Deserialize, Serialize};

pub mod host;
pub mod plugin;

pub use host::Config as Host;
pub use plugin::Config as Plugin;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PluginName(pub String);

impl Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    // these are the top three git hosts
    Github(GitShorthand),
    Gitlab(GitShorthand),
    Bitbucket(GitShorthand),
    Path(PathBuf),
    Builtin(BinarySource),
}

impl Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github(shorthand) => write!(f, "( github: {shorthand} )"),
            Self::Gitlab(shorthand) => write!(f, "( github: {shorthand} )"),
            Self::Bitbucket(shorthand) => write!(f, "( github: {shorthand} )"),
            Self::Path(path) => write!(f, "( path: {} )", path.display()),
            Self::Builtin(BinarySource { path, name }) => {
                write!(f, "( builtin: ( path: {}, name: {name} ) )", path.display())
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BinarySource {
    pub path: PathBuf,
    pub name: PluginName,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitShorthand {
    pub user: String,
    pub repo: String,
}

impl Display for GitShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { user, repo } = self;
        write!(f, "{user}/{repo}")
    }
}

impl FromStr for GitShorthand {
    type Err = usize;
    fn from_str(s: &str) -> Result<Self, usize> {
        let segments: Vec<_> = s.split('/').collect();
        let [user, repo] = &segments[..] else {
            return Err(segments.len());
        };
        Ok(Self {
            user: user.to_string(),
            repo: repo.to_string(),
        })
    }
}

impl Serialize for GitShorthand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct GitShorthandVisitor;

impl<'de> Visitor<'de> for GitShorthandVisitor {
    type Value = GitShorthand;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            r#"a git shorthand in the form of "{{user}}/{{repo}}""#
        )
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse()
            .map_err(|_| E::invalid_value(serde::de::Unexpected::Str(v), &self))
    }
}

impl<'de> Deserialize<'de> for GitShorthand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(GitShorthandVisitor)
    }
}
