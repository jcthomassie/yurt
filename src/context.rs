use crate::specs::PackageManager;
use crate::YurtArgs;

use anyhow::{anyhow, Context as _, Result};
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
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            managers: IndexSet::new(),
            variables: VarStack(HashMap::new()),
            home_dir: dirs::home_dir()
                .as_deref()
                .and_then(Path::to_str)
                .unwrap_or("~")
                .to_string(),
        }
    }

    /// Replace '~' with home directory and resolve variables
    pub fn parse_path(&self, input: &str) -> Result<String> {
        self.variables
            .parse_str(input)
            .map(|s| s.replace('~', &self.home_dir))
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new(Locale::default())
    }
}

impl From<&YurtArgs> for Context {
    fn from(args: &YurtArgs) -> Self {
        Self::new(Locale::from(args))
    }
}

#[derive(Debug, Clone)]
pub struct Locale {
    user: String,
    platform: String,
    distro: String,
}

impl Locale {
    pub fn with_overrides(
        user: Option<String>,
        platform: Option<String>,
        distro: Option<String>,
    ) -> Self {
        Self {
            user: user.unwrap_or_else(Self::get_user),
            platform: platform.unwrap_or_else(Self::get_platform),
            distro: distro.unwrap_or_else(Self::get_distro),
        }
    }

    #[inline]
    fn get_user() -> String {
        whoami::username()
    }

    #[inline]
    fn get_platform() -> String {
        whoami::platform().to_string().to_lowercase()
    }

    #[inline]
    fn get_distro() -> String {
        whoami::distro()
            .split(' ')
            .next()
            .unwrap()
            .to_string()
            .to_lowercase()
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::with_overrides(None, None, None)
    }
}

impl From<&YurtArgs> for Locale {
    fn from(args: &YurtArgs) -> Self {
        Self::with_overrides(
            args.override_user.clone(),
            args.override_platform.clone(),
            args.override_distro.clone(),
        )
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
            Self { user: Some(u), .. } if u != &context.locale.user => false,
            Self {
                platform: Some(p), ..
            } if p != &context.locale.platform => false,
            Self {
                distro: Some(d), ..
            } if d != &context.locale.distro => false,
            _ => true,
        }
    }
}

mod parse {
    use anyhow::{anyhow, Context as _, Result};
    use lazy_static::lazy_static;
    use regex::{Captures, Regex};
    use std::collections::hash_map::{Entry, HashMap};

    lazy_static! {
        static ref RE_REPLACE: Regex = Regex::new(
            r"(?x)
            \$\{\{\s*(?:
                (?:(?P<namespace>\w+)\.)?(?P<var>\w+)|
                env:(?P<envvar>\w+)|
                (?P<invalid>.*)
            )\s*\}\}"
        )
        .unwrap();
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub enum Key {
        Var(String),                  // ${{ var }}
        EnvVar(String),               // ${{ env:var }}
        NamespaceVar(String, String), // ${{ namespace.var }}
    }

    impl Key {
        pub fn get(&self) -> Result<String> {
            match self {
                Self::EnvVar(var) => ::std::env::var(var)
                    .with_context(|| format!("Failed to get environment variable: {var}")),
                _ => Err(anyhow!("Failed to get key: {:?}", self)),
            }
        }
    }

    // Only intended for use in this module
    impl TryFrom<&Captures<'_>> for Key {
        type Error = anyhow::Error;

        fn try_from(captures: &Captures) -> anyhow::Result<Self> {
            let cap_var = captures.name("var");
            let cap_envvar = captures.name("envvar");
            let cap_namespace = captures.name("namespace");
            let cap_invalid = captures.name("invalid");

            if let Some(var) = cap_var {
                if let Some(name) = cap_namespace {
                    Ok(Self::NamespaceVar(
                        name.as_str().to_string(),
                        var.as_str().to_string(),
                    ))
                } else {
                    Ok(Self::Var(var.as_str().to_string()))
                }
            } else if let Some(envvar) = cap_envvar {
                Ok(Self::EnvVar(envvar.as_str().to_string()))
            } else if let Some(invalid) = cap_invalid {
                Err(anyhow!("Invalid substitution: {}", invalid.as_str()))
            } else {
                Err(anyhow!("Key was initialized from invalid capture"))
            }
        }
    }

    pub struct KeyStack(HashMap<Key, Vec<String>>);

    impl KeyStack {
        pub fn new() -> Self {
            Self(HashMap::new())
        }

        /// Replace substitution tags in `input` using currently set values.
        /// Uses `Key::get()` as a fallback if the key is unset.
        pub fn replace(&self, input: &str) -> Result<String> {
            replace(input, |key| match self.get(&key) {
                Some(val) => Ok(val),
                None => key.get(),
            })
        }

        /// Get the last value for `key` from the stack
        fn get(&self, key: &Key) -> Option<String> {
            self.0.get(key).and_then(|vec| vec.last()).cloned()
        }

