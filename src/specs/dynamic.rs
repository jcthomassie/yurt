use crate::{
    context::{Context, LocaleSpec},
    specs::{shell::ShellCommand, BuildUnit, ResolveInto},
};

use anyhow::{bail, Context as _, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::ops::Not;

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
    use crate::{context::tests::get_context, specs::BuildSpec};
    use pretty_assertions::assert_eq;

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

    #[test]
    fn vars_resolve() {
        #[rustfmt::skip]
        let vars: Vars = serde_yaml::from_str("
            key_a: val_a
            key_b: val_b
        ").unwrap();
        let mut context = get_context(&[]);
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
        let mut context = get_context(&[]);
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
        let mut context = get_context(&[]);
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
              - !run ${{ matrix.inner }}
        "#).unwrap();
        #[rustfmt::skip]
        let values: Vec<BuildSpec> = serde_yaml::from_str(r#"
            - !run value_a
            - !run value_b
            - !run value_c
        "#).unwrap();
        assert_eq!(
            matrix.resolve_into_new(&mut context).unwrap(),
            values.resolve_into_new(&mut context).unwrap()
        );
    }
}
