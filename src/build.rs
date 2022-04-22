use crate::condition::{Case, Locale, LocaleSpec};
use crate::files::Link;
use crate::package::{Package, PackageManager};
use crate::repo::Repo;
use crate::shell::{Shell, ShellSpec};
use anyhow::{anyhow, bail, ensure, Context as AnyContext, Result};
use clap::{crate_version, ArgMatches};
use lazy_static::lazy_static;
use log::{info, warn};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
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
    pub locale: Locale,
    variables: BTreeMap<String, String>,
    managers: BTreeSet<PackageManager>,
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
            None if namespace == "env" => env::var(variable)
                .with_context(|| format!("Failed to get environment variable: {}", variable)),
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
                '~',
                self.home_dir
                    .to_str()
                    .ok_or_else(|| anyhow!("Invalid home directory: {:?}", self.home_dir))?,
            ))
    }
}

impl From<&ArgMatches> for Context {
    fn from(args: &ArgMatches) -> Self {
        Self {
            variables: BTreeMap::new(),
            locale: Locale::from(args),
            managers: BTreeSet::new(),
            home_dir: dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")),
        }
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

#[derive(Debug, PartialEq, Clone)]
pub enum BuildUnit {
    Repo(Repo),
    Link(Link),
    Run(String),
    Install(Package),
    Require(PackageManager),
}

impl BuildUnit {
    pub const ALL_NAMES: &'static [&'static str] = &["repo", "link", "run", "install", "require"];
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    Repo(Repo),
    Namespace(Namespace),
    Matrix(Matrix<Vec<BuildSpec>>),
    Case(Vec<Case<LocaleSpec, Vec<BuildSpec>>>),
    Link(Vec<Link>),
    Run(String),
    Install(Vec<Package>),
    Require(Vec<PackageManager>),
}

impl BuildSpec {
    fn absorb(self: &mut BuildSpec, unit: &BuildUnit) -> bool {
        match (self, unit) {
            (BuildSpec::Link(a), BuildUnit::Link(b)) => {
                a.push(b.clone());
                true
            }
            (BuildSpec::Install(a), BuildUnit::Install(b)) => {
                a.push(b.clone());
                true
            }
            (BuildSpec::Require(a), BuildUnit::Require(b)) => {
                a.push(*b);
                true
            }
            _ => false,
        }
    }
}

pub trait Resolve {
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
resolve_unit!(String, (self, context) => BuildUnit::Run(context.replace_variables(&self)?));
resolve_unit!(Package, (self, context) => {
    BuildUnit::Install(Package {
        name: context.replace_variables(&self.name)?,
        managers: match self.managers.is_empty() {
            false => context.managers.intersection(&self.managers).copied().collect(),
            true => context.managers.clone()
        },
        ..self
    })
});
resolve_unit!(PackageManager, (self, context) => {
    context.managers.insert(self);
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

    #[inline]
    fn _include(unit: &BuildUnit, units: &HashSet<String>) -> bool {
        match unit {
            BuildUnit::Repo(_) => units.contains("repo"),
            BuildUnit::Link(_) => units.contains("link"),
            BuildUnit::Install(_) => units.contains("install"),
            BuildUnit::Require(_) => units.contains("require"),
            BuildUnit::Run(_) => units.contains("run"),
        }
    }

    fn include(self, units: &HashSet<String>) -> Self {
        self.filter(|unit| Self::_include(unit, units))
    }

    fn exclude(self, units: &HashSet<String>) -> Self {
        self.filter(|unit| !Self::_include(unit, units))
    }

    // Pretty-print the complete build; optionally filter out trivial units
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
                BuildUnit::Run(cmd) => drop(self.shell.run(cmd.as_str())?),
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
            version: self.version,
            shell: Some(self.shell.into()),
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
                if let Some(units) = args.values_of("include") {
                    r.include(&units.map(String::from).collect())
                } else if let Some(units) = args.values_of("exclude") {
                    r.exclude(&units.map(String::from).collect())
                } else {
                    r
                }
            })
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub version: Option<String>,
    pub shell: Option<ShellSpec>,
    pub build: Vec<BuildSpec>,
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