        /// Push a new value for `key` onto the stack
        fn push<V>(&mut self, key: Key, val: V)
        where
            V: Into<String>,
        {
            self.0.entry(key).or_default().push(val.into());
        }

        /// Drop the last value for `key` from the stack
        fn drop(&mut self, key: Key) {
            if let Entry::Occupied(mut entry) = self.0.entry(key) {
                let vec = entry.get_mut();
                let _val = vec.pop();
                if vec.is_empty() {
                    entry.remove();
                }
            }
        }
    }

    // TODO: make private
    pub fn replace<F>(input: &str, f: F) -> Result<String>
    where
        F: Fn(Key) -> Result<String>,
    {
        // Build iterator of replaced values
        let values: Result<Vec<String>> = RE_REPLACE
            .captures_iter(input)
            .map(|caps| Key::try_from(&caps).and_then(&f))
            .collect();
        let mut values_iter = values?.into_iter();
        // Build new string with replacements
        Ok(RE_REPLACE
            .replace_all(input, |_: &Captures| values_iter.next().unwrap())
            .to_string())
    }
}

type Key = String;

#[derive(Debug, Clone)]
pub struct VarStack(HashMap<String, Vec<String>>);

impl VarStack {
    const ENV_NAMESPACE: &str = "env";

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
        let var = variable.as_ref();
        match namespace.as_ref() {
            Self::ENV_NAMESPACE => {
                env::var(var).with_context(|| format!("Failed to get environment variable: {var}"))
            }
            other => {
                let key = Self::key(other, var);
                self.get_raw(&key)
                    .cloned()
                    .with_context(|| format!("Variable {} is undefined", &key))
            }
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
    use crate::YurtArgs;
    use clap::Parser;
    use pretty_assertions::assert_eq;

    mod parse {
        use super::super::parse::{replace, Key};

        macro_rules! test_replace {
            ($name:ident, $f:expr, $input:literal, $output:literal) => {
                #[test]
                fn $name() {
                    let real_output = replace($input, $f).expect("`replace` call returned error");
                    pretty_assertions::assert_eq!(real_output, $output);
                }
            };
        }

        test_replace!(
            single_var,
            |key| match key {
                Key::Var(key) => Ok(format!("{key}_output")),
                _ => panic!(),
            },
            "${{ key }}",
            "key_output"
        );

        test_replace!(
            single_envvar,
            |key| match key {
                Key::EnvVar(key) => Ok(format!("{key}_output")),
                _ => panic!(),
            },
            "${{ env:key }}",
            "key_output"
        );

        test_replace!(
            single_namespacevar,
            |key| match key {
                Key::NamespaceVar(ns, key) => Ok(format!("{ns}_{key}_output")),
                _ => panic!(),
            },
            "${{ ns.key }}",
            "ns_key_output"
        );

        test_replace!(
            mixed,
            |key| match key {
                Key::Var(key) => Ok(format!("Var_{key}")),
                Key::EnvVar(key) => Ok(format!("EnvVar_{key}")),
                Key::NamespaceVar(ns, key) => Ok(format!("NamespaceVar_{ns}.{key}")),
            },
            "${{ key1 }} ${{ env:key2 }} ${{ namespace.key3 }}",
            "Var_key1 EnvVar_key2 NamespaceVar_namespace.key3"
        );

        test_replace!(
            no_replacements,
            |_key| { panic!() },
            "literal input",
            "literal input"
        );
    }

    #[inline]
    fn parse_locale(args: &[&str]) -> Locale {
        Locale::from(&YurtArgs::try_parse_from(args).expect("Failed to parse args"))
    }

    #[test]
    fn override_user() {
        let locale = parse_locale(&["yurt", "--override-user", "yurt-test-user", "show"]);
        assert_eq!(locale.user, "yurt-test-user");
    }

    #[test]
    fn override_distro() {
        let locale = parse_locale(&["yurt", "--override-distro", "yurt-test-distro", "show"]);
        assert_eq!(locale.distro, "yurt-test-distro");
    }

    #[test]
    fn override_platform() {
        let locale = parse_locale(&["yurt", "--override-platform", "yurt-test-platform", "show"]);
        assert_eq!(locale.platform, "yurt-test-platform");
    }

    #[test]
    fn locale_matching() {
        let context = Context::new(Locale::with_overrides(
            Some("u".to_string()),
            Some("p".to_string()),
            Some("d".to_string()),
        ));
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
        let mut context = Context::default();
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
        let mut context = Context::default();
        context.variables.push("a", [("b", "c")].into_iter());
        assert!(context.variables.parse_str("${{ a.b.c }}").is_err());
        assert!(context.variables.parse_str("${{ . }}").is_err());
        assert!(context.variables.parse_str("${{ a. }}").is_err());
        assert!(context.variables.parse_str("${{ .b }}").is_err());
        assert!(context.variables.parse_str("${{ a.c }}").is_err()); // missing key
        assert!(context.variables.parse_str("${{ b.a }}").is_err()); // missing namespace
    }
}
