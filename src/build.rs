use crate::{
    condition::{Case, Locale},
    link::Link,
    package::{Package, PackageManager},
    repo::Repo,
    shell::ShellCommand,
};
use anyhow::{anyhow, bail, Context as AnyContext, Result};
use clap::{crate_version, ArgMatches};
use indexmap::{IndexMap, IndexSet};
use lazy_static::lazy_static;
use log::{info, warn};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::File,
    io::BufReader,
    path::Path,
};

#[derive(Debug, Clone)]
pub struct Context {
    pub locale: Locale,
    pub managers: IndexSet<PackageManager>,
    variables: HashMap<String, String>,
    home_dir: String,
}

impl Context {
    #[inline]
    pub fn set_variable(&mut self, namespace: &str, key: &str, value: &str) -> Option<String> {
        self.variables
            .insert(format!("{}.{}", namespace, key), value.to_string())
    }

    pub fn get_variable(&self, namespace: &str, key: &str) -> Result<String> {
        match self.variables.get(&format!("{}.{}", namespace, key)) {
            Some(value) => Ok(value.clone()),
            None if namespace == "env" => env::var(key)
                .with_context(|| format!("Failed to get environment variable: {}", key)),
            None => Err(anyhow!("Variable {}.{} is undefined", namespace, key)),
        }
    }

    /// Replaces patterns of the form "${{ namespace.key }}"
    pub fn replace_variables(&self, input: &str) -> Result<String> {
        lazy_static! {
            static ref RE_OUTER: Regex = Regex::new(r"\$\{\{(?P<inner>[^{}]*)\}\}").unwrap();
            static ref RE_INNER: Regex = Regex::new(r"^\s*(?P<namespace>[a-zA-Z_][a-zA-Z_0-9]*)\.(?P<variable>[a-zA-Z_][a-zA-Z_0-9]*)\s*$").unwrap();
        }
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
            .replace('~', &self.home_dir))
    }
}

impl From<&ArgMatches> for Context {
    fn from(args: &ArgMatches) -> Self {
        Self {
            locale: Locale::from(args),
            managers: IndexSet::new(),
            variables: HashMap::new(),
            home_dir: dirs::home_dir()
                .as_deref()
                .and_then(Path::to_str)
                .unwrap_or("~")
                .to_string(),
        }
    }
}

pub trait Resolve {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit>;
}

pub trait ResolveInto {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()>;

    fn resolve_into_new(self, context: &mut Context) -> Result<Vec<BuildUnit>>
    where
        Self: Sized,
    {
        let mut output = Vec::new();
        self.resolve_into(context, &mut output)?;
        Ok(output)
    }
}

impl<T> ResolveInto for T
where
    T: Resolve,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        output.push(self.resolve(context)?);
        Ok(())
    }
}

