use crate::{
    context::Context,
    specs::{BuildSpec, BuildUnit, Hook, ResolveInto},
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
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedConfig<'c> {
    // Members should be treated as immutable
    pub context: &'c Context,
    build: Vec<BuildUnit>,
    version: Option<VersionReq>,
}

impl<'c> ResolvedConfig<'c> {
    #[inline]
    pub fn filter<P>(self, predicate: P) -> Self
    where
        P: FnMut(&BuildUnit) -> bool,
    {
        Self {
            build: self.build.into_iter().filter(predicate).collect(),
            ..self
        }
    }

    pub fn filter_args(self, args: &YurtArgs) -> Self {
        match args {
            YurtArgs {
                include: Some(units),
                ..
            } => self.filter(|unit| unit.included_in(units)),
            YurtArgs {
                exclude: Some(units),
                ..
            } => self.filter(|unit| !unit.included_in(units)),
            _ => self,
        }
    }

    pub fn filter_nontrivial(self, context: &Context) -> Self {
        self.filter(|unit| match unit {
            BuildUnit::Repo(repo) => !repo.is_available(),
            BuildUnit::Link(link) => !link.is_valid(),
            BuildUnit::Package(package) => !package.is_installed(context),
            BuildUnit::PackageManager(manager) => !manager.is_available(),
            BuildUnit::Hook(hook) => hook.applies(Hook::Install),
        })
    }

    #[inline]
    pub fn for_each_unit<F>(&self, f: F) -> Result<()>
    where
        F: FnMut(&BuildUnit) -> Result<()>,
    {
        self.build.iter().try_for_each(f)
    }

    pub fn into_config(self) -> Config {
        Config {
            version: self.version,
            build: self.build.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
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

    fn from_url(url: &str) -> Result<Self> {
        minreq::get(url)
            .send()
            .context("Failed to reach remote build file")
            .and_then(|response| {
                serde_yaml::from_reader(response.as_bytes())
                    .context("Failed to deserialize remote build file")
            })
    }

    pub fn resolve(self, context: &mut Context) -> Result<ResolvedConfig<'_>> {
        // Check version
        let version = match self.version {
            Some(req) if req.matches(&VERSION) => Some(req),
            Some(req) => bail!("Version requirement not satisfied: {} ({})", req, *VERSION),
            None => None,
        };
        // Resolve build
        Ok(ResolvedConfig {
            build: self
                .build
                .resolve_into_new(context)
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
                        let args = test.get_args();
                        let mut context = Context::try_from(&args).unwrap();
                        let config = Config::try_from(&args).unwrap();
                        let resolved_yaml = config
                            .resolve(&mut context)
                            .expect("Failed to resolve input build")
                            .filter_args(&args)
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
                        let mut context = Context::try_from(&args).unwrap();
                        let config = Config::try_from(&args).unwrap();
                        assert!(config.resolve(&mut context).is_err());
                    }
                };
            }

            test_case!(version_mismatch);
        }
    }
}
