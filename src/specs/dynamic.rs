use crate::{
    context::{Context, LocaleSpec},
    specs::{shell::ShellCommand, BuildUnit, ResolveInto},
};

use anyhow::{bail, Context as _, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::ops::Not;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all(deserialize = "snake_case"))]
enum Condition {
    Bool(bool),
    Locale(LocaleSpec),
    Eval(ShellCommand),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
}

impl Condition {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        match self {
            Self::Bool(literal) => Ok(*literal),
            Self::Locale(spec) => Ok(spec.is_local(context)),
            Self::Eval(command) => command.exec_bool(),
            Self::All(conds) | Self::Any(conds) => {
                let evaluated = conds
                    .iter()
                    .map(|c| c.evaluate(context))
                    .collect::<Result<Vec<bool>>>()?;
                Ok(match self {
                    Self::All(_) => evaluated.into_iter().all(|b| b),
                    Self::Any(_) => evaluated.into_iter().any(|b| b),
                    _ => unreachable!(),
                })
            }
            Self::Not(c) => c.evaluate(context).map(Not::not),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CaseBranch<T> {
    condition: Option<Condition>,
    when: Option<bool>,
    include: T,
}

impl<T> CaseBranch<T> {
    fn evaluate(&self, context: &Context) -> Result<bool> {
        self.condition
            .as_ref()
            .map_or(Ok(true), |c| c.evaluate(context))
            .map(|b| b == self.when.unwrap_or(true))
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
                return case.include.resolve_into(context, output);
            };
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Vars(IndexMap<String, String>);

impl Vars {
    const NAMESPACE: &str = "vars";
}

impl ResolveInto for Vars {
    fn resolve_into(self, context: &mut Context, _output: &mut Vec<BuildUnit>) -> Result<()> {
        context.variables.push(Self::NAMESPACE, self.0.iter());
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Matrix<T> {
    values: IndexMap<String, Vec<String>>,
    include: T,
}

impl<T> Matrix<T> {
    const NAMESPACE: &str = "matrix";

    fn length(&self) -> Result<usize> {
        let mut counts = self.values.values().map(Vec::len);
        match counts.next() {
            Some(first) => counts
                .all(|next| next == first)
                .then_some(first)
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
        for i in 0..self.length()? {
            let vals = self
                .values
                .values()
                .map(|vec| context.variables.parse_str(&vec[i]))
                .collect::<Result<Vec<_>>>()?;
            context
                .variables
                .push(Self::NAMESPACE, self.values.keys().zip(vals.iter()));
            self.include.clone().resolve_into(context, output)?;
            context.variables.drop(Self::NAMESPACE, self.values.keys());
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
        use crate::context::Context;
        use crate::specs::dynamic::Condition;
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
        #[ignore]
        /// Nested enum deserialization is not currently supported: <https://github.com/dtolnay/serde-yaml/blob/186cc67720545a7e387a420a10ecdbfa147a9c40/src/de.rs#L1716>
        fn not() {
            yaml_condition!("!not !bool false", Condition::Not(_), true);
            yaml_condition!("!not !bool true", Condition::Not(_), false);
        }
    }

    #[test]
    fn positive_match() {
        let case = CaseBranch {
            condition: Some(Condition::Bool(true)),
            when: Some(true),
            include: "something",
        };
        assert!(case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn positive_non_match() {
        let case = CaseBranch {
            condition: Some(Condition::Bool(false)),
            when: Some(true),
            include: "something",
        };
        assert!(!case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn negative_match() {
        let case = CaseBranch {
            condition: Some(Condition::Bool(false)),
            when: Some(false),
            include: "something",
        };
        assert!(case.evaluate(&Context::default()).unwrap());
    }

    #[test]
    fn negative_non_match() {
        let case = CaseBranch {
            condition: Some(Condition::Bool(true)),
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
        assert_eq!(
            context.variables.get(Vars::NAMESPACE, "key_a").unwrap(),
            "val_a"
        );
        assert_eq!(
            context.variables.get(Vars::NAMESPACE, "key_b").unwrap(),
            "val_b"
        );
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
        let mut context = Context::default();
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
        let mut context = Context::default();
        context
            .variables
            .push("outer", [("key", "value")].into_iter());
        #[rustfmt::skip]
        let matrix: Matrix<Vec<BuildSpec>> = serde_yaml::from_str(r#"
            values:
              inner:
                - "${{ outer.key }}_a"
                - "${{ outer.key }}_b"
                - "${{ outer.key }}_c"
            include:
              - !link
                - source: ${{ matrix.inner }}
                  target: const
        "#).unwrap();
        #[rustfmt::skip]
        let values: Vec<BuildSpec> = serde_yaml::from_str(r#"
            - !link
              - source: value_a
                target: const
              - source: value_b
                target: const
              - source: value_c
                target: const
        "#).unwrap();
        assert_eq!(
            matrix.resolve_into_new(&mut context).unwrap(),
            values.resolve_into_new(&mut context).unwrap()
        );
    }
}
