use super::link::Link;
use super::pack::{Package, PackageBundle, PackageManager, Shell};
use super::repo::Repo;
use anyhow::Result;
use clap::crate_version;
use lazy_static::lazy_static;
use log::warn;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::LinkedList;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::iter::Zip;
use std::path::Path;
use std::vec::IntoIter;

lazy_static! {
    pub static ref LOCALE: Locale<Cow<'static, str>> = Locale::new(
        Cow::Owned(whoami::username()),
        Cow::Owned(format!("{:?}", whoami::platform()).to_lowercase()),
        Cow::Owned(
            whoami::distro()
                .split(' ')
                .next()
                .map(String::from)
                .expect("failed to determine distro")
                .to_lowercase(),
        ),
    );
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Locale<T> {
    user: T,
    platform: T,
    distro: T,
}

impl<T> Locale<T> {
    pub fn new(user: T, platform: T, distro: T) -> Self {
        Self {
            user,
            platform,
            distro,
        }
    }
}

impl Locale<Option<String>> {
    fn zipped(&self) -> Zip<IntoIter<Option<&str>>, IntoIter<&str>> {
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
        s_vals.into_iter().zip(o_vals.into_iter())
    }

    pub fn is_local(&self) -> bool {
        self.zipped()
            .all(|(s, o)| !matches!(s, Some(val) if val != o))
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case {
    Positive {
        spec: Locale<Option<String>>,
        build: Vec<BuildSet>,
    },
    Negative {
        spec: Locale<Option<String>>,
        build: Vec<BuildSet>,
    },
    Default {
        build: Vec<BuildSet>,
    },
}

#[derive(Debug)]
pub enum BuildUnit {
    Link(Link),
    Package(Package),
    Bootstrap(PackageManager),
}

macro_rules! auto_convert {
    (@impl_from BuildUnit::$outer:ident, $inner:ty) => {
        impl From<BuildUnit> for Option<$inner> {
            fn from(unit: BuildUnit) -> Option<$inner> {
                match unit {
                    BuildUnit::$outer(u) => Some(u),
                    _ => None,
                }
            }
        }
    };

    (@impl_to BuildUnit::$outer:ident, $inner:ty, $var:ident, $var_map:expr) => {
        impl From<$inner> for BuildUnit {
            fn from($var: $inner) -> BuildUnit {
                BuildUnit::$outer($var_map)
            }
        }
    };

    (BuildUnit::$outer:ident, $inner:ty) => {
        auto_convert!(@impl_from BuildUnit::$outer, $inner);
        auto_convert!(@impl_to BuildUnit::$outer, $inner, x, x);
    };
    (BuildUnit::$outer:ident, $inner:ty, $var:ident, $var_map:expr) => {
        auto_convert!(@impl_from BuildUnit::$outer, $inner);
        auto_convert!(@impl_to BuildUnit::$outer, $inner, $var, $var_map);
    };
}

auto_convert!(BuildUnit::Link, Link, ln, ln.expand().unwrap());
auto_convert!(BuildUnit::Package, Package);
auto_convert!(BuildUnit::Bootstrap, PackageManager);

trait UnitResolves {
    fn resolve(self) -> Result<BuildUnit>;
}

impl<T> UnitResolves for T
where
    T: Into<BuildUnit>,
{
    fn resolve(self) -> Result<BuildUnit> {
        Ok(self.into())
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSet {
    Case(Vec<Case>),
    Link(Vec<Link>),
    Install(Vec<Package>),
    Bundle(PackageBundle),
    Bootstrap(Vec<PackageManager>),
}

trait SetResolves {
    fn resolve(self) -> Result<LinkedList<BuildUnit>>;
}

impl SetResolves for BuildSet {
    // Recursively resolve all case units; collect into single vec
    fn resolve(self) -> Result<LinkedList<BuildUnit>> {
        match self {
            Self::Case(v) => v.resolve(),
            Self::Link(v) => v.resolve(),
            Self::Install(v) => v.resolve(),
            Self::Bundle(v) => v.resolve(),
            Self::Bootstrap(v) => v.resolve(),
        }
    }
}

impl<T> SetResolves for Vec<T>
where
    T: UnitResolves,
{
    fn resolve(self) -> Result<LinkedList<BuildUnit>> {
        self.into_iter().map(|u| u.resolve()).collect()
    }
}

impl SetResolves for PackageBundle {
    fn resolve(self) -> Result<LinkedList<BuildUnit>> {
        let manager = self.manager;
        self.packages
            .into_iter()
            .map(|name| Package::new(name, vec![manager.clone()]).resolve())
            .collect()
    }
}

impl SetResolves for Vec<Case> {
    // Recursively filter cases
    fn resolve(self) -> Result<LinkedList<BuildUnit>> {
        let mut default = true;
        let mut units = LinkedList::new();
        for case in self {
            match case {
                Case::Positive { spec, build } if spec.is_local() => {
                    default = false;
                    for set in build {
                        units.extend(set.resolve()?);
                    }
                }
                Case::Negative { spec, build } if !spec.is_local() => {
                    default = false;
                    for set in build {
                        units.extend(set.resolve()?);
                    }
                }
                Case::Default { build } if default => {
                    for set in build {
                        units.extend(set.resolve()?);
                    }
                }
                _ => continue,
            };
        }
        Ok(units)
    }
}

#[derive(Debug)]
pub struct ResolvedConfig<'a> {
    pub version: Option<String>,
    pub shell: Option<Shell<'a>>,
    pub repo: Option<Repo>,
    pub build: LinkedList<BuildUnit>,
}

impl<'a> ResolvedConfig<'a> {
    pub fn map_build<RL, RP, RB, E>(
        &self,
        lf: fn(&Link) -> Result<RL, E>,
        pf: fn(&Package) -> Result<RP, E>,
        bf: fn(&PackageManager) -> Result<RB, E>,
    ) -> Result<(), E> {
        for unit in &self.build {
            match unit {
                BuildUnit::Link(ln) => drop(lf(ln)?),
                BuildUnit::Package(pkg) => drop(pf(pkg)?),
                BuildUnit::Bootstrap(pm) => drop(bf(pm)?),
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Config<'a> {
    pub version: Option<String>,
    pub shell: Option<Shell<'a>>,
    pub repo: Option<Repo>,
    pub build: Option<Vec<BuildSet>>,
}

impl<'a> Config<'a> {
    pub fn from_str<S>(string: S) -> Result<Self>
    where
        S: AsRef<str>,
    {
        Ok(serde_yaml::from_str::<Self>(string.as_ref())?)
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        Self::from_file(file)
    }

    pub fn from_file(file: File) -> Result<Self> {
        let reader = BufReader::new(file);
        Ok(serde_yaml::from_reader::<_, Self>(reader)?)
    }

    pub fn from_url(url: &str) -> Result<Self> {
        let body = reqwest::blocking::get(url)?.text()?;
        Self::from_str(body)
    }

    pub fn version_matches(&self, strict: bool) -> bool {
        if let Some(ref v) = self.version {
            return v == crate_version!();
        }
        !strict
    }

    pub fn resolve(self) -> Result<ResolvedConfig<'a>> {
        // Check version
        if !self.version_matches(false) {
            warn!(
                "Config version mismatch: {} | {}",
                self.version.as_deref().unwrap_or("None"),
                crate_version!()
            );
        }
        // Resolve repo
        let repo = match self.repo {
            Some(mut repo) => {
                repo = repo.resolve()?;
                env::set_var("YURT_REPO_LOCAL", &repo.local);
                Some(repo)
            }
            None => None,
        };
        // Resolve build
        let mut build = LinkedList::new();
        if let Some(raw_build) = self.build {
            for set in raw_build {
                build.extend(set.resolve()?);
            }
        }
        Ok(ResolvedConfig {
            version: self.version,
            shell: self.shell,
            repo,
            build,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static YAML: &str = include_str!("../test/build.yaml");

    #[test]
    fn empty_build_fails() {
        assert!(Config::from_str("").is_err())
    }

    #[test]
    fn build_parses() {
        Config::from_str(YAML).unwrap();
    }

    #[test]
    fn build_resolves() {
        let resolved = Config::from_str(YAML).unwrap().resolve().unwrap();
        let mut links = 1;
        let mut boots = 2;
        let mut names = vec![
            "package_0",
            "package_1",
            "package_2",
            "package_3",
            "package_4",
        ]
        .into_iter();
        for unit in resolved.build.into_iter() {
            match unit {
                BuildUnit::Link(_) => links -= 1,
                BuildUnit::Package(pkg) => assert_eq!(pkg.name, names.next().unwrap()),
                BuildUnit::Bootstrap(_) => boots -= 1,
            }
        }
        assert_eq!(links, 0);
        assert_eq!(boots, 0);
        assert!(names.next().is_none());
    }

    #[test]
    fn build_version_check() {
        let mut cfg = Config {
            version: None,
            shell: None,
            repo: None,
            build: None,
        };
        assert!(!cfg.version_matches(true));
        assert!(cfg.version_matches(false));
        cfg.version = Some("9.9.9".to_string());
        assert!(!cfg.version_matches(true));
        assert!(!cfg.version_matches(false));
        cfg.version = Some(crate_version!().to_string());
        assert!(cfg.version_matches(true));
        assert!(cfg.version_matches(false));
    }

    #[test]
    fn positive_match() {
        let set = BuildSet::Case(vec![Case::Positive {
            spec: Locale::new(None, Some(LOCALE.platform.to_string()), None),
            build: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve().unwrap().is_empty());
    }

    #[test]
    fn positive_non_match() {
        let set = BuildSet::Case(vec![Case::Positive {
            spec: Locale::new(None, None, Some("nothere".to_string())),
            build: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve().unwrap().is_empty());
    }

    #[test]
    fn negative_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(None, Some("nothere".to_string()), None),
            build: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve().unwrap().is_empty());
    }

    #[test]
    fn negative_non_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(Some(LOCALE.user.to_string()), None, None),
            build: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve().unwrap().is_empty());
    }
}
