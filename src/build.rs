use super::files::Link;
use super::package::{Package, PackageManager};
use super::repo::Repo;
use super::shell::{Shell, ShellCmd};
use anyhow::{anyhow, bail, ensure, Result};
use clap::crate_version;
use lazy_static::lazy_static;
use log::{info, warn};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
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
struct Context {
    variables: BTreeMap<String, String>,
    managers: BTreeSet<PackageManager>,
    locale: Locale,
    home_dir: PathBuf,
}

impl Context {
    #[inline]
    fn set_variable(&mut self, namespace: &str, variable: &str, value: &str) -> Option<String> {
        self.variables
            .insert(format!("{}.{}", namespace, variable), value.to_string())
    }

    fn get_variable(&self, namespace: &str, variable: &str) -> Result<String> {
        match self.variables.get(&format!("{}.{}", namespace, variable)) {
            Some(value) => Ok(value.clone()),
            None if namespace == "env" => Ok(env::var(variable)?),
            None => Err(anyhow!("Variable {}.{} is undefined", namespace, variable)),
        }
    }

    fn replace_variables(&self, input: &str) -> Result<String> {
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

#[derive(Debug, PartialEq, Clone)]
pub struct Locale {
    user: String,
    platform: String,
    distro: String,
}

impl Locale {
    fn new(user: String, platform: String, distro: String) -> Self {
        Self {
            user,
            platform,
            distro,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct LocaleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
}

impl LocaleSpec {
    fn is_local(&self, rubric: &Locale) -> bool {
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

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Namespace {
    name: String,
    values: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Matrix<T> {
    values: BTreeMap<String, Vec<String>>,
    include: T,
}

impl<T> Matrix<T> {
    fn length(&self) -> Result<usize> {
        let mut counts = self.values.values().map(Vec::len);
        match counts.next() {
            Some(len) => {
                ensure!(counts.all(|next| next == len), "Matrix array size mismatch");
                Ok(len)
            }
            None => bail!("Matrix values must be non-empty"),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case<T> {
    Positive { spec: LocaleSpec, include: T },
    Negative { spec: LocaleSpec, include: T },
    Default { include: T },
}

impl<T> Case<T> {
    fn rule(self, default: bool, rubric: &Locale) -> Option<T> {
        match self {
            Case::Positive { spec, include } if spec.is_local(rubric) => Some(include),
            Case::Negative { spec, include } if !spec.is_local(rubric) => Some(include),
            Case::Default { include } if default => Some(include),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct PackageSpec {
    name: String,
    managers: Option<BTreeSet<PackageManager>>,
    aliases: Option<BTreeMap<PackageManager, String>>,
}

impl From<Package> for PackageSpec {
    fn from(package: Package) -> PackageSpec {
        PackageSpec {
            name: package.name,
            managers: Some(package.managers).filter(|set| !set.is_empty()),
            aliases: Some(package.aliases).filter(|map| !map.is_empty()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
enum BuildUnit {
    Repo(Repo),
    Link(Link),
    ShellCmd(String),
    Install(Package),
    Require(PackageManager),
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    Repo(Repo),
    Namespace(Namespace),
    Matrix(Matrix<Vec<BuildSpec>>),
    Case(Vec<Case<Vec<BuildSpec>>>),
    Link(Vec<Link>),
    Run(String),
    Install(Vec<PackageSpec>),
    Require(Vec<PackageManager>),
}

impl BuildSpec {
    fn absorb(self: &mut BuildSpec, other: &mut BuildSpec) -> bool {
        match (self, other) {
            (BuildSpec::Link(a), BuildSpec::Link(b)) => {
                a.append(b);
                true
            }
            (BuildSpec::Install(a), BuildSpec::Install(b)) => {
                a.append(b);
                true
            }
            (BuildSpec::Require(a), BuildSpec::Require(b)) => {
                a.append(b);
                true
            }
            _ => false,
        }
    }
}

trait Resolve {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()>;

    fn resolve(self, context: &mut Context) -> Result<Vec<BuildUnit>>
    where
        Self: Sized,
    {
        let mut output = Vec::new();
        self.resolve_into(context, &mut output)?;
        Ok(output)
    }
}

macro_rules! resolve_unit {
    ($type:ty, ($self:ident, $context:ident) => $mapped:expr) => {
        impl Resolve for $type {
            fn resolve_into($self, $context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
                output.push($mapped);
                Ok(())
            }
        }
    };
}

resolve_unit!(Link, (self, context) => {
    BuildUnit::Link(Link::new(
        context.replace_variables(self.head.to_str().unwrap())?,
        context.replace_variables(self.tail.to_str().unwrap())?,
    ))
});
resolve_unit!(String, (self, context) => BuildUnit::ShellCmd(context.replace_variables(&self)?));
resolve_unit!(PackageSpec, (self, context) => {
    BuildUnit::Install(Package {
        name: context.replace_variables(&self.name)?,
        managers: match &self.managers {
            Some(m) => context.managers.intersection(m).cloned().collect(),
            None => context.managers.clone()
        },
        aliases: self.aliases.unwrap_or_default(),
    })
});
resolve_unit!(PackageManager, (self, context) => {
    context.managers.insert(self.clone());
    BuildUnit::Require(self)
});
resolve_unit!(Repo, (self, context) => {
    let path = context.replace_variables(&self.path)?;
    let new = Repo { path, ..self };
    let name = new.path.split('/').last().ok_or_else(|| anyhow!("Repo local path is empty"))?;
    context.set_variable(name, "path", &new.path);
    BuildUnit::Repo(new)
});

impl<T> Resolve for Vec<T>
where
    T: Resolve,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        for raw in self {
            raw.resolve_into(context, output)?;
        }
        Ok(())
    }
}

impl Resolve for BuildSpec {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        match self {
            Self::Repo(r) => r.resolve_into(context, output)?,
            Self::Namespace(n) => n.resolve_into(context, output)?,
            Self::Matrix(m) => m.resolve_into(context, output)?,
            Self::Case(v) => v.resolve_into(context, output)?,
            Self::Link(v) => v.resolve_into(context, output)?,
            Self::Run(s) => s.resolve_into(context, output)?,
            Self::Install(v) => v.resolve_into(context, output)?,
            Self::Require(v) => v.resolve_into(context, output)?,
        }
        Ok(())
    }
}

impl Resolve for Namespace {
    fn resolve_into(self, context: &mut Context, _output: &mut Vec<BuildUnit>) -> Result<()> {
        for (variable, value) in &self.values {
            context.set_variable(&self.name, variable, value);
        }
        Ok(())
    }
}

impl<T> Resolve for Matrix<T>
where
    T: Resolve + Clone,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        let len = self.length()?;
        let mut iters: Vec<_> = self.values.iter().map(|(k, v)| (k, v.iter())).collect();
        let mut context = context.clone();
        for _ in 0..len {
            for (variable, values) in &mut iters {
                // Iterator size has been validated as count; unwrap here is safe
                let value = &context.replace_variables(values.next().unwrap())?;
                context.set_variable("matrix", variable, value);
            }
            self.include.clone().resolve_into(&mut context, output)?;
        }
        Ok(())
    }
}

impl<T> Resolve for Vec<Case<T>>
where
    T: Resolve,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        let mut default = true;
        for case in self {
            match case.rule(default, &context.locale) {
                Some(build) => {
                    default = false;
                    build.resolve_into(context, output)?;
                }
                None => continue,
            };
        }
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    // Members should be treated as immutable
    context: Context,
    version: Option<String>,
    shell: Shell,
    build: Vec<BuildUnit>,
}

impl ResolvedConfig {
    fn nontrivial(&self) -> Vec<&BuildUnit> {
        self.build
            .iter()
            .filter(|&unit| match unit {
                BuildUnit::Repo(repo) => !repo.is_available(),
                BuildUnit::Link(link) => !link.is_valid(),
                BuildUnit::Install(package) => !package.is_installed(),
                BuildUnit::Require(manager) => !manager.is_available(),
                BuildUnit::ShellCmd(_) => true,
            })
            .collect()
    }

    // Pretty-print the complete build; optionally filter out trivial units
    pub fn show(&self, nontrivial: bool) -> Result<()> {
        if nontrivial {
            println!("{:#?}", self.nontrivial());
        } else {
            print!("{}", serde_yaml::to_string(&self.clone().into_config())?);
        }
        Ok(())
    }

    // Eliminate elements that will conflict with installation
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
                BuildUnit::ShellCmd(cmd) => drop(cmd.as_str().run(&self.shell)?),
                BuildUnit::Install(package) => package.install()?,
                BuildUnit::Require(manager) => manager.require()?,
            }
        }
        Ok(())
    }

    pub fn uninstall(&self, packages: bool) -> Result<()> {
        info!("Uninstalling...");
        for unit in &self.build {
            match unit {
                BuildUnit::Link(link) => link.unlink()?,
                BuildUnit::Install(package) if packages => package.uninstall()?,
                _ => continue,
            }
        }
        Ok(())
    }

    pub fn update(&self) -> Result<()> {
        todo!()
    }

    fn into_config(self) -> yaml::Config {
        let mut build: Vec<BuildSpec> = Vec::new();
        for unit in self.build.into_iter() {
            let mut next = match unit {
                BuildUnit::Repo(repo) => BuildSpec::Repo(repo),
                BuildUnit::Link(link) => BuildSpec::Link(vec![link]),
                BuildUnit::ShellCmd(cmd) => BuildSpec::Run(cmd),
                BuildUnit::Install(package) => BuildSpec::Install(vec![package.into()]),
                BuildUnit::Require(manager) => BuildSpec::Require(vec![manager]),
            };
            if let Some(spec) = build.last_mut() {
                if spec.absorb(&mut next) {
                    continue;
                }
            }
            build.push(next);
        }
        yaml::Config {
            version: self.version,
            shell: Some(self.shell),
            build: Some(build).filter(|spec| !spec.is_empty()),
        }
    }
}

pub mod yaml {
    use super::*;

    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    pub struct Config {
        pub version: Option<String>,
        pub shell: Option<Shell>,
        pub build: Option<Vec<BuildSpec>>,
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
            // Resolve build
            let build = match self.build {
                Some(raw) => raw.resolve(&mut context)?,
                None => Vec::new(),
            };
            Ok(ResolvedConfig {
                context,
                version: self.version,
                shell: self.shell.unwrap_or_else(Shell::from_env),
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
        let mut repos = 1;
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
                BuildUnit::Repo(_) => repos -= 1,
                BuildUnit::Link(link) => assert_eq!(link.tail, links.next().unwrap()),
                BuildUnit::ShellCmd(_) => comms -= 1,
                BuildUnit::Install(package) => assert_eq!(package.name, names.next().unwrap()),
                BuildUnit::Require(_) => boots -= 1,
            }
        }
        assert_eq!(repos, 0);
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

    macro_rules! unpack {
        (@unit $value:expr, BuildUnit::$variant:ident) => {
            if let BuildUnit::$variant(ref unwrapped) = $value {
                unwrapped
            } else {
                panic!("Failed to unpack build unit");
            }
        };
        (@unit_vec $value:expr, BuildUnit::$variant:ident) => {
            {
                assert_eq!($value.len(), 1);
                unpack!(@unit ($value)[0], BuildUnit::$variant)
            }
        };
    }

    #[test]
    fn package_name_substitution() {
        let spec: PackageSpec = serde_yaml::from_str("name: ${{ namespace.key }}").unwrap();
        let mut context = Context::default();
        context.set_variable("namespace", "key", "value");
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(package.name, "value");
    }

    #[test]
    fn package_manager_prune_empty() {
        let spec: PackageSpec = serde_yaml::from_str("name: some-package").unwrap();
        let mut context = Context::default();
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert!(package.managers.is_empty());
    }

    #[test]
    fn package_manager_prune_some() {
        #[rustfmt::skip]
        let spec: PackageSpec = serde_yaml::from_str("
            name: some-package
            managers: [ apt, brew ]
        ").unwrap();
        // Add partially overlapping managers
        let mut context = Context::default();
        context.managers.insert(PackageManager::Cargo);
        context.managers.insert(PackageManager::Brew);
        // Overlap remains
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(
            package.managers,
            vec![PackageManager::Brew].into_iter().collect()
        );
    }

    #[test]
    fn namespace_resolves() {
        #[rustfmt::skip]
        let namespace: Namespace = serde_yaml::from_str("
            name: namespace
            values:
              key_a: val_a
              key_b: val_b
        ").unwrap();
        let mut context = Context::default();
        namespace.resolve(&mut context).unwrap();
        assert_eq!(context.get_variable("namespace", "key_a").unwrap(), "val_a");
        assert_eq!(context.get_variable("namespace", "key_b").unwrap(), "val_b");
    }

    #[test]
    fn matrix_length() {
        #[rustfmt::skip]
        let matrix: Matrix<Vec<String>> = serde_yaml::from_str("
            values:
              a: [1, 2, 3]
              b: [4, 5, 6]
            include: [ ]
        ").unwrap();
        assert_eq!(matrix.length().unwrap(), 3);
    }

    #[test]
    fn matrix_array_mismatch() {
        #[rustfmt::skip]
        let matrix: Matrix<Vec<String>> = serde_yaml::from_str("
            values:
              a: [1, 2, 3]
              b: [4, 5, 6, 7]
            include: [ ]
        ").unwrap();
        assert!(matrix.resolve(&mut Context::default()).is_err());
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
        let set = BuildSpec::Case(vec![Case::Positive {
            spec: LocaleSpec {
                user: None,
                platform: Some(format!("{:?}", whoami::platform()).to_lowercase()),
                distro: None,
            },
            include: vec![BuildSpec::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn positive_non_match() {
        let set = BuildSpec::Case(vec![Case::Positive {
            spec: LocaleSpec {
                user: None,
                platform: None,
                distro: Some("nothere".to_string()),
            },
            include: vec![BuildSpec::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_match() {
        let set = BuildSpec::Case(vec![Case::Negative {
            spec: LocaleSpec {
                user: None,
                platform: Some("nothere".to_string()),
                distro: None,
            },
            include: vec![BuildSpec::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(!set.resolve(&mut Context::default()).unwrap().is_empty());
    }

    #[test]
    fn negative_non_match() {
        let set = BuildSpec::Case(vec![Case::Negative {
            spec: LocaleSpec {
                user: Some(whoami::username()),
                platform: None,
                distro: None,
            },
            include: vec![BuildSpec::Link(vec![Link::new("a", "b")])],
        }]);
        assert!(set.resolve(&mut Context::default()).unwrap().is_empty());
    }
}
