use super::files::Link;
use super::repo::Repo;
use super::shell::{Package, PackageBundle, PackageManager, Shell, ShellCmd};
use anyhow::{anyhow, Result};
use clap::crate_version;
use lazy_static::lazy_static;
use log::{info, warn};
use regex::{Captures, Regex};
use serde::Deserialize;
use std::process::Output;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

lazy_static! {
    // Matches: "${{ anything here }}"
    static ref RE_OUTER: Regex = Regex::new(r"\$\{\{(?P<inner>[^{}]*)\}\}").unwrap();
    // Matches: "namespace.variable_name"
    static ref RE_INNER: Regex = Regex::new(r"^\s*(?P<namespace>[a-zA-Z_][a-zA-Z_0-9]*)\.(?P<variable>[a-zA-Z_][a-zA-Z_0-9]*)\s*$").unwrap();
}

#[derive(Debug, Clone)]
pub struct Context {
    variables: BTreeMap<String, String>,
    pub managers: BTreeSet<PackageManager>,
    pub locale: Locale<String>,
    pub home_dir: PathBuf,
}

impl Context {
    #[inline]
    fn set_variable(&mut self, namespace: &str, variable: &str, value: &str) -> Option<String> {
        self.variables
            .insert(format!("{}.{}", namespace, variable), value.to_string())
    }

    pub fn get_variable(&self, namespace: &str, variable: &str) -> Result<String> {
        if namespace == "env" {
            Ok(env::var(variable)?)
        } else {
            self.variables
                .get(&format!("{}.{}", namespace, variable))
                .cloned()
                .ok_or_else(|| anyhow!("Variable {}.{} is undefined", namespace, variable))
        }
    }

    pub fn replace_variables(&self, input: &str) -> Result<String> {
        // Build iterator of replaced values
        let values: Result<Vec<String>> = RE_OUTER
            .captures_iter(input)
            .map(|outer| match RE_INNER.captures(&outer["inner"]) {
                Some(inner) => self.get_variable(&inner["namespace"], &inner["variable"]),
                None => Err(anyhow!("Invalid substitution: {}", &outer["inner"])),
            })
            .collect();
        let mut values_iter = values?.into_iter();
        // Build new string with replacements
        Ok(RE_OUTER
            .replace_all(input, |_: &Captures| values_iter.next().unwrap())
            .replace(
                "~",
                self.home_dir
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid home directory: {:?}", self.home_dir))?,
            ))
    }
}

