use crate::{
    build::{BuildUnit, Context, ResolveInto},
    shell::ShellCommand,
};
use anyhow::Result;
use clap::ArgMatches;
use serde::{Deserialize, Serialize};

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
    Locale(LocaleSpec),
    Command(ShellCommand),
    Bool(bool),
}

impl Condition {
    fn evaluate(&self, context: &Context) -> bool {
        match self {
            Self::Locale(spec) => spec.is_local(context),
            Self::Command(command) => command.run().is_ok(),
            Self::Bool(literal) => *literal,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all(deserialize = "snake_case"))]
enum CaseUnit<T> {
    Positive { condition: Condition, include: T },
    Negative { condition: Condition, include: T },
    Default { include: T },
}

impl<T> CaseUnit<T> {
    fn evaluate(&self, default: bool, context: &Context) -> bool {
        match self {
            Self::Positive { condition, .. } => condition.evaluate(context),
            Self::Negative { condition, .. } => !condition.evaluate(context),
            Self::Default { .. } => default,
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
pub struct Case<T>(Vec<CaseUnit<T>>);

impl<T> ResolveInto for Case<T>
where
    T: ResolveInto,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        let mut default = true;
        for case in self.0 {
            if case.evaluate(default, context) {
                default = false;
                case.unpack().resolve_into(context, output)?;
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
        let locale = get_context(&["yurt", "--override-user", "some-other-user"]).locale;
        assert_eq!(locale.user, "some-other-user");
    }

    #[test]
    fn override_distro() {
        let locale = get_context(&["yurt", "--override-distro", "some-other-distro"]).locale;
        assert_eq!(locale.distro, "some-other-distro");
    }

    #[test]
    fn override_platform() {
        let locale = get_context(&["yurt", "--override-platform", "some-other-platform"]).locale;
        assert_eq!(locale.platform, "some-other-platform");
    }

    #[test]
    fn locale_condition() {
        let condition = Condition::Locale(LocaleSpec {
            user: Some(whoami::username()),
            platform: None,
            distro: None,
        });
        assert!(condition.evaluate(&get_context(&[])));
    }

    #[test]
    fn command_condition() {
        let condition = Condition::Command(ShellCommand::from("echo 'test'".to_string()));
        assert!(condition.evaluate(&get_context(&[])));
    }

    #[test]
    fn bool_condition() {
        let condition = Condition::Bool(true);
        assert!(condition.evaluate(&get_context(&[])));
    }

    #[test]
    fn positive_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Positive {
            condition: Condition::Bool(true),
            include: "something",
        };
        assert!(case.evaluate(false, &context));
    }

    #[test]
    fn positive_non_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Positive {
            condition: Condition::Bool(false),
            include: "something",
        };
        assert!(!case.evaluate(false, &context));
    }

    #[test]
    fn negative_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Negative {
            condition: Condition::Bool(false),
            include: "something",
        };
        assert!(case.evaluate(false, &context));
    }

    #[test]
    fn negative_non_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Negative {
            condition: Condition::Bool(true),
            include: "something",
        };
        assert!(!case.evaluate(false, &context));
    }
}
