use crate::{
    context::Context,
    specs::{BuildSpec, BuildUnit, ResolveInto},
};

use anyhow::{bail, Context as _, Result};
use clap::{crate_version, ArgMatches};
use lazy_static::lazy_static;
use log::info;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, env, fs::File, io::BufReader, path::Path};

lazy_static! {
    static ref VERSION: Version = Version::parse(crate_version!()).unwrap();
    static ref DEFAULT_VERSION_REQ: VersionReq = VersionReq::parse(crate_version!()).unwrap();
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    // Members should be treated as immutable
    context: Context,
    version: VersionReq,
    build: Vec<BuildUnit>,
}

impl ResolvedConfig {
    #[inline]
    fn filter<P>(self, predicate: P) -> Self
    where
        P: FnMut(&BuildUnit) -> bool,
    {
        Self {
            build: self.build.into_iter().filter(predicate).collect(),
            ..self
        }
    }

    fn nontrivial(self) -> Self {
        self.filter(|unit| match unit {
            BuildUnit::Repo(repo) => !repo.is_available(),
            BuildUnit::Link(link) => !link.is_valid(),
            BuildUnit::Install(package) => !package.is_installed(),
            BuildUnit::Require(manager) => !manager.is_available(),
            BuildUnit::Run(_) => true,
        })
    }

    fn _include(unit: &BuildUnit, units: &HashSet<String>) -> bool {
        match unit {
            BuildUnit::Repo(_) => units.contains("repo"),
            BuildUnit::Link(_) => units.contains("link"),
            BuildUnit::Install(_) => units.contains("install"),
            BuildUnit::Require(_) => units.contains("require"),
            BuildUnit::Run(_) => units.contains("run"),
        }
    }

    #[inline]
    fn include(self, units: &HashSet<String>) -> Self {
        self.filter(|unit| Self::_include(unit, units))
    }

    #[inline]
    fn exclude(self, units: &HashSet<String>) -> Self {
        self.filter(|unit| !Self::_include(unit, units))
    }

    /// Pretty-print the complete build; optionally filter out trivial units
    pub fn show(&self, nontrivial: bool) -> Result<()> {
        print!(
            "{}",
            match nontrivial {
                true => self.clone().nontrivial(),
                false => self.clone(),
            }
            .into_yaml()?
        );
        Ok(())
    }

    /// Eliminate elements that will conflict with installation
    pub fn clean(&self) -> Result<()> {
        info!("Cleaning link heads...");
        for unit in &self.build {
            match unit {
                BuildUnit::Link(link) => link.clean()?,
                _ => continue,
            }
        }
        Ok(())
    }

    pub fn install(&self, clean: bool) -> Result<()> {
        info!("Installing...");
        for unit in &self.build {
            match unit {
                BuildUnit::Repo(repo) => drop(repo.require()?),
                BuildUnit::Link(link) => link.link(clean)?,
                BuildUnit::Run(cmd) => cmd.run()?,
                BuildUnit::Install(package) => package.install()?,
                BuildUnit::Require(manager) => manager.require()?,
            }
        }
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        info!("Uninstalling...");
        for unit in &self.build {
            match unit {
                BuildUnit::Link(link) => link.unlink()?,
                BuildUnit::Install(package) => package.uninstall()?,
                _ => continue,
            }
        }
        Ok(())
    }

    #[allow(clippy::unused_self)]
    pub fn update(&self) -> Result<()> {
        todo!()
    }

    fn into_config(self) -> Config {
        let mut build: Vec<BuildSpec> = Vec::new();
        for unit in self.build {
            if let Some(spec) = build.last_mut() {
                if spec.absorb(&unit) {
                    continue;
                }
            }
            build.push(match unit {
                BuildUnit::Repo(repo) => BuildSpec::Repo(repo),
                BuildUnit::Link(link) => BuildSpec::Link(vec![link]),
                BuildUnit::Run(cmd) => BuildSpec::Run(cmd),
                BuildUnit::Install(package) => BuildSpec::Install(vec![package]),
                BuildUnit::Require(manager) => BuildSpec::Require(vec![manager]),
            });
        }
        Config {
            version: Some(self.version),
            build,
        }
    }