impl Default for Context {
    fn default() -> Self {
        Self {
            variables: BTreeMap::new(),
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
            managers: BTreeSet::new(),
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
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

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct Matrix<T> {
    values: BTreeMap<String, Vec<String>>,
    include: T,
}

impl<T> Matrix<T> {
    // Number of expanded elements; returns Err if value counts do not match
    pub fn length(&self) -> Result<usize> {
        let counts: Vec<usize> = self.values.values().map(Vec::len).collect();
        if counts.is_empty() {
            return Err(anyhow!("Matrix values must be non-empty"));
        }
        if counts.windows(2).any(|w| w[1] != w[0]) {
            return Err(anyhow!("Matrix array length mismatch"));
        }
        Ok(counts[0])
    }

    // Transpose into nested vec of key, value pairs
    pub fn transpose(&self) -> Result<Vec<Vec<(&str, &str)>>> {
        let mut groups: Vec<Vec<(&str, &str)>> = std::iter::repeat_with(Vec::new)
            .take(self.length()?)
            .collect();
        for (key, vals) in self.values.iter() {
            for (i, val) in vals.iter().enumerate() {
                groups[i].push((key, val));
            }
        }
        Ok(groups)
    }
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
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
            Case::Positive { spec, include } if spec.is_local(rubric) => Some(include),
            Case::Negative { spec, include } if !spec.is_local(rubric) => Some(include),
            Case::Default { include } if default => Some(include),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum BuildUnit {
    Link(Link),
    ShellCmd(String),
    Install(Package),
    Require(PackageManager),
}

trait ResolveUnit {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit>;
}

macro_rules! auto_convert {
    (@impl_try_from BuildUnit::$outer:ident, $inner:ty, $self:ident, $context:ident, $mapped:expr) => {
        impl ResolveUnit for $inner {
            fn resolve($self, $context: &mut Context) -> Result<BuildUnit> {
                ($mapped).map(BuildUnit::$outer)
            }
        }
    };

    (BuildUnit::$outer:ident($inner:ty)) => {
        auto_convert!(@impl_try_from BuildUnit::$outer, $inner, self, _context, Ok(self));
    };
    (BuildUnit::$outer:ident($inner:ty), ($a:ident, $b:ident) => $mapped:expr) => {
        auto_convert!(@impl_try_from BuildUnit::$outer, $inner, $a, $b, $mapped);
    };
}

auto_convert!(BuildUnit::Link(Link), (self, context) => self.replace_variables(context));
auto_convert!(BuildUnit::ShellCmd(String), (self, context) => context.replace_variables(&self));
auto_convert!(BuildUnit::Install(Package), (self, context) => self.replace_variables(context).map(|p| p.prune(context)));
auto_convert!(BuildUnit::Require(PackageManager), (self, context) => {
    context.managers.insert(self.clone());
    Ok(self)
});

type Build = Vec<BuildSet>;

#[derive(Debug, PartialEq, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSet {
    Matrix(Matrix<Build>),
    Case(Vec<Case<Build>>),
    Link(Vec<Link>),
    Run(String),
    Install(Vec<Package>),
    Bundle(PackageBundle),
    Require(Vec<PackageManager>),
}

trait Resolve {
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>>;
}

impl Resolve for BuildSet {
    // Recursively resolve all case units; collect into single vec
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
        match self {
            Self::Matrix(m) => m.resolve(context),
            Self::Case(v) => v.resolve(context),
            Self::Link(v) => v.resolve(context),
            Self::Run(s) => Ok(vec![s.resolve(context)?]),
            Self::Install(v) => v.resolve(context),
            Self::Bundle(v) => v.resolve(context),
            Self::Require(v) => v.resolve(context),
        }
    }
}

impl<T> Resolve for Vec<T>
where
    T: ResolveUnit,
{
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
        self.into_iter().map(|u| u.resolve(context)).collect()
    }
}

impl Resolve for Build {
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
        let mut units = Vec::new();
        for build in self {
            units.extend(build.resolve(context)?);
        }
        Ok(units)
    }
}

impl Resolve for PackageBundle {
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
        let manager = self.manager.clone();
        self.packages
            .into_iter()
            .map(|name| {
                Package::new(name, Some(manager.clone()).into_iter().collect()).resolve(context)
            })
            .collect()
    }
}

impl<T> Resolve for Matrix<T>
where
    T: Resolve + Clone,
{
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
        let groups = self.transpose()?;
        let mut context = context.clone();
        let mut units = Vec::with_capacity(groups.len() * groups[0].len());
        for map in groups {
            for (key, val) in map {
                context.set_variable("matrix", key, &context.replace_variables(val)?);
            }
            units.extend(self.include.clone().resolve(&mut context)?);
        }
        Ok(units)
    }
}

