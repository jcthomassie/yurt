use crate::build::{BuildUnit, Context, ResolveInto};
use anyhow::Result;
use clap::ArgMatches;
use serde::{Deserialize, Serialize};

trait Condition {
    fn evaluate(&self, context: &Context) -> bool;
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
pub struct LocaleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
}

impl Condition for LocaleSpec {
    fn evaluate(&self, context: &Context) -> bool {
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
#[serde(rename_all(deserialize = "snake_case"))]
enum CaseUnit<C, T> {
    Positive { spec: C, include: T },
    Negative { spec: C, include: T },
    Default { include: T },
}

impl<C, T> CaseUnit<C, T>
where
    C: Condition,
{
    fn evaluate(&self, default: bool, context: &Context) -> bool {
        match self {
            Self::Positive { spec, .. } => spec.evaluate(context),
            Self::Negative { spec, .. } => !spec.evaluate(context),
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
pub struct Case<C, T>(Vec<CaseUnit<C, T>>);

impl<C, T> ResolveInto for Case<C, T>
where
    C: Condition,
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

    fn locale_spec(s: &str) -> LocaleSpec {
        serde_yaml::from_str::<LocaleSpec>(s).expect("Invalid yaml LocaleSpec")
    }

    #[test]
    fn positive_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Positive {
            spec: locale_spec(format!("user: {}", whoami::username()).as_str()),
            include: "something",
        };
        assert!(case.evaluate(false, &context));
    }

    #[test]
    fn positive_non_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Positive {
            spec: locale_spec("distro: something_else"),
            include: "something",
        };
        assert!(!case.evaluate(false, &context));
    }

    #[test]
    fn negative_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Negative {
            spec: locale_spec("platform: somewhere_else"),
            include: "something",
        };
        assert!(case.evaluate(false, &context));
    }

    #[test]
    fn negative_non_match() {
        let context = get_context(&[]);
        let case = CaseUnit::Negative {
            spec: locale_spec(format!("user: {}", whoami::username()).as_str()),
            include: "something",
        };
        assert!(!case.evaluate(false, &context));
    }
}
