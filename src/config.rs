use crate::{
    context::Context,
    specs::{BuildSpec, BuildUnit, BuildUnitKind, ResolveInto},
    YurtArgs,
};

use anyhow::{bail, Context as _, Result};
use clap::crate_version;
use lazy_static::lazy_static;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

lazy_static! {
    static ref VERSION: Version = Version::parse(crate_version!()).unwrap();
    static ref DEFAULT_VERSION_REQ: VersionReq = VersionReq::parse(crate_version!()).unwrap();
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    // Members should be treated as immutable
    pub context: Context,
    build: Vec<BuildUnit>,
    version: VersionReq,
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

    fn _include(unit: &BuildUnit, units: &[BuildUnitKind]) -> bool {
        match unit {
            BuildUnit::Repo(_) => units.contains(&BuildUnitKind::Repo),
            BuildUnit::Link(_) => units.contains(&BuildUnitKind::Link),
            BuildUnit::Install(_) => units.contains(&BuildUnitKind::Install),
            BuildUnit::Require(_) => units.contains(&BuildUnitKind::Require),
            BuildUnit::Run(_) => units.contains(&BuildUnitKind::Run),
        }
    }

    #[inline]
    fn include(self, units: &[BuildUnitKind]) -> Self {
        self.filter(|unit| Self::_include(unit, units))
    }

    #[inline]
    fn exclude(self, units: &[BuildUnitKind]) -> Self {
        self.filter(|unit| !Self::_include(unit, units))
    }

    pub fn nontrivial(self) -> Self {
        self.filter(|unit| match unit {
            BuildUnit::Repo(repo) => !repo.is_available(),
            BuildUnit::Link(link) => !link.is_valid(),
            BuildUnit::Install(package) => !package.is_installed(),
            BuildUnit::Require(manager) => !manager.is_available(),
            BuildUnit::Run(_) => true,
        })
    }

    pub fn for_each_unit<F>(self, f: F) -> Result<()>
    where
        F: FnMut(BuildUnit) -> Result<()>,
    {
        self.build.into_iter().try_for_each(f)
    }

    pub fn into_config(self) -> Config {
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
}

impl TryFrom<&YurtArgs> for ResolvedConfig {
    type Error = anyhow::Error;

    fn try_from(args: &YurtArgs) -> Result<Self> {
        Config::try_from(args)
            .and_then(|c| c.resolve(Context::from(args)))
            .map(|r| match args {
                YurtArgs {
                    include: Some(units),
                    ..
                } => r.include(units),
                YurtArgs {
                    exclude: Some(units),
                    ..
                } => r.exclude(units),
                _ => r,
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

    pub fn resolve(self, mut context: Context) -> Result<ResolvedConfig> {
        // Check version
        let version = match self.version {
            Some(req) if req.matches(&VERSION) => req,
            Some(req) => bail!("Version requirement not satisfied: {} ({})", req, *VERSION),
            None => DEFAULT_VERSION_REQ.clone(),
        };
        // Resolve build
        Ok(ResolvedConfig {
            build: self
                .build
                .resolve_into_new(&mut context)
                .context("Failed to resolve build")?,
            version,
            context,
        })
    }

    pub fn yaml(&self) -> Result<String> {
        serde_yaml::to_string(&self).context("Failed to serialize config")
    }
}

impl TryFrom<&YurtArgs> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &YurtArgs) -> Result<Self> {
        if let Some(ref url) = args.yaml_url {
            Self::from_url(url)
        } else {
            Self::from_path(match args.yaml {
                Some(ref path) => path.clone(),
                None => env::var("YURT_BUILD_FILE")
                    .map(PathBuf::from)
                    .context("Build file not specified")?,
            })
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    mod yaml {
        use super::*;
        use clap::Parser;
        use std::fs::read_to_string;
        use std::path::PathBuf;

        struct TestData {
            input_path: PathBuf,
            output_path: PathBuf,
            args_path: PathBuf,
        }

        impl TestData {
            fn new(parts: &[&str]) -> Self {
                let dir: PathBuf = [&[env!("CARGO_MANIFEST_DIR"), "test"], parts]
                    .concat()
                    .iter()
                    .collect();
                Self {
                    input_path: dir.join("input.yaml"),
                    output_path: dir.join("output.yaml"),
                    args_path: dir.join("args"),
                }
            }

            fn get_args(&self) -> YurtArgs {
                let mut args = vec![
                    "yurt".to_string(),
                    "--yaml".to_string(),
                    self.input_path.to_str().unwrap().to_owned(),
                ];
                if self.args_path.is_file() {
                    args.extend(
                        read_to_string(&self.args_path)
                            .unwrap()
                            .split(' ')
                            .map(String::from),
                    );
                }
                args.push("show".to_string());
                YurtArgs::try_parse_from(args).expect("Failed to parse args")
            }

            fn get_output_yaml(&self) -> String {
                read_to_string(&self.output_path).expect("Failed to read output comparison")
            }
        }

        mod io {
            use super::*;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let test = TestData::new(&["io", stringify!($name)]);
                        let resolved_yaml = ResolvedConfig::try_from(&test.get_args())
                            .expect("Failed to resolve input build")
                            .into_config()
                            .yaml()
                            .expect("Failed to generate resolved yaml");
                        pretty_assertions::assert_eq!(resolved_yaml, test.get_output_yaml())
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
            use super::*;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let test = TestData::new(&["invalid", "parse", stringify!($name)]);
                        assert!(Config::try_from(&test.get_args()).is_err());
                    }
                };
            }

            test_case!(empty);
            test_case!(no_build);
            test_case!(unknown_key);
        }

        mod invalid_resolve {
            use super::*;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let test = TestData::new(&["invalid", "resolve", stringify!($name)]);
                        let args = test.get_args();
                        assert!(Config::try_from(&args).is_ok());
                        assert!(ResolvedConfig::try_from(&args).is_err());
                    }
                };
            }

            test_case!(version_mismatch);
        }
    }
}
