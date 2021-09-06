use super::link::Link;
use super::pack::{Package, PackageBundle, PackageManager, Shell};
use super::repo::Repo;
use anyhow::{anyhow, Result};
use clap::crate_version;
use lazy_static::lazy_static;
use log::warn;
use regex::{Captures, Regex};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

lazy_static! {
    // Matches: "${{ anything here }}"
    static ref RE_OUTER: Regex = Regex::new(r"\$\{\{(?P<inner>[^{}]*)\}\}").unwrap();
    // Matches: "namespace.variable_name"
    static ref RE_INNER: Regex = Regex::new(r"^\s*(?P<namespace>[a-zA-Z_][a-zA-Z_0-9]*)\.(?P<variable>[a-zA-Z_][a-zA-Z_0-9]*)\s*$").unwrap();
}

#[derive(Debug)]
pub struct Context {
    locale: Locale<String>,
    variables: HashMap<String, String>,
    managers: HashSet<PackageManager>,
    home_dir: String,
}

impl Context {
    #[inline]
    fn insert(&mut self, namespace: &str, variable: &str, value: &str) -> Option<String> {
        self.variables
            .insert(format!("{}.{}", namespace, variable), value.to_string())
    }

    pub fn lookup(&self, namespace: &str, variable: &str) -> Result<String> {
        if namespace == "env" {
            Ok(env::var(variable)?)
        } else {
            self.variables
                .get(&format!("{}.{}", namespace, variable))
                .cloned()
                .ok_or_else(|| anyhow!("Variable {}.{} is undefined", namespace, variable))
        }
    }

    pub fn substitute(&self, input: &str) -> Result<String> {
        // Build iterator of replaced values
        let values: Result<Vec<String>> = RE_OUTER
            .captures_iter(input)
            .map(|cap_outer| match RE_INNER.captures(&cap_outer["inner"]) {
                Some(cap_inner) => self.lookup(&cap_inner["namespace"], &cap_inner["variable"]),
                None => Err(anyhow!("Invalid substitution: {}", &cap_outer["inner"])),
            })
            .collect();
        let mut values_iter = values?.into_iter();
        // Build new string with replacements
        Ok(RE_OUTER
            .replace_all(input, |_: &Captures| values_iter.next().unwrap())
            .replace("~", &self.home_dir))
    }
}

