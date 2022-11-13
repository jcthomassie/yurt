use crate::specs::PackageManager;

use anyhow::{anyhow, Context as _, Result};
use clap::ArgMatches;
use indexmap::IndexSet;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::Entry, HashMap},
    env,
    path::Path,
};

#[derive(Debug, Clone)]
pub struct Context {
    pub locale: Locale,
    pub managers: IndexSet<PackageManager>,
    pub variables: VarStack,
    home_dir: String,
}

impl Context {
    /// Replace '~' with home directory and resolve variables
    pub fn parse_path(&self, input: &str) -> Result<String> {
        self.variables
            .parse_str(input)
            .map(|s| s.replace('~', &self.home_dir))
    }
}

impl From<&ArgMatches> for Context {
    fn from(args: &ArgMatches) -> Self {
        Self {
            locale: Locale::from(args),
            managers: IndexSet::new(),
            variables: VarStack(HashMap::new()),
            home_dir: dirs::home_dir()
                .as_deref()
                .and_then(Path::to_str)
                .unwrap_or("~")
                .to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Locale {
    user: String,
    platform: String,
    distro: String,
}

impl Locale {
    #[inline]
    fn get_user() -> String {
        whoami::username()
    }

    #[inline]
    fn get_platform() -> String {
        format!("{:?}", whoami::platform()).to_lowercase()
    }

    #[inline]
    fn get_distro() -> String {
        whoami::distro()
            .split(' ')
            .next()
            .expect("Failed to determine distro")
            .to_owned()
            .to_lowercase()
    }
}

impl From<&ArgMatches> for Locale {
    fn from(args: &ArgMatches) -> Self {
        Self {
            user: args
                .get_one::<String>("user")
                .map_or_else(Self::get_user, String::from),
            platform: args
                .get_one::<String>("platform")
                .map_or_else(Self::get_platform, String::from),
            distro: args
                .get_one::<String>("distro")
                .map_or_else(Self::get_distro, String::from),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LocaleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
}

impl LocaleSpec {
    pub fn is_local(&self, context: &Context) -> bool {
        match self {
            LocaleSpec { user: Some(u), .. } if u != &context.locale.user => false,
            LocaleSpec {
                platform: Some(p), ..
            } if p != &context.locale.platform => false,
            LocaleSpec {
                distro: Some(d), ..
            } if d != &context.locale.distro => false,
            _ => true,
        }
    }
}

type Key = String;

#[derive(Debug, Clone)]
pub struct VarStack(HashMap<String, Vec<String>>);

impl VarStack {
    /// Replaces patterns of the form "${{ namespace.key }}"
    pub fn parse_str(&self, input: &str) -> Result<String> {
        lazy_static! {
            static ref RE_OUTER: Regex = Regex::new(r"\$\{\{(?P<inner>[^{}]*)\}\}").unwrap();
            static ref RE_INNER: Regex = Regex::new(r"^\s*(?P<namespace>[a-zA-Z_][a-zA-Z_0-9]*)\.(?P<variable>[a-zA-Z_][a-zA-Z_0-9]*)\s*$").unwrap();
        }
        // Build iterator of replaced values
        let values: Result<Vec<String>> = RE_OUTER
            .captures_iter(input)
            .map(|outer| match RE_INNER.captures(&outer["inner"]) {
                Some(inner) => self.get(&inner["namespace"], &inner["variable"]),
                None => Err(anyhow!("Invalid substitution: {}", &outer["inner"])),
            })
            .collect();
        let mut values_iter = values?.into_iter();
        // Build new string with replacements
        Ok(RE_OUTER
            .replace_all(input, |_: &Captures| values_iter.next().unwrap())
            .to_string())
    }

    #[inline]
    pub fn key<N: AsRef<str>, K: AsRef<str>>(namespace: N, variable: K) -> Key {
        format!("{}.{}", namespace.as_ref(), variable.as_ref())
    }

    pub fn get<N: AsRef<str>, K: AsRef<str>>(&self, namespace: N, variable: K) -> Result<String> {
        let key = Self::key(namespace.as_ref(), variable.as_ref());
        match self.get_raw(&key) {
            Some(value) => Ok(value.clone()),
            None if namespace.as_ref() == "env" => env::var(variable.as_ref()).with_context(|| {
                format!("Failed to get environment variable: {}", variable.as_ref())
            }),
            None => Err(anyhow!("Variable {} is undefined", key)),
        }
    }

    pub fn push<K: AsRef<str>, V: Into<String>>(
        &mut self,
        namespace: &str,
        items: impl Iterator<Item = (K, V)>,
    ) {
        for (key, val) in items {
            self.push_raw(Self::key(namespace, key), val.into());
        }
    }

    pub fn drop<N: AsRef<str>, K: AsRef<str>>(
        &mut self,
        namespace: N,
        keys: impl Iterator<Item = K>,
    ) {
        for key in keys {
            self.drop_raw(Self::key(namespace.as_ref(), key));
        }
    }

    fn get_raw(&self, key: &Key) -> Option<&String> {
        self.0.get(key).and_then(|vec| vec.last())
    }

    fn push_raw(&mut self, key: Key, val: String) {
        self.0.entry(key).or_default().push(val);
    }

    fn drop_raw(&mut self, key: Key) {
        if let Entry::Occupied(mut entry) = self.0.entry(key) {
            let vec = entry.get_mut();
            let _val = vec.pop();
            if vec.is_empty() {
                entry.remove();
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::yurt_command;
    use pretty_assertions::assert_eq;

    #[inline]
    pub fn get_context(args: &[&str]) -> Context {
        Context::from(&yurt_command().get_matches_from(args))
    }

    #[test]
    fn override_user() {
        let locale = get_context(&["yurt", "--override-user", "yurt-test-user"]).locale;
        assert_eq!(locale.user, "yurt-test-user");
    }

    #[test]
    fn override_distro() {
        let locale = get_context(&["yurt", "--override-distro", "yurt-test-distro"]).locale;
        assert_eq!(locale.distro, "yurt-test-distro");
    }

    #[test]
    fn override_platform() {
        let locale = get_context(&["yurt", "--override-platform", "yurt-test-platform"]).locale;
        assert_eq!(locale.platform, "yurt-test-platform");
    }

    #[test]
    fn locale_matching() {
        let context = get_context(&[
            "yurt",
            "--override-user=u",
            "--override-distro=d",
            "--override-platform=p",
        ]);
        let cases = [
            ("{}", true),
            ("{ user: u }", true),
            ("{ distro: d }", true),
            ("{ user: u, distro: d }", true),
            ("{ user: u, distro: d, platform: p }", true),
            ("{ user: _ }", false),
            ("{ platform: _ }", false),
            ("{ user: u, distro: _ }", false),
            ("{ user: u, distro: d, platform: _ }", false),
        ];
        for (yaml, result) in cases {
            let locale: LocaleSpec = serde_yaml::from_str(yaml).expect("Deserialization failed");
            assert_eq!(locale.is_local(&context), result);
        }
    }

    #[test]
    fn path_variable_sub() {
        let mut context = get_context(&[]);
        context
            .variables
            .push("name", [("var_1", "val_1"), ("var_2", "val_2")].into_iter());
        assert!(!context.parse_path("~").unwrap().is_empty());
        assert_eq!(context.parse_path("${{ name.var_1 }}").unwrap(), "val_1");
        assert_eq!(
            context
                .parse_path("${{ name.var_1 }}/${{ name.var_2 }}")
                .unwrap(),
            "val_1/val_2"
        );
    }

    #[test]
    fn variable_sub_invalid() {
        let mut context = get_context(&[]);
        context.variables.push("a", [("b", "c")].into_iter());
        assert!(context.variables.parse_str("${{ a.b.c }}").is_err());
        assert!(context.variables.parse_str("${{ . }}").is_err());
        assert!(context.variables.parse_str("${{ a. }}").is_err());
        assert!(context.variables.parse_str("${{ .b }}").is_err());
        assert!(context.variables.parse_str("${{ a.c }}").is_err()); // missing key
        assert!(context.variables.parse_str("${{ b.a }}").is_err()); // missing namespace
    }
}
