use crate::{
    context::{parse::ObjectKey, Context, LocaleSpec},
    specs::{shell::ShellCommand, BuildUnit, ResolveInto},
    yaml_example,
};

use anyhow::{bail, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum Condition {
    /// Literal boolean
    Bool(bool),
    /// `true` when [`!locale_spec`](LocaleSpec) matches local environment
    Locale(LocaleSpec),
    /// `true` when [`!shell_command`](ShellCommand) exits successfully
    Eval(ShellCommand),
    /// `true` when all inner [`conditions`](Condition) are `true`
    All(Vec<Condition>),
    /// `true` when any inner [`conditions`](Condition) are `true`
    Any(Vec<Condition>),
    /// Equivalent to the negation of [`!any`](Condition::Any)
    Not(Vec<Condition>),
    /// Equivalent to [`!bool true`](Condition::Bool)
    Default,
}

impl Condition {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        match self {
            Self::Bool(literal) => Ok(*literal),
            Self::Locale(spec) => Ok(spec.matches(&context.locale)),
            Self::Eval(command) => command.exec_bool(),
            Self::All(conds) | Self::Any(conds) | Self::Not(conds) => {
                let evaluated = conds
                    .iter()
                    .map(|c| c.evaluate(context))
                    .collect::<Result<Vec<bool>>>()?;
                Ok(match self {
                    Self::All(_) => evaluated.into_iter().all(|b| b),
                    Self::Any(_) => evaluated.into_iter().any(|b| b),
                    Self::Not(_) => !evaluated.into_iter().any(|b| b),
                    _ => unreachable!(),
                })
            }
            Self::Default => Ok(true),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CaseBranch<T> {
    /// Boolean expression that is evaluated to determine inclusion
    condition: Condition,
    /// Result of [`condition`] required for inclusion (default `true`)
    #[serde(skip_serializing_if = "Option::is_none")]
    when: Option<bool>,
    /// Object to be included when [`condition`] output matches [`when`]
    include: T,
}

impl<T> CaseBranch<T> {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        self.condition
            .evaluate(context)
            .map(|b| b == self.when.unwrap_or(true))
    }
}

/// Expression that resolves the first matching [branch](CaseBranch).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Case<T>(Vec<CaseBranch<T>>);

impl<T> ResolveInto for Case<T>
where
    T: ResolveInto,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        for case in self.0 {
            if case.evaluate(context)? {
                return case.include.resolve_into(context, output);
            };
        }
        Ok(())
    }
}

/// Map of string substitutions
#[doc = yaml_example!("../../examples/vars.yaml")]
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(transparent)]
pub struct Vars(IndexMap<String, String>);

impl ObjectKey for Vars {
    const OBJECT_NAME: &'static str = "vars";
}

impl ResolveInto for Vars {
    fn resolve_into(self, context: &mut Context, _output: &mut Vec<BuildUnit>) -> Result<()> {
        for (key, val) in self.0 {
            context.variables.push(Self::object_key(key), val);
        }
        Ok(())
    }
}

/// Object to include repeatedly for each value
#[doc = yaml_example!("../../examples/matrix.yaml")]
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Matrix<T> {
    /// Sequence of string substitution mappings
    values: Vec<IndexMap<String, String>>,
    /// Object to be included
    include: T,
}

impl<T> ObjectKey for Matrix<T> {
    const OBJECT_NAME: &'static str = "matrix";
}