    fn into_yaml(self) -> Result<String> {
        serde_yaml::to_string(&self.into_config()).context("Failed to serialize config")
    }
}

impl TryFrom<&ArgMatches> for ResolvedConfig {
    type Error = anyhow::Error;

    fn try_from(args: &ArgMatches) -> Result<Self> {
        Config::try_from(args)
            .and_then(|c| c.resolve(Context::from(args)))
            .map(|r| {
                if let Some(units) = args.get_many::<String>("include") {
                    r.include(&units.cloned().collect())
                } else if let Some(units) = args.get_many::<String>("exclude") {
                    r.exclude(&units.cloned().collect())
                } else {
                    r
                }
            })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    version: Option<VersionReq>,
    build: Vec<BuildSpec>,
}

impl Config {
    fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        File::open(path)
            .map(BufReader::new)
            .context("Failed to open build file")
            .and_then(|reader| {
                serde_yaml::from_reader(reader).context("Failed to deserialize build file")
            })
    }

    fn from_url<U: reqwest::IntoUrl>(url: U) -> Result<Self> {
        reqwest::blocking::get(url)
            .context("Failed to reach remote build file")
            .and_then(|response| {
                serde_yaml::from_reader(response).context("Failed to deserialize remote build file")
            })
    }

    fn resolve(self, mut context: Context) -> Result<ResolvedConfig> {
        // Check version
        let version = match self.version {
            Some(req) if req.matches(&VERSION) => req,
            Some(req) => bail!("Version requirement not satisfied: {} ({})", req, *VERSION),
            None => DEFAULT_VERSION_REQ.clone(),
        };
        // Resolve build
        Ok(ResolvedConfig {
            build: self.build.resolve_into_new(&mut context)?,
            version,
            context,
        })
    }
}

impl TryFrom<&ArgMatches> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &ArgMatches) -> Result<Self> {
        if let Some(url) = args.get_one::<String>("yaml-url") {
            Self::from_url(url)
        } else {
            Self::from_path(match args.get_one::<String>("yaml") {
                Some(path) => path.clone(),
                None => env::var("YURT_BUILD_FILE").context("Build file not specified")?,
            })
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::yurt_command;

    mod yaml {
        use super::*;

        mod io {
            use super::*;
            use std::fs::read_to_string;
            use std::path::{Path, PathBuf};

            fn get_test_path(parts: &[&str]) -> PathBuf {
                [&[env!("CARGO_MANIFEST_DIR"), "test"], parts]
                    .concat()
                    .iter()
                    .collect()
            }

            fn get_resolved(dir: &Path) -> ResolvedConfig {
                let input = dir
                    .join("input.yaml")
                    .into_os_string()
                    .into_string()
                    .unwrap();
                let mut args = vec!["yurt".to_string(), "--yaml".to_string(), input];
                let arg_path = dir.join("args");
                if arg_path.is_file() {
                    args.extend(
                        read_to_string(dir.join("args"))
                            .unwrap()
                            .split(' ')
                            .map(String::from),
                    );
                }
                ResolvedConfig::try_from(&yurt_command().get_matches_from(args))
                    .expect("Failed to resolve input build")
            }

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let dir = get_test_path(&["io", stringify!($name)]);
                        let resolved = get_resolved(&dir);
                        let raw_output = read_to_string(&dir.join("output.yaml"))
                            .expect("Failed to read output");
                        let yaml = resolved.into_yaml().unwrap();
                        pretty_assertions::assert_eq!(yaml, raw_output)
                    }
                };
            }

            test_case!(packages);
            test_case!(packages_expanded);
            test_case!(matrix);
            test_case!(vars);
            test_case!(case);
            test_case!(repo);
            test_case!(exclude);
            test_case!(include);
        }

        mod invalid_parse {
            use super::Config;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let path = concat!("../test/invalid/parse/", stringify!($name), ".yaml");
                        assert!(Config::from_path(path).is_err());
                    }
                };
            }

            test_case!(empty);
            test_case!(no_build);
            test_case!(unknown_key);
            test_case!(version_mismatch);
        }
    }
}
