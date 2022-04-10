use crate::build::{BuildUnit, Context, Resolve};
use anyhow::Result;
use clap::ArgMatches;
use serde::{Deserialize, Serialize};

pub trait Condition {
    fn evaluate(&self, context: &Context) -> bool;
}

#[derive(Serialize, Deserialize)]
pub struct Conditional<C, I> {
    condition: C,
    include: I,
    #[serde(default)]
    negate: bool,
}

impl<C, I> Resolve for Conditional<C, I>
where
    C: Condition,
    I: Resolve,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        if self.condition.evaluate(context) {
            self.include.resolve_into(context, output)
        } else {
            Ok(())
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

#[derive(Debug, Default, PartialEq, Deserialize, Serialize, Clone)]
pub struct LocaleSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<String>,
}

impl LocaleSpec {
    pub fn is_local(&self, rubric: &Locale) -> bool {
        match self {
            LocaleSpec { user: Some(u), .. } if u != &rubric.user => false,
            LocaleSpec {
                platform: Some(p), ..
            } if p != &rubric.platform => false,
            LocaleSpec {
                distro: Some(d), ..
            } if d != &rubric.distro => false,
            _ => true,
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
    pub fn rule(self, default: bool, rubric: &Locale) -> Option<T> {
        match self {
            Case::Positive { spec, include } if spec.is_local(rubric) => Some(include),
            Case::Negative { spec, include } if !spec.is_local(rubric) => Some(include),
            Case::Default { include } if default => Some(include),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yurt_command;

    fn get_locale(args: &[&str]) -> Locale {
        Locale::from(&yurt_command().get_matches_from(args))
    }

    #[test]
    fn override_user() {
        let locale = get_locale(&["yurt", "--override-user", "some-other-user"]);
        assert_eq!(locale.user, "some-other-user");
    }

    #[test]
    fn override_distro() {
        let locale = get_locale(&["yurt", "--override-distro", "some-other-distro"]);
        assert_eq!(locale.distro, "some-other-distro");
    }

    #[test]
    fn override_platform() {
        let locale = get_locale(&["yurt", "--override-platform", "some-other-platform"]);
        assert_eq!(locale.platform, "some-other-platform");
    }

    fn locale_spec(s: &str) -> LocaleSpec {
        serde_yaml::from_str::<LocaleSpec>(s).expect("Invalid yaml LocaleSpec")
    }

    #[test]
    fn positive_match() {
        let locale = get_locale(&[]);
        let case = Case::Positive {
            spec: locale_spec(format!("user: {}", whoami::username()).as_str()),
            include: "something",
        };
        assert_eq!(case.rule(false, &locale).unwrap(), "something");
    }

    #[test]
    fn positive_non_match() {
        let locale = get_locale(&[]);
        let case = Case::Positive {
            spec: locale_spec("distro: something_else"),
            include: "something",
        };
        assert!(case.rule(false, &locale).is_none());
    }

    #[test]
    fn negative_match() {
        let locale = get_locale(&[]);
        let case = Case::Negative {
            spec: locale_spec("platform: somewhere_else"),
            include: "something",
        };
        assert_eq!(case.rule(false, &locale).unwrap(), "something");
    }

    #[test]
    fn negative_non_match() {
        let locale = get_locale(&[]);
        let case = Case::Negative {
            spec: locale_spec(format!("user: {}", whoami::username()).as_str()),
            include: "something",
        };
        assert!(case.rule(false, &locale).is_none());
    }
}
