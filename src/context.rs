use crate::specs::PackageManager;
use crate::YurtArgs;

use anyhow::Result;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Context {
    pub locale: Locale,
    pub managers: IndexSet<PackageManager>,
    pub variables: parse::KeyStack,
    home_dir: String,
}

impl Context {
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            managers: IndexSet::new(),
            variables: parse::KeyStack::new(),
            home_dir: dirs::home_dir()
                .as_deref()
                .and_then(Path::to_str)
                .unwrap_or("~")
                .to_string(),
        }
    }

    pub fn parse_str(&self, input: &str) -> Result<String> {
        parse::replace(input, |key| self.variables.try_get(&key))
    }

    /// Replace '~' with home directory and resolve variables
    pub fn parse_path(&self, input: &str) -> Result<String> {
        parse::replace(input, |key| self.variables.try_get(&key))
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

pub mod parse {
    use anyhow::{anyhow, Context as _, Result};
    use lazy_static::lazy_static;
    use regex::{Captures, Regex};
    use std::collections::HashMap;

    lazy_static! {
        static ref RE_KEY_WRAPPER: Regex = Regex::new(r"\$\{\{(?P<key>[^{}]*)\}\}").unwrap();
        static ref RE_KEY: Regex = Regex::new(
            r"(?x)^\s*(?:
                (?:(?P<namespace>\w+)\.)?(?P<var>\w+)|
                env:(?P<envvar>\w+)
            )\s*$"
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

    impl TryFrom<&str> for Key {
        type Error = anyhow::Error;

        fn try_from(s: &str) -> anyhow::Result<Self> {
            let captures = RE_KEY
                .captures(s)
                .with_context(|| format!("Invalid key format: {s}"))?;
            if let Some(var) = captures.name("var") {
                if let Some(name) = captures.name("namespace") {
                    Ok(Self::NamespaceVar(
                        name.as_str().to_string(),
                        var.as_str().to_string(),
                    ))
                } else {
                    Ok(Self::Var(var.as_str().to_string()))
                }
            } else if let Some(envvar) = captures.name("envvar") {
                Ok(Self::EnvVar(envvar.as_str().to_string()))
            } else {
                unreachable!("RE_KEY regex is malformed")
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct KeyStack(HashMap<Key, Vec<String>>);

    impl KeyStack {
        pub fn new() -> Self {
            Self(HashMap::new())
        }

        #[cfg(test)]
        pub fn try_push<K, V>(&mut self, key: K, val: V) -> Result<()>
        where
            K: TryInto<Key, Error = anyhow::Error>,
            V: Into<String>,
        {
            self.push(key.try_into()?, val.into());
            Ok(())
        }

        /// Get the last value for `key` from the stack.
        /// Uses `Key::get()` as a fallback if unset.
        pub fn try_get(&self, key: &Key) -> Result<String> {
            match self.get(key) {
                Some(val) => Ok(val),
                None => key.get(),
            }
        }

        /// Get the last value for `key` from the stack
        pub fn get(&self, key: &Key) -> Option<String> {
            self.0.get(key).and_then(|vec| vec.last()).cloned()
        }

        /// Push a new value for `key` onto the stack
        pub fn push(&mut self, key: Key, val: String) {
            self.0.entry(key).or_default().push(val);
        }

        /// Drop the last value for `key` from the stack
        pub fn drop(&mut self, key: &Key) {
            if let Some(vec) = self.0.get_mut(key) {
                vec.pop();
                if vec.is_empty() {
                    self.0.remove(key);
                }
            }
        }
    }

    /// Replace keys in `input` by mapping with `f`
    pub fn replace<F>(input: &str, f: F) -> Result<String>
    where
        F: Fn(Key) -> Result<String>,
    {
        // Build iterator of replaced values
        let values: Result<Vec<String>> = RE_KEY_WRAPPER
            .captures_iter(input)
            .map(|caps| Key::try_from(&caps["key"]).and_then(&f))
            .collect();
        let mut values_iter = values?.into_iter();
        // Build new string with replacements
        Ok(RE_KEY_WRAPPER
            .replace_all(input, |_: &Captures| values_iter.next().unwrap())
            .to_string())
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

        #[test]
        fn key_var() {
            assert_eq!(
                Key::Var("key_1".to_string()),
                Key::try_from("key_1").unwrap()
            );
        }

        #[test]
        fn key_envvar() {
            assert_eq!(
                Key::EnvVar("key_1".to_string()),
                Key::try_from("env:key_1").unwrap()
            );
        }

        #[test]
        fn key_namespace() {
            assert_eq!(
                Key::NamespaceVar("ns_1".to_string(), "key_1".to_string()),
                Key::try_from("ns_1.key_1").unwrap()
            );
        }

        #[test]
        fn key_invalid() {
            assert!(Key::try_from("").is_err());
            assert!(Key::try_from(" ").is_err());
            assert!(Key::try_from("key.").is_err());
            assert!(Key::try_from("key-").is_err());
            assert!(Key::try_from("{key}").is_err());
        }

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
                _ => panic!("{key:?}"),
            },
            "${{ key }}",
            "key_output"
        );

        test_replace!(
            single_envvar,
            |key| match key {
                Key::EnvVar(key) => Ok(format!("{key}_output")),
                _ => panic!("{key:?}"),
            },
            "${{ env:key }}",
            "key_output"
        );

        test_replace!(
            single_namespacevar,
            |key| match key {
                Key::NamespaceVar(ns, key) => Ok(format!("{ns}_{key}_output")),
                _ => panic!("{key:?}"),
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
            |key| panic!("{key:?}"),
            "literal input",
            "literal input"
        );

        test_replace!(
            no_whitespace,
            |_| Ok("output".to_string()),
            "${{key}}",
            "output"
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
        context.variables.try_push("var_1", "val_1").unwrap();
        context.variables.try_push("var_2", "val_2").unwrap();
        assert!(!context.parse_path("~").unwrap().is_empty());
        assert_eq!(context.parse_path("${{ var_1 }}").unwrap(), "val_1");
        assert_eq!(
            context.parse_path("${{ var_1 }}/${{ var_2 }}").unwrap(),
            "val_1/val_2"
        );
    }
}