impl<T> ResolveInto for Vec<T>
where
    T: ResolveInto,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        for inner in self {
            inner.resolve_into(context, output)?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Namespace {
    name: String,
    values: IndexMap<String, String>,
}

impl ResolveInto for Namespace {
    fn resolve_into(self, context: &mut Context, _output: &mut Vec<BuildUnit>) -> Result<()> {
        for (variable, value) in &self.values {
            context.set_variable(&self.name, variable, value);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct Matrix<T> {
    values: IndexMap<String, Vec<String>>,
    include: T,
}

impl<T> Matrix<T> {
    fn length(&self) -> Result<usize> {
        let mut counts = self.values.values().map(Vec::len);
        match counts.next() {
            Some(first) => counts
                .all(|next| next == first)
                .then(|| first)
                .context("Matrix array size mismatch"),
            None => bail!("Matrix values must be non-empty"),
        }
    }
}

impl<T> ResolveInto for Matrix<T>
where
    T: ResolveInto + Clone,
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

#[derive(Debug, PartialEq, Clone)]
pub enum BuildUnit {
    Repo(Repo),
    Link(Link),
    Run(ShellCommand),
    Install(Package),
    Require(PackageManager),
}

impl BuildUnit {
    pub const ALL_NAMES: &'static [&'static str] = &["repo", "link", "run", "install", "require"];
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum BuildSpec {
    Repo(Repo),
    Namespace(Namespace),
    Matrix(Matrix<Vec<BuildSpec>>),
    Case(Case<Vec<BuildSpec>>),
    Link(Vec<Link>),
    Run(ShellCommand),
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

impl ResolveInto for BuildSpec {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        match self {
            Self::Repo(r) => r.resolve_into(context, output),
            Self::Namespace(n) => n.resolve_into(context, output),
            Self::Matrix(m) => m.resolve_into(context, output),
            Self::Case(v) => v.resolve_into(context, output),
            Self::Link(v) => v.resolve_into(context, output),
            Self::Run(s) => s.resolve_into(context, output),
            Self::Install(v) => v.resolve_into(context, output),
            Self::Require(v) => v.resolve_into(context, output),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    // Members should be treated as immutable
    context: Context,
    version: Option<String>,
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
            version: self.version,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    version: Option<String>,
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

    #[inline]
    fn version_matches(&self, strict: bool) -> bool {
        match self.version {
            Some(ref v) => v == crate_version!(),
            None => !strict,
        }
    }

    fn resolve(self, mut context: Context) -> Result<ResolvedConfig> {
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
            build: self.build.resolve_into_new(&mut context)?,
            context,
        })
    }
}

impl TryFrom<&ArgMatches> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &ArgMatches) -> Result<Self> {
        if let Some(url) = args.value_of("yaml-url") {
            Self::from_url(url)
        } else {
            Self::from_path(match args.value_of("yaml") {
                Some(path) => path.to_string(),
                None => env::var("YURT_BUILD_FILE").context("Build file not specified")?,
            })
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::yurt_command;

    #[inline]
    pub fn get_context(args: &[&str]) -> Context {
        Context::from(&yurt_command().get_matches_from(args))
    }

    mod yaml {
        use super::*;

        #[test]
        fn version_check() {
            let mut cfg = Config {
                version: None,
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

        mod io {
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
        }
    }

    #[test]
    fn variable_sub() {
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

    #[test]
    fn variable_sub_invalid() {
        let mut context = get_context(&[]);
        context.set_variable("a", "b", "c");
        assert!(context.replace_variables("${{ a.b.c }}").is_err());
        assert!(context.replace_variables("${{ . }}").is_err());
        assert!(context.replace_variables("${{ a. }}").is_err());
        assert!(context.replace_variables("${{ .b }}").is_err());
        assert!(context.replace_variables("${{ a.c }}").is_err()); // missing key
        assert!(context.replace_variables("${{ b.a }}").is_err()); // missing namespace
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
        namespace.resolve_into_new(&mut context).unwrap();
        assert_eq!(context.get_variable("namespace", "key_a").unwrap(), "val_a");
        assert_eq!(context.get_variable("namespace", "key_b").unwrap(), "val_b");
    }

    #[test]
    fn matrix_length() {
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str("
            values:
              a: [1, 2, 3]
              b: [4, 5, 6]
            include: [ ]
        ").unwrap();
        assert_eq!(matrix.length().unwrap(), 3);
    }

    #[test]
    fn matrix_length_mismatch() {
        let mut context = get_context(&[]);
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str("
            values:
              a: [1, 2, 3]
              b: [4, 5, 6, 7]
            include: [ ]
        ").unwrap();
        assert!(matrix.resolve_into_new(&mut context).is_err());
    }

    #[test]
    fn matrix_expansion() {
        let mut context = get_context(&[]);
        context.set_variable("outer", "key", "value");
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str(r#"
            values:
              inner:
                - "${{ outer.key }}_a"
                - "${{ outer.key }}_b"
                - "${{ outer.key }}_c"
            include:
              - run: "${{ matrix.inner }}"
        "#).unwrap();
        #[rustfmt::skip]
        let values: Vec<BuildSpec> = serde_yaml::from_str(r#"
            - run: value_a
            - run: value_b
            - run: value_c
        "#).unwrap();
        assert_eq!(
            matrix.resolve_into_new(&mut context).unwrap(),
            values.resolve_into_new(&mut context).unwrap()
        );
    }
}
