use crate::{
    context::Context,
    specs::{BuildSpec, BuildUnit, ResolveInto},
    yaml_example_doc,
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
pub struct ResolvedConfig {
    // Members should be treated as immutable
    pub context: Context,
    build: Vec<BuildUnit>,
    version: Option<VersionReq>,
}

impl ResolvedConfig {
    #[inline]
    pub fn filter<P>(self, predicate: P) -> Self
    where
        P: Fn(&BuildUnit, &Context) -> bool,
    {
        Self {
            build: self
                .build
                .into_iter()
                .filter(|unit| predicate(unit, &self.context))
                .collect(),
            ..self
        }
    }

    #[inline]
    pub fn for_each_unit<F>(&self, f: F) -> Result<()>
    where
        F: Fn(&BuildUnit, &Context) -> Result<()>,
    {
        self.build
            .iter()
            .try_for_each(|unit| f(unit, &self.context))
    }

    pub fn into_config(self) -> Config {
        Config {
            version: self.version,
            build: self.build.into_iter().map(Into::into).collect(),
        }
    }
}

/// Top level yurt build file YAML object.
///
/// Order of build steps is preserved after resolution.
/// Some build steps (such as [`!vars`](BuildSpec::Vars) and
/// [`!package_manager`](BuildSpec::PackageManager)) modify the resolver state.
/// The order of build steps may change the resolved values.
#[doc = yaml_example_doc!("config.yaml")]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<VersionReq>,
    build: Vec<BuildSpec>,
}

impl Config {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        File::open(path)
            .map(BufReader::new)
            .context("Failed to open build file")
            .and_then(|reader| {
                serde_yaml::from_reader(reader).context("Failed to deserialize build file")
            })
    }

    pub fn from_env() -> Result<Self> {
        env::var("YURT_BUILD_FILE")
            .map(PathBuf::from)
            .context("Build file not specified")
            .and_then(Self::from_path)
    }

    pub fn from_url(url: &str) -> Result<Self> {
        minreq::get(url)
            .send()
            .context("Failed to reach remote build file")
            .and_then(|response| {
                serde_yaml::from_reader(response.as_bytes())
                    .context("Failed to deserialize remote build file")
            })
    }

    pub fn resolve(self, mut context: Context) -> Result<ResolvedConfig> {
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

#[cfg(test)]
pub mod tests {
    use super::*;

    mod yaml {
        use super::*;
        use crate::YurtArgs;
        use clap::Parser;
        use std::fs::read_to_string;
        use std::path::PathBuf;

        struct TestData {
            input: PathBuf,
            output: PathBuf,
            args: PathBuf,
        }

        impl TestData {
            fn new(parts: &[&str]) -> Self {
                let dir: PathBuf = [&[env!("CARGO_MANIFEST_DIR"), "yaml", "tests"], parts]
                    .concat()
                    .iter()
                    .collect();
                Self {
                    input: dir.join("input.yaml"),
                    output: dir.join("output.yaml"),
                    args: dir.join("args"),
                }
            }

            fn get_args(&self) -> YurtArgs {
                let mut args = vec![
                    "yurt".to_string(),
                    "--file".to_string(),
                    self.input.to_str().unwrap().to_owned(),
                ];
                if self.args.is_file() {
                    args.extend(
                        read_to_string(&self.args)
                            .unwrap()
                            .split(' ')
                            .map(String::from),
                    );
                }
                args.push("show".to_string());
                YurtArgs::try_parse_from(args).expect("Failed to parse args")
            }

            fn get_output_yaml(&self) -> String {
                read_to_string(&self.output).expect("Failed to read output comparison")
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
                        let resolved_yaml = args
                            .get_resolved_config()
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
            test_case!(shell);
        }

        mod invalid_parse {
            use super::*;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let test = TestData::new(&["invalid", "parse", stringify!($name)]);
                        assert!(test.get_args().get_resolved_config().is_err());
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
                        assert!(test.get_args().get_resolved_config().is_err());
                    }
                };
            }

            test_case!(version_mismatch);
        }
    }
}
