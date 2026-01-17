use std::{
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
    sync::OnceLock,
};

use git2::{CredentialType, Repository};
use regex::Regex;
use serde::{Deserialize, Serialize, de::Visitor};
use sha1_smol::Sha1;

use crate::Error;

fn update_submodules_of(repo: &Repository, host: Host) -> Result<(), git2::Error> {
    fn submodule_update_options<'cb>(host: Host, url: &str) -> git2::SubmoduleUpdateOptions<'cb> {
        let mut opts = git2::SubmoduleUpdateOptions::new();
        let host = match url.parse::<FetchUrl>() {
            Ok(url) => url.shorthand.host,
            Err(_) => host,
        };
        let (fetch_options, _) = host.fetch_options();
        opts.fetch(fetch_options);
        opts
    }

    fn add_subrepos(
        host: Host,
        repo: &Repository,
        list: &mut Vec<Repository>,
    ) -> Result<(), git2::Error> {
        for mut subm in repo.submodules()? {
            // use the url of the submodule so we can input an ssh key if necessary
            let mut opts = submodule_update_options(host, subm.url().unwrap_or_default());
            subm.update(true, Some(&mut opts))?;
            list.push(subm.open()?);
        }
        Ok(())
    }

    let mut repos = Vec::new();
    add_subrepos(host, repo, &mut repos)?;
    while let Some(subrepo) = repos.pop() {
        add_subrepos(host, &subrepo, &mut repos)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shorthand {
    pub user: String,
    pub repo: String,
}

impl Shorthand {
    #[must_use]
    pub fn fetch_url(&self, host: Host, method: FetchMethod) -> String {
        self.with_host(host).fetch_url(method)
    }

    #[must_use]
    pub const fn with_host(&self, host: Host) -> HostedShorthandRef<'_> {
        HostedShorthandRef {
            host,
            shorthand: self,
        }
    }

    pub(crate) fn update_sha1(&self, sha1: &mut Sha1) {
        sha1.update(self.user.as_bytes());
        sha1.update(&[0]);
        sha1.update(self.repo.as_bytes());
    }
}

impl Display for Shorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { user, repo } = self;
        write!(f, "{user}/{repo}")
    }
}

impl FromStr for Shorthand {
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

impl Serialize for Shorthand {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct ShorthandVisitor;

impl Visitor<'_> for ShorthandVisitor {
    type Value = Shorthand;

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

impl<'de> Deserialize<'de> for Shorthand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(ShorthandVisitor)
    }
}

/// A common hosting service for git repositories
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Host {
    Github,
    Gitlab,
    Bitbucket,
}

impl Host {
    /// Returns the base url for the hosting service
    ///
    /// # Examples
    ///
    /// ```
    /// use auditeur_plugin_host::source::git::Host;
    /// assert_eq!(Host::Github.url(), "github.com");
    /// assert_eq!(Host::Bitbucket.url(), "bitbucket.org");
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
    /// use auditeur_plugin_host::source::git::Host;
    /// assert_eq!(Host::Github.name(), "github");
    /// assert_eq!(Host::Gitlab.name(), "gitlab");
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
    /// [`Github`]: Host::Github
    #[must_use]
    pub const fn is_github(&self) -> bool {
        matches!(self, Self::Github)
    }

    /// Returns `true` if the git host is [`Gitlab`].
    ///
    /// [`Gitlab`]: Host::Gitlab
    #[must_use]
    pub const fn is_gitlab(&self) -> bool {
        matches!(self, Self::Gitlab)
    }

    /// Returns `true` if the git host is [`Bitbucket`].
    ///
    /// [`Bitbucket`]: Host::Bitbucket
    #[must_use]
    pub const fn is_bitbucket(&self) -> bool {
        matches!(self, Self::Bitbucket)
    }

    fn fetch_options<'cb>(self) -> (git2::FetchOptions<'cb>, FetchMethod) {
        let mut callbacks = git2::RemoteCallbacks::new();
        let env_var = format!("AUDITEUR_SSH_{}", self.name().to_uppercase());
        let ssh = std::env::var(env_var).map_or(FetchMethod::Https, |key| {
            let key = PathBuf::from(key);
            callbacks.credentials(move |_url, user_from_url, cred_type| {
                let user_from_url = user_from_url.unwrap_or("git");
                if cred_type.contains(CredentialType::USERNAME) {
                    git2::Cred::username(user_from_url)
                } else {
                    git2::Cred::ssh_key(user_from_url, None, &key, None)
                }
            });
            FetchMethod::Ssh
        });

        let mut fo = git2::FetchOptions::new();
        fo.remote_callbacks(callbacks);

        (fo, ssh)
    }
}

