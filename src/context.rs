use crate::specs::PackageManager;
use crate::YurtArgs;

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Context {
    pub locale: Locale,
    pub managers: IndexMap<String, PackageManager>,
    pub variables: parse::KeyStack,
    home_dir: String,
}

impl Context {
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            managers: IndexMap::new(),
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
    pub fn matches(&self, locale: &Locale) -> bool {
        match self {
            Self { user: Some(u), .. } if u != &locale.user => false,
            Self {
                platform: Some(p), ..
            } if p != &locale.platform => false,
            Self {
                distro: Some(d), ..
            } if d != &locale.distro => false,
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
                (?P<var>\w+)|
                env:(?P<envvar>\w+)|
                (?P<object>\w+)(?:\#(?P<id>\w+))?\.(?P<attr>\w+)
            )\s*$"
        )
        .unwrap();
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub enum Key {
        Var(String),    // ${{ var }}
        EnvVar(String), // ${{ env:var }}
        ObjectAttr {
            object: String,
            attr: String,
        }, // ${{ object.attr }}
        ObjectInstanceAttr {
            object: String,
            id: String,
            attr: String,
        }, // ${{ object#id.attr }}
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
            let capture = |key: &str| captures.name(key).map(|val| val.as_str().to_string());

            if let Some(var) = capture("var") {
                Ok(Self::Var(var))
            } else if let Some(envvar) = capture("envvar") {
                Ok(Self::EnvVar(envvar))
            } else if let (Some(object), Some(attr)) = (capture("object"), capture("attr")) {
                if let Some(id) = capture("id") {
                    Ok(Self::ObjectInstanceAttr { object, attr, id })
                } else {
                    Ok(Self::ObjectAttr { object, attr })
                }
            } else {
                unreachable!("RE_KEY regex is malformed")
            }
        }
    }

    pub trait ObjectKey {
        const OBJECT_NAME: &'static str;

        /// Create a `Key::ObjectAttr` using `Self::OBJECT_NAME`
        fn object_key<A>(attr: A) -> Key
        where
            A: Into<String>,
        {
            Key::ObjectAttr {
                object: Self::OBJECT_NAME.into(),
                attr: attr.into(),
            }
        }

        /// Create a `Key::ObjectInstanceAttr` using `Self::OBJECT_NAME`
        fn object_instance_key<A, I>(attr: A, id: I) -> Key
        where
            A: Into<String>,
            I: Into<String>,
        {
            Key::ObjectInstanceAttr {
                object: Self::OBJECT_NAME.into(),
                id: id.into(),
                attr: attr.into(),
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
            self.get(key).map_or_else(|| key.get(), Ok)
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
        fn key_env_var() {
            assert_eq!(
                Key::EnvVar("key_1".to_string()),
                Key::try_from("env:key_1").unwrap()
            );
        }

        #[test]
        fn key_object_attr() {
            assert_eq!(
                Key::ObjectAttr {
                    object: "obj_1".to_string(),
                    attr: "attr_1".to_string()
                },
                Key::try_from("obj_1.attr_1").unwrap()
            );
        }

        #[test]
        fn key_object_instance_attr() {
            assert_eq!(
                Key::ObjectInstanceAttr {
                    object: "obj_1".to_string(),
                    id: "id_1".to_string(),
                    attr: "attr_1".to_string()
                },
                Key::try_from("obj_1#id_1.attr_1").unwrap()
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

        #[test]
        fn key_get_envvar() {
            assert!(Key::EnvVar("key".to_string()).get().is_err());
            ::std::env::set_var("key", "value");
            assert_eq!(Key::EnvVar("key".to_string()).get().unwrap(), "value");
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
                Key::Var(var) => Ok(format!("{var}_output")),
                _ => panic!("{key:?}"),
            },
            "${{ key }}",
            "key_output"
        );

        test_replace!(
            single_env_var,
            |key| match key {
                Key::EnvVar(var) => Ok(format!("{var}_output")),
                _ => panic!("{key:?}"),
            },
            "${{ env:key }}",
            "key_output"
        );

        test_replace!(
            single_object_attr,
            |key| match key {
                Key::ObjectAttr { object, attr } => Ok(format!("{object}_{attr}_output")),
                _ => panic!("{key:?}"),
            },
            "${{ ns.key }}",
            "ns_key_output"
        );

        test_replace!(
            mixed,
            |key| match key {
                Key::Var(var) => Ok(format!("Var_{var}")),
                Key::EnvVar(var) => Ok(format!("EnvVar_{var}")),
                Key::ObjectAttr { object, attr } => Ok(format!("ObjectAttr_{object}.{attr}")),
                Key::ObjectInstanceAttr { object, id, attr } =>
                    Ok(format!("ObjectInstanceAttr_{object}#{id}.{attr}")),
            },
            "${{ key1 }} ${{ env:key2 }} ${{ obj.key3 }} ${{ obj#id.key4 }}",
            "Var_key1 EnvVar_key2 ObjectAttr_obj.key3 ObjectInstanceAttr_obj#id.key4"
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
        let locale = Locale::with_overrides(
            Some("u".to_string()),
            Some("p".to_string()),
            Some("d".to_string()),
        );
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
            let spec: LocaleSpec = serde_yaml::from_str(yaml).expect("Deserialization failed");
            assert_eq!(spec.matches(&locale), result);
        }
    }

    #[test]
    fn parse_str() {
        let mut context = Context::default();
        context.variables.try_push("key", "value").unwrap();
        context.variables.try_push("ns.key", "ns_value").unwrap();
        ::std::env::set_var("key", "env_value");
        assert_eq!(context.parse_str("${{ key }}").unwrap(), "value");
        assert_eq!(context.parse_str("${{ ns.key }}").unwrap(), "ns_value");
        assert_eq!(context.parse_str("${{ env:key }}").unwrap(), "env_value");
    }

    #[test]
    fn parse_str_invalid() {
        let mut context = Context::default();
        context.variables.try_push("a.b", "value").unwrap();
        assert!(context.parse_str("${{ a.b.c }}").is_err());
        assert!(context.parse_str("${{ . }}").is_err());
        assert!(context.parse_str("${{ a. }}").is_err());
        assert!(context.parse_str("${{ .b }}").is_err());
        assert!(context.parse_str("${{ a.c }}").is_err()); // missing key
        assert!(context.parse_str("${{ b.a }}").is_err()); // missing namespace
    }

    #[test]
    fn parse_path() {
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
