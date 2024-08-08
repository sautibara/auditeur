use std::{borrow::Borrow, fmt::Display, path::PathBuf, str::FromStr, sync::OnceLock};

use regex::Regex;
use serde::{de::Visitor, Deserialize, Serialize};

pub mod host;
pub mod plugin;

pub use host::Config as Host;
pub use plugin::Config as Plugin;
use thiserror::Error;

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
            Self::Gitlab(shorthand) => write!(f, "( gitlab: {shorthand} )"),
            Self::Bitbucket(shorthand) => write!(f, "( bitbucket: {shorthand} )"),
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
    #[must_use]
    pub fn fetch_url(&self, host: GitHost, method: GitFetchMethod) -> String {
        self.with_host(host).fetch_url(method)
    }

    #[must_use]
    pub const fn with_host(&self, host: GitHost) -> HostedGitShorthandRef {
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
            user: (*user).to_string(),
            repo: (*repo).to_string(),
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

/// A common hosting service for git repositories
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitHost {
    Github,
    Gitlab,
    Bitbucket,
}

impl GitHost {
    /// Returns the base url for the hosting service
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur_plugin_host::config::GitHost;
    /// assert_eq!(GitHost::Github.url(), "github.com");
    /// assert_eq!(GitHost::Bitbucket.url(), "bitbucket.org");
    /// ```
    #[must_use]
    pub const fn url(&self) -> &str {
        match self {
            Self::Github => "github.com",
            Self::Gitlab => "gitlab.com",
            Self::Bitbucket => "bitbucket.org",
        }
    }

    /// Returns the name of the hosting service
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur_plugin_host::config::GitHost;
    /// assert_eq!(GitHost::Github.name(), "github");
    /// assert_eq!(GitHost::Gitlab.name(), "gitlab");
    /// ```
    #[must_use]
    pub const fn name(&self) -> &str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Bitbucket => "bitbucket",
        }
    }

    /// Returns `true` if the git host is [`Github`].
    ///
    /// [`Github`]: GitHost::Github
    #[must_use]
    pub const fn is_github(&self) -> bool {
        matches!(self, Self::Github)
    }

    /// Returns `true` if the git host is [`Gitlab`].
    ///
    /// [`Gitlab`]: GitHost::Gitlab
    #[must_use]
    pub const fn is_gitlab(&self) -> bool {
        matches!(self, Self::Gitlab)
    }

    /// Returns `true` if the git host is [`Bitbucket`].
    ///
    /// [`Bitbucket`]: GitHost::Bitbucket
    #[must_use]
    pub const fn is_bitbucket(&self) -> bool {
        matches!(self, Self::Bitbucket)
    }
}

impl Display for GitHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.url())
    }
}

/// A git host was not recognized so it couldn't be [parsed](GitHost::from_str)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Error)]
#[error("unknown git host: {0}")]
pub struct GitUnknownHost(String);

impl FromStr for GitHost {
    type Err = GitUnknownHost;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github.com" => Ok(Self::Github),
            "gitlab.com" => Ok(Self::Gitlab),
            "bitbucket.org" => Ok(Self::Bitbucket),
            unknown => Err(GitUnknownHost(unknown.to_owned())),
        }
    }
}

/// A network protocol that can be used to fetch a git repository from a hosting service
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GitFetchMethod {
    Https,
    Ssh,
}

impl GitFetchMethod {
    /// Returns `true` if the git fetch method is [`Https`].
    ///
    /// [`Https`]: GitFetchMethod::Https
    #[must_use]
    pub const fn is_https(&self) -> bool {
        matches!(self, Self::Https)
    }

    /// Returns `true` if the git fetch method is [`Ssh`].
    ///
    /// [`Ssh`]: GitFetchMethod::Ssh
    #[must_use]
    pub const fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh)
    }

    #[must_use]
    pub fn fetch_url(&self, host: GitHost, user: &str, repo: &str) -> String {
        match self {
            Self::Https => format!("https://{host}/{user}/{repo}.git"),
            Self::Ssh => format!("git@{host}:{user}/{repo}.git"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostedGitShorthandRef<'a> {
    pub host: GitHost,
    pub shorthand: &'a GitShorthand,
}

impl<'a> HostedGitShorthandRef<'a> {
    #[must_use]
    pub fn fetch_url(&self, method: GitFetchMethod) -> String {
        method.fetch_url(self.host, &self.shorthand.user, &self.shorthand.repo)
    }

    #[must_use]
    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    #[must_use]
    pub fn to_owned(self) -> HostedGitShorthand {
        HostedGitShorthand {
            host: self.host,
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
    pub host: GitHost,
    pub shorthand: GitShorthand,
}

impl HostedGitShorthand {
    #[must_use]
    pub fn fetch_url(&self, method: GitFetchMethod) -> String {
        self.to_ref().fetch_url(method)
    }

    #[must_use]
    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    #[must_use]
    pub const fn to_ref(&self) -> HostedGitShorthandRef {
        HostedGitShorthandRef {
            host: self.host,
            shorthand: &self.shorthand,
        }
    }
}

impl Display for HostedGitShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_ref().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitFetchUrl {
    pub method: GitFetchMethod,
    pub shorthand: HostedGitShorthand,
}

impl GitFetchUrl {
    #[must_use]
    pub fn fetch_url(&self) -> String {
        self.shorthand.fetch_url(self.method)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Error)]
pub enum GitUrlParseError {
    #[error("{0}")]
    UnknownHost(#[from] GitUnknownHost),
    #[error("could not parse git url: {0}")]
    Unknown(String),
}

impl FromStr for GitFetchUrl {
    type Err = GitUrlParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        static SSH_REGEX: OnceLock<Regex> = OnceLock::new();
        static HTTPS_REGEX: OnceLock<Regex> = OnceLock::new();
        let ssh_regex = SSH_REGEX.get_or_init(|| {
            Regex::new(r"^git@(?<host>[\w\.]+):(?<user>[\w]+)/(?<repo>[\w]+)(?:.git)?$")
                .expect("regex should compile")
        });
        let https_regex = HTTPS_REGEX.get_or_init(|| {
            Regex::new(r"^https://(?<host>[\w\.]+)/(?<user>[\w]+)/(?<repo>[\w]+)(?:.git)?$")
                .expect("regex should compile")
        });

        // test https first because it'll probably be more common
        let (method, captures) = if let Some(captures) = https_regex.captures(s) {
            (GitFetchMethod::Https, captures)
        } else if let Some(captures) = ssh_regex.captures(s) {
            (GitFetchMethod::Ssh, captures)
        } else {
            return Err(GitUrlParseError::Unknown(s.to_owned()));
        };

        Ok(Self {
            method,
            // both regex have the same capture groups for the shorthand
            shorthand: HostedGitShorthand {
                host: captures["host"].parse()?,
                shorthand: GitShorthand {
                    user: captures["user"].to_owned(),
                    repo: captures["repo"].to_owned(),
                },
            },
        })
    }
}
