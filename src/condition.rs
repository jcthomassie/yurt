use crate::{
    build::{BuildUnit, Context, ResolveInto},
    shell::ShellCommand,
};
use anyhow::Result;
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use std::ops::Not;

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
                .value_of("user")
                .map_or_else(Self::get_user, String::from),
            platform: args
                .value_of("platform")
                .map_or_else(Self::get_platform, String::from),
            distro: args
                .value_of("distro")
                .map_or_else(Self::get_distro, String::from),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct LocaleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
}

impl LocaleSpec {
    fn is_local(&self, context: &Context) -> bool {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
enum Condition {
    Bool(bool),
    Locale(LocaleSpec),
    Command(ShellCommand),
}

impl Condition {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        match self {
            Self::Bool(literal) => Ok(*literal),
            Self::Locale(spec) => Ok(spec.is_local(context)),
            Self::Command(command) => command.run_bool(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all(deserialize = "snake_case"))]
enum CaseBranch<T> {
    Positive { condition: Condition, include: T },
    Negative { condition: Condition, include: T },
    Default { include: T },
}

impl<T> CaseBranch<T> {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        match self {
            Self::Positive { condition, .. } => condition.evaluate(context),
            Self::Negative { condition, .. } => condition.evaluate(context).map(Not::not),
            Self::Default { .. } => Ok(true),
        }
    }

    fn unpack(self) -> T {
        match self {
            Self::Positive { include, .. }
            | Self::Negative { include, .. }
            | Self::Default { include } => include,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Case<T>(Vec<CaseBranch<T>>);

impl<T> ResolveInto for Case<T>
where
    T: ResolveInto,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        for case in self.0 {
            if case.evaluate(context)? {
                return case.unpack().resolve_into(context, output);
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::tests::get_context;

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

    macro_rules! yaml_condition {
        ($yaml:expr, $enum_pattern:pat, $evaluation:literal) => {
            let cond: Condition = serde_yaml::from_str($yaml).expect("Deserialization failed");
            assert!(matches!(cond, $enum_pattern));
            assert_eq!(cond.evaluate(&get_context(&[])).unwrap(), $evaluation);
        };
    }

    #[test]
    fn locale_condition() {
        let user_locale = format!("{{ user: {} }}", whoami::username());
        yaml_condition!(user_locale.as_str(), Condition::Locale(_), true);
        yaml_condition!("{ platform: fake }", Condition::Locale(_), false);
    }

    #[test]
    fn command_condition() {
        yaml_condition!(r#""echo 'hello'""#, Condition::Command(_), true);
        yaml_condition!("bad-command -a -b", Condition::Command(_), false);
    }

    #[test]
    fn bool_condition() {
        yaml_condition!("true", Condition::Bool(true), true);
        yaml_condition!("false", Condition::Bool(false), false);
    }

    #[test]
    fn positive_match() {
        let context = get_context(&[]);
        let case = CaseBranch::Positive {
            condition: Condition::Bool(true),
            include: "something",
        };
        assert!(case.evaluate(&context).unwrap());
    }

    #[test]
    fn positive_non_match() {
        let context = get_context(&[]);
        let case = CaseBranch::Positive {
            condition: Condition::Bool(false),
            include: "something",
        };
        assert!(!case.evaluate(&context).unwrap());
    }

    #[test]
    fn negative_match() {
        let context = get_context(&[]);
        let case = CaseBranch::Negative {
            condition: Condition::Bool(false),
            include: "something",
        };
        assert!(case.evaluate(&context).unwrap());
    }

    #[test]
    fn negative_non_match() {
        let context = get_context(&[]);
        let case = CaseBranch::Negative {
            condition: Condition::Bool(true),
            include: "something",
        };
        assert!(!case.evaluate(&context).unwrap());
    }
}