    pub fn resolve(self, mut context: Context) -> Result<ResolvedConfig> {
        // Check version
        if !self.version_matches(false) {
            warn!(
                "Config version mismatch: {} | {}",
                self.version.as_deref().unwrap_or("None"),
                crate_version!()
            );
        }
        // Resolve build
        Ok(ResolvedConfig {
            version: self.version,
            shell: self.shell.map_or_else(Shell::from_env, Shell::from),
            build: self.build.resolve(&mut context)?,
            context,
        })
    }
}

impl TryFrom<&ArgMatches> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &ArgMatches) -> Result<Self> {
        if let Some(url) = args.value_of("yaml-url") {
            Self::from_url(url).context("Failed to parse remote build file")
        } else {
            let path = match args.value_of("yaml") {
                Some(path) => Ok(path.to_string()),
                None => env::var("YURT_BUILD_FILE"),
            }
            .context("Config file not specified")?;
            Self::from_path(path).context("Failed to parse local build file")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yurt_command;

    fn get_context(args: &[&str]) -> Context {
        Context::from(&yurt_command().get_matches_from(args))
    }

    mod yaml {
        use super::*;

        #[test]
        fn version_check() {
            let mut cfg = Config {
                version: None,
                shell: None,
                build: Vec::new(),
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

        mod args {
            use super::*;
            use pretty_assertions::assert_eq;
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
                let extra = read_to_string(dir.join("args")).expect("Failed to read args");
                let mut args = vec!["yurt", "--yaml", &input];
                args.extend(extra.split(' '));
                ResolvedConfig::try_from(&yurt_command().get_matches_from(args))
                    .expect("Failed to resolve input build")
            }

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let dir = get_test_path(&["args", stringify!($name)]);
                        let resolved = get_resolved(&dir);
                        let raw_output = read_to_string(&dir.join("output.yaml"))
                            .expect("Failed to read output");
                        let yaml = resolved.into_yaml().unwrap();
                        assert_eq!(yaml, raw_output)
                    }
                };
            }

            test_case!(exclude);
            test_case!(include);
        }

        mod io {
            use super::super::get_context;
            use super::Config;
            use pretty_assertions::assert_eq;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let raw_input =
                            include_str!(concat!("../test/io/", stringify!($name), "/input.yaml"));
                        let raw_output =
                            include_str!(concat!("../test/io/", stringify!($name), "/output.yaml"));
                        let config =
                            Config::from_str(raw_input).expect("failed to parse input case");
                        let resolved = config
                            .resolve(get_context(&[]))
                            .expect("failed to resolve input case");
                        let yaml = resolved.into_yaml().unwrap();
                        assert_eq!(yaml, raw_output)
                    }
                };
            }

            test_case!(packages);
            test_case!(packages_expanded);
            test_case!(matrix);
            test_case!(namespace);
            test_case!(case);
            test_case!(repo);
        }

        mod invalid_parse {
            use super::Config;

            macro_rules! test_case {
                ($name:ident) => {
                    #[test]
                    fn $name() {
                        let raw_input = include_str!(concat!(
                            "../test/invalid/parse/",
                            stringify!($name),
                            ".yaml"
                        ));
                        assert!(Config::from_str(raw_input).is_err())
                    }
                };
            }

            test_case!(empty);
            test_case!(no_build);
        }
    }

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
        let mut context = get_context(&[]);
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
        let spec: Package = serde_yaml::from_str("name: ${{ namespace.key }}").unwrap();
        let mut context = get_context(&[]);
        context.set_variable("namespace", "key", "value");
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(package.name, "value");
    }

    #[test]
    fn package_manager_prune_empty() {
        let spec: Package = serde_yaml::from_str("name: some-package").unwrap();
        let mut context = get_context(&[]);
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert!(package.managers.is_empty());
    }

    #[test]
    fn package_manager_prune_some() {
        #[rustfmt::skip]
        let spec: Package = serde_yaml::from_str("
            name: some-package
            managers: [ apt, brew ]
        ").unwrap();
        // Add partially overlapping managers
        let mut context = get_context(&[]);
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
        let mut context = get_context(&[]);
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
        let mut context = get_context(&[]);
        #[rustfmt::skip]
        let matrix: Matrix<Vec<String>> = serde_yaml::from_str("
            values:
              a: [1, 2, 3]
              b: [4, 5, 6, 7]
            include: [ ]
        ").unwrap();
        assert!(matrix.resolve(&mut context).is_err());
    }

    #[test]
    fn matrix_expansion() {
        let mut context = get_context(&[]);
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
}