impl Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.url())
    }
}

/// A git host was not recognized so it couldn't be [parsed](Host::from_str)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Error)]
#[error("unknown git host: {0}")]
pub struct UnknownHost(String);

impl FromStr for Host {
    type Err = UnknownHost;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github.com" => Ok(Self::Github),
            "gitlab.com" => Ok(Self::Gitlab),
            "bitbucket.org" => Ok(Self::Bitbucket),
            unknown => Err(UnknownHost(unknown.to_owned())),
        }
    }
}

/// A network protocol that can be used to fetch a git repository from a hosting service
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FetchMethod {
    Https,
    Ssh,
}

impl FetchMethod {
    /// Returns `true` if the git fetch method is [`Https`].
    ///
    /// [`Https`]: FetchMethod::Https
    #[must_use]
    pub const fn is_https(&self) -> bool {
        matches!(self, Self::Https)
    }

    /// Returns `true` if the git fetch method is [`Ssh`].
    ///
    /// [`Ssh`]: FetchMethod::Ssh
    #[must_use]
    pub const fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh)
    }

    #[must_use]
    pub fn fetch_url(&self, host: Host, user: &str, repo: &str) -> String {
        match self {
            Self::Https => format!("https://{host}/{user}/{repo}.git"),
            Self::Ssh => format!("git@{host}:{user}/{repo}.git"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostedShorthandRef<'a> {
    pub host: Host,
    pub shorthand: &'a Shorthand,
}

impl HostedShorthandRef<'_> {
    #[must_use]
    pub fn fetch_url(&self, method: FetchMethod) -> String {
        method.fetch_url(self.host, &self.shorthand.user, &self.shorthand.repo)
    }

    #[must_use]
    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    #[must_use]
    pub fn to_owned(self) -> HostedShorthand {
        HostedShorthand {
            host: self.host,
            shorthand: self.shorthand.to_owned(),
        }
    }

    /// Clone this shorthand into `path`
    ///
    /// # Errors
    ///
    /// - If there is an error with [`git`](git2)
    pub fn clone_into(&self, path: &Path) -> Result<(), Error> {
        let (fetch_options, fetch_method) = self.host.fetch_options();
        let url = self.fetch_url(fetch_method);
        git2::build::RepoBuilder::new()
            .fetch_options(fetch_options)
            .clone(&url, path)
            .and_then(|repo| update_submodules_of(&repo, self.host))
            .map_err(|error| Error::Git { error, url })?;
        Ok(())
    }
}

impl Display for HostedShorthandRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { host, shorthand } = self;
        write!(f, "{host} - {shorthand}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostedShorthand {
    pub host: Host,
    pub shorthand: Shorthand,
}

impl HostedShorthand {
    #[must_use]
    pub fn fetch_url(&self, method: FetchMethod) -> String {
        self.to_ref().fetch_url(method)
    }

    #[must_use]
    pub fn repo(&self) -> &str {
        &self.shorthand.repo
    }

    #[must_use]
    pub const fn to_ref(&self) -> HostedShorthandRef<'_> {
        HostedShorthandRef {
            host: self.host,
            shorthand: &self.shorthand,
        }
    }

    /// Clone this shorthand into `path`
    ///
    /// # Errors
    ///
    /// - If there is an error with [`git`](git2)
    pub fn clone_into(&self, path: &Path) -> Result<(), Error> {
        self.to_ref().clone_into(path)
    }
}

impl Display for HostedShorthand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_ref().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FetchUrl {
    pub method: FetchMethod,
    pub shorthand: HostedShorthand,
}

impl FetchUrl {
    #[must_use]
    pub fn fetch_url(&self) -> String {
        self.shorthand.fetch_url(self.method)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Error)]
pub enum UrlParseError {
    #[error("{0}")]
    UnknownHost(#[from] UnknownHost),
    #[error("could not parse git url: {0}")]
    Unknown(String),
}

impl FromStr for FetchUrl {
    type Err = UrlParseError;
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
            (FetchMethod::Https, captures)
        } else if let Some(captures) = ssh_regex.captures(s) {
            (FetchMethod::Ssh, captures)
        } else {
            return Err(UrlParseError::Unknown(s.to_owned()));
        };

        Ok(Self {
            method,
            // both regex have the same capture groups for the shorthand
            shorthand: HostedShorthand {
                host: captures["host"].parse()?,
                shorthand: Shorthand {
                    user: captures["user"].to_owned(),
                    repo: captures["repo"].to_owned(),
                },
            },
        })
    }
}