impl<T> ResolveInto for Matrix<T>
where
    T: ResolveInto + Clone,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        if self.values.is_empty() {
            bail!("Matrix values must be non-empty")
        }
        for item in self.values {
            for (key, val) in item.keys().zip(item.values()) {
                context.variables.push(
                    Self::object_key(key),
                    context.parse_str(val)?, // internal replacement
                );
            }
            self.include.clone().resolve_into(context, output)?;
            for key in item.into_keys() {
                context.variables.drop(&Self::object_key(key));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::specs::BuildSpec;
    use pretty_assertions::assert_eq;

    mod condition {
        use super::*;
        use pretty_assertions::assert_eq;

        macro_rules! yaml_condition {
            ($yaml:expr, $enum_pattern:pat, $evaluation:literal) => {
                let cond: Condition = serde_yaml::from_str($yaml).expect("Deserialization failed");
                assert!(matches!(cond, $enum_pattern));
                assert_eq!(cond.evaluate(&Context::default()).unwrap(), $evaluation);
            };
        }

        #[test]
        fn locale() {
            let user_locale = format!("!locale {{ user: {} }}", whoami::username());
            yaml_condition!(user_locale.as_str(), Condition::Locale(_), true);
            yaml_condition!("!locale { platform: fake }", Condition::Locale(_), false);
        }

        #[test]
        fn eval() {
            yaml_condition!(r#"!eval "echo 'hello'""#, Condition::Eval(_), true);
            yaml_condition!("!eval bad-command -a -b", Condition::Eval(_), false);
        }

        #[test]
        fn bool() {
            yaml_condition!("!bool true", Condition::Bool(true), true);
            yaml_condition!("!bool false", Condition::Bool(false), false);
        }

        #[test]
        fn all() {
            yaml_condition!(
                "!all [ !bool true, !bool true, !bool true ]",
                Condition::All(_),
                true
            );
            yaml_condition!(
                "!all [ !bool true, !bool true, !bool false ]",
                Condition::All(_),
                false
            );
        }

        #[test]
        fn any() {
            yaml_condition!("!any [ ]", Condition::Any(_), false);
            yaml_condition!(
                "!any [ !bool false, !bool false, !bool false ]",
                Condition::Any(_),
                false
            );
            yaml_condition!(
                "!any [ !bool false, !bool false, !bool true ]",
                Condition::Any(_),
                true
            );
        }

        #[test]
        fn not() {
            yaml_condition!("!not [ ]", Condition::Not(_), true);
            yaml_condition!("!not [ !bool false ]", Condition::Not(_), true);
            yaml_condition!("!not [ !bool true ]", Condition::Not(_), false);
            yaml_condition!("!not [ !bool false, !bool true ]", Condition::Not(_), false);
        }
    }

    #[test]
    fn positive_match() {
        let case = CaseBranch {
            condition: Condition::Bool(true),
            when: Some(true),
            include: "something",
        };
        assert!(case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn positive_non_match() {
        let case = CaseBranch {
            condition: Condition::Bool(false),
            when: Some(true),
            include: "something",
        };
        assert!(!case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn negative_match() {
        let case = CaseBranch {
            condition: Condition::Bool(false),
            when: Some(false),
            include: "something",
        };
        assert!(case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn negative_non_match() {
        let case = CaseBranch {
            condition: Condition::Bool(true),
            when: Some(false),
            include: "something",
        };
        assert!(!case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn vars_resolve() {
        #[rustfmt::skip]
        let vars: Vars = serde_yaml::from_str("
            key_a: val_a
            key_b: val_b
        ").unwrap();
        let mut context = Context::default();
        vars.resolve_into_new(&mut context).unwrap();
        assert_eq!(context.parse_str("${{ vars.key_a }}").unwrap(), "val_a");
        assert_eq!(context.parse_str("${{ vars.key_b }}").unwrap(), "val_b");
    }

    #[test]
    fn matrix_empty() {
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str(r#"
            values: []
            include: []
        "#).unwrap();
        let mut context = Context::default();
        assert!(matrix.resolve_into_new(&mut context).is_err());
    }

    #[test]
    fn matrix_expansion() {
        let mut context = Context::default();
        context.variables.try_push("outer.key", "value").unwrap();
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str(r#"
            values:
              - a: "${{ outer.key }}_a"
              - a: "${{ outer.key }}_b"
              - a: "${{ outer.key }}_c"
            include:
              - !link
                  source: ${{ matrix.a }}
                  target: const
        "#).unwrap();
        #[rustfmt::skip]
        let values: Vec<BuildSpec> = serde_yaml::from_str(r#"
            - !link
                source: value_a
                target: const
            - !link
                source: value_b
                target: const
            - !link
                source: value_c
                target: const
        "#).unwrap();
        assert_eq!(
            matrix.resolve_into_new(&mut context).unwrap(),
            values.resolve_into_new(&mut context).unwrap()
        );
    }
}