impl<T> Resolve for Vec<Case<T>>
where
    T: Resolve,
{
    // Process a block of cases
    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>> {
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
    // Members should be treated as immutable
    context: Context,
    version: Option<String>,
    shell: Option<Shell>,
    repo: Option<Repo>,
    build: Vec<BuildUnit>,
}

impl ResolvedConfig {
    // Eliminate elements that will conflict with installation
    pub fn clean(&self) -> Result<()> {
        info!("Cleaning link heads...");
        for unit in &self.build {
            match unit {
                BuildUnit::Link(ln) => ln.clean()?,
                _ => continue,
            }
        }
        Ok(())
    }

    pub fn install(&mut self, clean: bool) -> Result<()> {
        if let Some(repo) = &self.repo {
            repo.require()?;
        }
        if clean {
            self.clean()?;
        }
        info!("Starting build steps...");
        for unit in &self.build {
            match unit {
                BuildUnit::Link(ln) => ln.link()?,
                BuildUnit::ShellCmd(cmd) => drop(cmd.as_str().run()?),
                BuildUnit::Install(pkg) => pkg.install()?,
                BuildUnit::Require(pm) => pm.require()?,
            }
        }
        if let Some(shell) = &self.shell {
            shell.chsh()?;
        }
        Ok(())
    }

    pub fn uninstall(&self, packages: bool) -> Result<()> {
        if packages {
            info!("Uninstalling dotfiles and packages...");
        } else {
            info!("Uninstalling dotfiles...");
        }
        for unit in &self.build {
            match unit {
                BuildUnit::Link(ln) => ln.unlink()?,
                BuildUnit::Install(pkg) if packages => pkg.uninstall()?,
                _ => continue,
            }
        }
        Ok(())
    }

    pub fn edit(&self, editor: &str) -> Result<Output> {
        let path = &self
            .repo
            .as_ref()
            .ok_or_else(|| anyhow!("Dotfile repo root is not set"))?
            .local;
        format!("{} {}", editor, path).as_str().run()
    }
}

pub mod yaml {
    use super::*;

    #[derive(Debug, PartialEq, Deserialize)]
    pub struct Config {
        pub version: Option<String>,
        pub shell: Option<Shell>,
        pub repo: Option<Repo>,
        pub build: Option<Build>,
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
                    repo = repo.resolve(&mut context)?;
                    context.set_variable("repo", "local", &repo.local);
                    Some(repo)
                }
                None => None,
            };
            // Resolve build
            let build = match self.build {
                Some(raw) => raw.resolve(&mut context)?,
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
}

#[cfg(test)]
mod tests {
    use super::{yaml::*, *};

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
    fn replace_from_context() {
        let mut context = Context::default();
        context.set_variable("name", "var_1", "val_1");
        context.set_variable("name", "var_2", "val_2");
        assert!(!context.replace_variables("~").unwrap().is_empty());
        assert_eq!(
            context.replace_variables("${{ name.var_1 }}").unwrap(),
            "val_1"
        );
        assert_eq!(
            context
                .replace_variables("${{ name.var_1 }}/${{ name.var_2 }}")
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
        let mut comms = 1;
        let mut boots = 2;
        let mut links = vec!["dir_a/tail_1", "dir_b/tail_2", "dir_c/tail_3"]
            .into_iter()
            .map(std::path::PathBuf::from);
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
                BuildUnit::Link(ln) => assert_eq!(ln.tail, links.next().unwrap()),
                BuildUnit::ShellCmd(_) => comms -= 1,
                BuildUnit::Install(pkg) => assert_eq!(pkg.name, names.next().unwrap()),
                BuildUnit::Require(_) => boots -= 1,
            }
        }
        assert!(links.next().is_none());
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
    fn matrix_expansion() {
        let mut context = Context {
            locale: Locale::new(String::new(), String::new(), String::new()),
            variables: BTreeMap::new(),
            managers: BTreeSet::new(),
            home_dir: PathBuf::new(),
        };
        context.set_variable("outer", "key", "value");
        let values = vec![
            "${{ outer.key }}_a".to_string(),
            "${{ outer.key }}_b".to_string(),
            "${{ outer.key }}_c".to_string(),
        ];
        let matrix = Matrix {
            values: {
                let mut map = BTreeMap::new();
                map.insert("key".to_string(), values.clone());
                map
            },
            include: vec!["${{ matrix.key }}".to_string()],
        };
        assert_eq!(
            matrix.resolve(&mut context).unwrap(),
            values.resolve(&mut context).unwrap()
        );
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
        assert!(!set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn positive_non_match() {
        let set = BuildSet::Case(vec![Case::Positive {
            spec: Locale::new(None, None, Some("nothere".to_string())),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(None, Some("nothere".to_string()), None),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_non_match() {
        let set = BuildSet::Case(vec![Case::Negative {
            spec: Locale::new(Some(whoami::username()), None, None),
            include: vec![BuildSet::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&mut Context::default()).unwrap().is_empty());
    }
}