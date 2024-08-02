use std::{borrow::Borrow, fmt::Display, path::PathBuf, str::FromStr};

use serde::{de::Visitor, Deserialize, Serialize};

pub mod host;
pub mod plugin;

pub use host::Config as Host;
pub use plugin::Config as Plugin;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PluginName(pub String);

impl Borrow<String> for PluginName {
    fn borrow(&self) -> &String {
        &self.0
    }
}

impl Borrow<str> for PluginName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    // these are the top three git hosts
    // TODO: generic git source
    // TODO: choosing commit / version
    Github(GitShorthand),
    Gitlab(GitShorthand),
    Bitbucket(GitShorthand),
    Path(PathBuf),
    Builtin(BinarySource),
}

impl Source {
    pub fn new_github(user: impl Into<String>, repo: impl Into<String>) -> Self {
        Self::Github(GitShorthand {
            user: user.into(),
            repo: repo.into(),
        })
    }
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

impl GitShorthand {
    pub fn as_https(&self, host: &str) -> String {
        self.with_host(host).as_https()
    }

    pub fn as_ssh(&self, host: &str) -> String {
        self.with_host(host).as_ssh()
    }

    pub fn with_host<'a>(&'a self, host: &'a str) -> HostedGitShorthandRef {
        HostedGitShorthandRef {
            host,
            shorthand: self,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostedGitShorthandRef<'a> {
    pub host: &'a str,
    pub shorthand: &'a GitShorthand,
}

impl<'a> HostedGitShorthandRef<'a> {
    pub fn as_https(&self) -> String {
        let Self {
            shorthand: GitShorthand { user, repo },
            host,
        } = self;
        format!("https://{host}/{user}/{repo}.git")
    }

    pub fn as_ssh(&self) -> String {
        let Self {
            shorthand: GitShorthand { user, repo },
            host,
        } = self;
        format!("git@{host}:{user}/{repo}")
    }

    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    pub fn to_owned(self) -> HostedGitShorthand {
        HostedGitShorthand {
            host: self.host.to_owned(),
            shorthand: self.shorthand.to_owned(),
        }
    }
}

impl Display for HostedGitShorthandRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { host, shorthand } = self;
        write!(f, "{host} - {shorthand}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostedGitShorthand {
    pub host: String,
    pub shorthand: GitShorthand,
}

impl HostedGitShorthand {
    pub fn as_https(&self) -> String {
        self.to_ref().as_https()
    }

    pub fn as_ssh(&self) -> String {
        self.to_ref().as_ssh()
    }

    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    pub fn to_ref(&self) -> HostedGitShorthandRef {
        HostedGitShorthandRef {
            host: &self.host,
            shorthand: &self.shorthand,
        }
    }
}

impl Display for HostedGitShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_ref().fmt(f)
    }
}
