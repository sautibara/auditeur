#![allow(dead_code, unused_variables)]

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    io::Read,
    path::PathBuf,
    process::Stdio,
};

use cargo_metadata::Message as CargoMessage;
use cargo_toml::Dependency;
use log::warn;
use tokio::io::AsyncWriteExt;

use crate::prelude::*;

use super::manifest::Manifest;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompileOptions {
    target: String,
    version: String,
    opt_level: String,
    debug: bool,
    num_jobs: u32,
    incremental: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageName(String);

impl Borrow<str> for PackageName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Crate {
    source: PathBuf,
    mirror: PathBuf,
    package_name: PackageName,
    library_name: String,
}

#[derive(Default)]
pub struct BuildArtifacts {
    pub artifacts: HashMap<PackageName, PathBuf>,
    pub errors: Vec<cargo_metadata::CompilerMessage>,
}

impl BuildArtifacts {
    fn add_artifact(&mut self, artifact: cargo_metadata::Artifact) {
        let package = PackageName(artifact.target.name);
        if let [binary] = &artifact.filenames[..] {
            self.artifacts
                .insert(package, binary.as_std_path().to_owned());
        } else {
            warn!(
                "incorrect number of binaries received for package {}. found {}, expected 1",
                package.0,
                artifact.filenames.len()
            );
        }
    }
}

pub struct Workspace {
    path: PathBuf,
    opts: CompileOptions,
    crates: HashMap<PackageName, Crate>,
}

impl Workspace {
    const PROFILE: &'static str = "auditeur";

    pub async fn new(path: impl Into<PathBuf> + Send, opts: CompileOptions) -> Result<Self, Error> {
        let path = path.into();
        tokio::fs::create_dir_all(&path).await?;
        let workspace = Self {
            path,
            opts,
            crates: HashMap::new(),
        };
        workspace.check_version().await?;
        Ok(workspace)
    }

    pub const fn crates(&self) -> &HashMap<PackageName, Crate> {
        &self.crates
    }

    async fn check_version(&self) -> Result<(), Error> {
        let found_bytes = tokio::process::Command::new("rustc")
            .arg("--version")
            .output()
            .await?
            .stdout;
        let found = String::from_utf8_lossy(&found_bytes).trim().to_owned();
        let expected = &self.opts.version;
        if &found == expected {
            Ok(())
        } else {
            Err(Error::InvalidRustc {
                found,
                expected: expected.to_owned(),
            })
        }
    }

    async fn init_workspace_manifest(&self) -> Result<(), Error> {
        let path_list = self
            .crates
            .values()
            .map(|crt| crt.mirror.to_string_lossy())
            .fold('\n'.to_string(), |mut string, path| {
                string.push_str(&format!("  \"{path}\",\n"));
                string
            });
        let opt_level = &self.opts.opt_level;
        let debug = self.opts.debug;
        let profile = Self::PROFILE;
        let workspace_toml = format!(
            r#"
[workspace]

members = [{path_list}]
resolver = "2"

[profile.{profile}]
inherits = "release"
opt-level = {opt_level}
debug = {debug}
"#
        );
        tokio::fs::File::create(self.path.join("Cargo.toml"))
            .await?
            .write_all(workspace_toml.as_bytes())
            .await?;
        Ok(())
    }

    pub async fn add(
        &mut self,
        source: PathBuf,
        dependencies: &HashMap<PackageName, PluginName>,
        plugin: &PluginName,
    ) -> Result<(), Error> {
        let manifest = Manifest::load_dir(&source).await?;

        let Some(package) = manifest.package.as_ref() else {
            return Err(Error::PackageExpected {
                manifest_path: source.join("Cargo.toml"),
                plugin: plugin.clone(),
            });
        };
        let package_name = PackageName(package.name.clone());

        let Some(library) = manifest.lib.as_ref() else {
            return Err(Error::LibraryExpected {
                manifest_path: source.join("Cargo.toml"),
                plugin: plugin.clone(),
            });
        };
        let library_name = (library.name)
            .as_ref()
            .cloned()
            .unwrap_or_else(|| package_name.0.replace('-', "_"));

        let mirror_path = self.path.join(&package_name.0);
        let mut manifest = manifest.copy_to_path(&mirror_path);
        for (dependency, dep_plugin) in dependencies {
            let (dep_name, dependency) =
                self.create_dependency_link(dependency, plugin, dep_plugin)?;
            manifest.dependencies.insert(dep_name, dependency);
        }
        manifest.save().await?;

        let workspace_crate = Crate {
            mirror: mirror_path,
            source: source.clone(),
            package_name: package_name.clone(),
            library_name,
        };
        self.crates.insert(package_name, workspace_crate);

        Ok(())
    }

    fn create_dependency_link(
        &self,
        dep_package: &PackageName,
        dep_plugin: &PluginName,
        for_plugin: &PluginName,
    ) -> Result<(String, Dependency), Error> {
        let crt = self
            .crates
            .get(dep_package)
            .ok_or_else(|| Error::DependencyNotFound {
                plugin: for_plugin.clone(),
                dependency: dep_plugin.clone(),
            })?;
        let dependency = Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
            path: Some(crt.source.to_string_lossy().into_owned()),
            ..Default::default()
        }));
        Ok((crt.package_name.0.clone(), dependency))
    }

    async fn build(&mut self, packages: &HashSet<PackageName>) -> Result<BuildArtifacts, Error> {
        if packages.is_empty() {
            return Ok(BuildArtifacts::default());
        }

        // need to make sure that the workspace manifest is here before building
        self.init_workspace_manifest().await?;

        // only build the plugins that haven't been built yet by specifing their crate names
        let package_specifiers = packages
            .iter()
            .flat_map(|package| ["--package", package.borrow()])
            .map(ToOwned::to_owned);
        let mut build_command = std::process::Command::new("cargo")
            .arg("build")
            .args(package_specifiers)
            // canaries
            .args(["--profile", Self::PROFILE])
            .args(["--target", &self.opts.target])
            .args(["--jobs", &self.opts.num_jobs.to_string()])
            .env(
                "CARGO_INCREMENTAL",
                usize::from(self.opts.incremental).to_string(),
            )
            // output
            .arg("--message-format=json-render-diagnostics") // output as json
            .arg("--quiet") // avoid non-json messages
            .arg("--keep-going") // keep going even if one plugin fails to compile
            // run the command at the root of the workspace
            .current_dir(&self.path)
            // pipe the stdout of the build command to this process
            .stdout(Stdio::piped())
            .spawn()?;

        let message_stream = build_command.stdout.take().expect("stdout should be piped");
        // collect the names out here so it can be given to the other thread without references
        // these are used to filter the cargo messages so that they only contain plugin artifacts
        // parse the messages in another thread since cargo_metadata doesn't work with async
        let libraries = packages
            .iter()
            .filter_map(|package| self.crates.get(package))
            .map(|crt| crt.library_name.clone())
            .collect();
        let messages: Vec<CargoMessage> =
            crate::spawn_blocking(move || Self::parse_messages(message_stream, &libraries)).await?;
        // wait for the process to finish
        let _exit_code = crate::spawn_blocking(move || build_command.wait()).await?;

        let mut artifacts = BuildArtifacts::default();
        for message in messages {
            match message {
                CargoMessage::CompilerArtifact(artifact) => artifacts.add_artifact(artifact),
                CargoMessage::CompilerMessage(message) => artifacts.errors.push(message),
                _ => unreachable!(),
            }
        }

        Ok(artifacts)
    }

    fn parse_messages(
        stream: impl Read,
        library_names: &HashSet<String>,
    ) -> std::io::Result<Vec<CargoMessage>> {
        let reader = std::io::BufReader::new(stream);
        CargoMessage::parse_stream(reader)
            .filter(|message| match message {
                Ok(CargoMessage::CompilerArtifact(artifact))
                    if library_names.contains(&*artifact.target.name) =>
                {
                    true
                }
                Ok(CargoMessage::CompilerMessage(_)) | Err(_) => true,
                _ => false,
            })
            .collect()
    }
}
