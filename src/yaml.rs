use super::error::YurtResult;
use super::link::Link;
use super::pack::{Package, PackageManager};
use super::repo::Repo;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use whoami;

lazy_static! {
    pub static ref LOCALE: Locale<Cow<'static, str>> = Locale {
        user: Cow::Owned(whoami::username()),
        platform: Cow::Owned(format!("{:?}", whoami::platform())),
        distro: Cow::Owned(
            whoami::distro()
                .split(" ")
                .next()
                .map(String::from)
                .expect("failed to determine distro"),
        ),
    };
}

pub fn apply<RL, RP, RB, E>(
    units: Vec<BuildUnit>,
    lf: fn(Link) -> Result<RL, E>,
    pf: fn(Package) -> Result<RP, E>,
    bf: fn(PackageManager) -> Result<RB, E>,
) -> Result<(), E> {
    for unit in units {
        match unit {
            BuildUnit::Link(ln) => drop(lf(ln)?),
            BuildUnit::Package(pkg) => drop(pf(pkg)?),
            BuildUnit::Bootstrap(pm) => drop(bf(pm)?),
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Locale<T> {
    user: T,
    platform: T,
    distro: T,
}

impl Locale<Option<String>> {
    pub fn is_local(&self) -> bool {
        let s_vals = vec![
            self.user.as_deref(),
            self.platform.as_deref(),
            self.distro.as_deref(),
        ];
        let o_vals = vec![
            LOCALE.user.as_ref(),
            LOCALE.platform.as_ref(),
            LOCALE.distro.as_ref(),
        ];
        s_vals
            .into_iter()
            .zip(o_vals.into_iter())
            .all(|(s, o)| match s {
                Some(val) if val != o => false,
                _ => true,
            })
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case {
    Local {
        spec: Locale<Option<String>>,
        build: Vec<BuildSet>,
    },
    Default {
        build: Vec<BuildSet>,
    },
}

#[derive(Debug, Clone)]
pub enum BuildUnit {
    Link(Link),
    Package(Package),
    Bootstrap(PackageManager),
}

impl Into<Option<Link>> for BuildUnit {
    fn into(self) -> Option<Link> {
        match self {
            Self::Link(v) => Some(v),
            _ => None,
        }
    }
}

impl Into<Option<Package>> for BuildUnit {
    fn into(self) -> Option<Package> {
        match self {
            Self::Package(v) => Some(v),
            _ => None,
        }
    }
}

impl Into<Option<PackageManager>> for BuildUnit {
    fn into(self) -> Option<PackageManager> {
        match self {
            Self::Bootstrap(v) => Some(v),
            _ => None,
        }
    }
}

impl From<Link> for BuildUnit {
    fn from(ln: Link) -> BuildUnit {
        BuildUnit::Link(ln)
    }
}

impl From<Package> for BuildUnit {
    fn from(pkg: Package) -> BuildUnit {
        BuildUnit::Package(pkg)
    }
}

impl From<PackageManager> for BuildUnit {
    fn from(pkg: PackageManager) -> BuildUnit {
        BuildUnit::Bootstrap(pkg)
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub enum BuildSet {
    #[serde(rename = "case")]
    CaseVec(Vec<Case>),
    #[serde(rename = "link")]
    LinkVec(Vec<Link>),
    #[serde(rename = "install")]
    PackageVec(Vec<Package>),
    #[serde(rename = "bootstrap")]
    BootstrapVec(Vec<PackageManager>),
}

impl BuildSet {
    // Recursively resolve all case units; collect into single vec
    pub fn resolve(&self) -> YurtResult<Vec<BuildUnit>> {
        match self {
            // Recursively filter cases
            Self::CaseVec(case_vec) => {
                let mut default = true;
                let mut unit_vec: Vec<BuildUnit> = Vec::new();
                for case in case_vec {
                    match case {
                        Case::Local { spec, build } if spec.is_local() => {
                            default = false;
                            for set in build {
                                unit_vec.extend(set.resolve()?)
                            }
                        }
                        Case::Default { build } if default => {
                            for set in build {
                                unit_vec.extend(set.resolve()?)
                            }
                        }
                        _ => (),
                    };
                }
                Ok(unit_vec)
            }
            // Expand links
            Self::LinkVec(link_vec) => link_vec
                .iter()
                .map(|la| la.expand().and_then(|lb| Ok(lb.into())))
                .collect(),
            // Clone packages
            Self::PackageVec(pkg_vec) => pkg_vec
                .iter() // expand
                .map(|pkg| Ok(pkg.clone().into()))
                .collect(),
            // Clone package managers
            Self::BootstrapVec(pm_vec) => pm_vec
                .iter() // expand
                .map(|pm| Ok(pm.clone().into()))
                .collect(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Build {
    pub repo: Repo,
    pub build: Vec<BuildSet>,
}

impl Build {
    pub fn from_path<P: AsRef<Path>>(path: P) -> YurtResult<Self> {
        let file = File::open(path)?;
        Self::from_file(file)
    }

    pub fn from_file(file: File) -> YurtResult<Self> {
        let reader = BufReader::new(file);
        Ok(serde_yaml::from_reader::<_, Self>(reader)?)
    }

    pub fn resolve(&self) -> YurtResult<(Repo, Vec<BuildUnit>)> {
        // Resolve repo
        let repo = self.repo.resolve()?;
        env::set_var("YURT_REPO_LOCAL", &repo.local);
        let mut build_vec: Vec<BuildUnit> = Vec::new();
        for set in &self.build {
            build_vec.extend(set.resolve()?);
        }
        Ok((repo, build_vec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::prelude::*;
    use tempfile;

    const YAML: &str = "
---
repo:
  local: $HOME/dotfiles
  remote: https://github.com/user/dotfiles.git
build:
  - case:
    - local:
        spec:
          user: notme
        build:
        - install:
          - name: wrong
            managers:
            - apt
    - default:
        build:
        - install:
          - name: right
            managers:
            - apt
            - brew
  - link:
    - tail: some/file
      head: some/link
  - bootstrap:
    - cargo
";

    fn yaml_file() -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(YAML.as_bytes()).unwrap();
        file
    }

    #[test]
    fn build_parses() {
        let file = yaml_file();
        Build::from_path(file.path()).unwrap();
    }

    #[test]
    fn build_resolves() {
        let file = yaml_file();
        let build = Build::from_path(file.path()).unwrap();
        build.resolve().unwrap();
    }

    #[test]
    fn empty_build_set() {
        let set = BuildSet::CaseVec(vec![Case::Local {
            spec: Locale {
                user: None,
                platform: Some("nothere".to_string()),
                distro: None,
            },
            build: vec![BuildSet::LinkVec(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve().unwrap().is_empty());
    }
}