impl Default for Context {
    fn default() -> Self {
        Self {
            variables: HashMap::new(),
            locale: Locale::new(
                whoami::username(),
                format!("{:?}", whoami::platform()).to_lowercase(),
                whoami::distro()
                    .split(' ')
                    .next()
                    .expect("Failed to determine distro")
                    .to_owned()
                    .to_lowercase(),
            ),
            managers: PackageManager::all()
                .iter()
                .filter(|pm| pm.is_available())
                .cloned()
                .collect(),
            home_dir: dirs::home_dir()
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("~")
                .to_string(),
        }
    }
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
    pub fn is_local(&self, rubric: &Locale<String>) -> bool {
        let s_vals = vec![
            self.user.as_deref(),
            self.platform.as_deref(),
            self.distro.as_deref(),
        ];
        let o_vals = vec![
            rubric.user.as_str(),
            rubric.platform.as_str(),
            rubric.distro.as_str(),
        ];
        s_vals
            .into_iter()
            .zip(o_vals.into_iter())
            .all(|(s, o)| !matches!(s, Some(val) if val != o))
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case<T> {
    Positive {
        spec: Locale<Option<String>>,
        include: T,
    },
    Negative {
        spec: Locale<Option<String>>,
        include: T,
    },
    Default {
        include: T,
    },
}

impl<T> Case<T> {
    pub fn rule(self, default: bool, rubric: &Locale<String>) -> Option<T> {
        match self {
            Case::Positive { spec, include } if spec.is_local(&rubric) => Some(include),
            Case::Negative { spec, include } if !spec.is_local(&rubric) => Some(include),
            Case::Default { include } if default => Some(include),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum BuildUnit {
    Link(Link),
    ShellCmd(String),
    Package(Package),
    Bootstrap(PackageManager),
}

trait ResolveUnit {
    fn resolve(self, context: &Context) -> Result<BuildUnit>;
}

macro_rules! auto_convert {
    (@impl_try_from BuildUnit::$outer:ident, $inner:ty, $self:ident, $context:ident, $mapped:expr) => {
        impl ResolveUnit for $inner {
            fn resolve($self, $context: &Context) -> Result<BuildUnit> {
                ($mapped).map(BuildUnit::$outer)
            }
        }
    };

    (BuildUnit::$outer:ident, $inner:ty) => {
        auto_convert!(@impl_try_from BuildUnit::$outer, $inner, self, _context, Ok(self));
    };
    (BuildUnit::$outer:ident, $inner:ty, ($a:ident, $b:ident) => $mapped:expr) => {
        auto_convert!(@impl_try_from BuildUnit::$outer, $inner, $a, $b, $mapped);
    };
}

auto_convert!(BuildUnit::Link, Link, (self, context) => self.expand(context));
auto_convert!(BuildUnit::Package, Package);
auto_convert!(BuildUnit::Bootstrap, PackageManager);
auto_convert!(
    BuildUnit::ShellCmd,
    String, (self, context) => context.substitute(&self)
);

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSet {
    Case(Vec<Case<Vec<BuildSet>>>),
    Link(Vec<Link>),
    Run(String),
    Install(Vec<Package>),
    Bundle(PackageBundle),
    Bootstrap(Vec<PackageManager>),
}

trait Resolve {
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>>;
}

impl Resolve for BuildSet {
    // Recursively resolve all case units; collect into single vec
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>> {
        match self {
            Self::Case(v) => v.resolve(context),
            Self::Link(v) => v.resolve(context),
            Self::Run(s) => Ok(vec![s.resolve(context)?]),
            Self::Install(v) => v.resolve(context),
            Self::Bundle(v) => v.resolve(context),
            Self::Bootstrap(v) => v.resolve(context),
        }
    }
}

impl<T> Resolve for Vec<T>
where
    T: ResolveUnit,
{
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>> {
        self.into_iter().map(|u| u.resolve(context)).collect()
    }
}

impl Resolve for Vec<BuildSet> {
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>> {
        let mut units = Vec::new();
        for build in self {
            units.extend(build.resolve(context)?);
        }
        Ok(units)
    }
}

impl Resolve for PackageBundle {
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>> {
        let manager = self.manager;
        self.packages
            .into_iter()
            .map(|name| Package::new(name, vec![manager.clone()]).resolve(context))
            .collect()
    }
}

impl<T> Resolve for Vec<Case<T>>
where
    T: Resolve,
{
    // Process a block of cases
    fn resolve(self, context: &Context) -> Result<Vec<BuildUnit>> {
        let mut default = true;
        let mut units = Vec::new();
        for case in self {
            match case.rule(default, &context.locale) {
                Some(build) => {
                    default = false;
                    units.extend(build.resolve(context)?);
                }
                None => continue,
            };
        }
        Ok(units)
    }
}

#[derive(Debug)]
pub struct ResolvedConfig {
    pub context: Context,
    pub version: Option<String>,
    pub shell: Option<Shell>,
    pub repo: Option<Repo>,
    pub build: Vec<BuildUnit>,
}

impl ResolvedConfig {
    pub fn map_build<RL, RS, RP, RB, E>(
        &self,
        lf: fn(&Link) -> Result<RL, E>,
        sf: fn(&str) -> Result<RS, E>,
        pf: fn(&Package) -> Result<RP, E>,
        bf: fn(&PackageManager) -> Result<RB, E>,
    ) -> Result<(), E> {
        for unit in &self.build {
            match unit {
                BuildUnit::Link(ln) => drop(lf(ln)?),
                BuildUnit::ShellCmd(cmd) => drop(sf(cmd)?),
                BuildUnit::Package(pkg) => drop(pf(pkg)?),
                BuildUnit::Bootstrap(pm) => drop(bf(pm)?),
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Config {
    pub version: Option<String>,
    pub shell: Option<Shell>,
    pub repo: Option<Repo>,
    pub build: Option<Vec<BuildSet>>,
}

impl Config {
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

    pub fn resolve(self) -> Result<ResolvedConfig> {
        let mut context = Context::default();
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
                repo = repo.resolve(&context)?;
                context.insert("repo", "local", &repo.local);
                Some(repo)
            }
            None => None,
        };
        // Resolve build
        let build = match self.build {
            Some(raw) => raw.resolve(&context)?,
            None => Vec::new(),
        };
        Ok(ResolvedConfig {
            context,
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

    fn check_pattern_outer(input: &str, output: &str) {
        let caps = RE_OUTER.captures(input).unwrap();
        assert_eq!(&caps[0], input);
        assert_eq!(&caps["inner"], output);
    }

    fn check_pattern_inner(input: &str, namespace: &str, variable: &str) {
        let caps = RE_INNER.captures(input).unwrap();
        assert_eq!(&caps["namespace"], namespace);
        assert_eq!(&caps["variable"], variable);
    }

    #[test]
    fn substitution_pattern_outer() {
        check_pattern_outer("${{}}", "");
        check_pattern_outer("${{ var }}", " var ");
        check_pattern_outer("${{ env.var }}", " env.var ");
    }

    #[test]
    fn substitution_pattern_inner() {
        check_pattern_inner("   a.b\t ", "a", "b");
        check_pattern_inner("mod_1.var_1", "mod_1", "var_1");
    }

    #[test]
    fn substitute_from_context() {
        let mut expander = Context::default();
        expander.insert("name", "var_1", "val_1");
        expander.insert("name", "var_2", "val_2");
        assert!(!expander.substitute("~").unwrap().is_empty());
        assert_eq!(expander.substitute("${{ name.var_1 }}").unwrap(), "val_1");
        assert_eq!(
            expander
                .substitute("${{ name.var_1 }}/${{ name.var_2 }}")
                .unwrap(),
            "val_1/val_2"
        );
    }

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
        let mut comms = 1;
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
                BuildUnit::ShellCmd(_) => comms -= 1,
                BuildUnit::Package(pkg) => assert_eq!(pkg.name, names.next().unwrap()),
                BuildUnit::Bootstrap(_) => boots -= 1,
            }
        }
        assert_eq!(links, 0);
        assert_eq!(comms, 0);
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
            spec: Locale::new(
                None,
                Some(format!("{:?}", whoami::platform()).to_lowercase()),
                None,
            ),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve(&Context::default()).unwrap().is_empty());
    }

    #[test]
    fn positive_non_match() {
        let set = BuildSet::Case(vec![Case::Positive {
            spec: Locale::new(None, None, Some("nothere".to_string())),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(None, Some("nothere".to_string()), None),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve(&Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_non_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(Some(whoami::username()), None, None),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&Context::default()).unwrap().is_empty());
    }
}
