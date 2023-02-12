mod dynamic;
mod link;
mod package;
mod repo;
mod shell;

pub use self::package::PackageManager;
pub use self::shell::Hook;
use self::{
    dynamic::{Case, Matrix, Vars},
    link::Link,
    package::Package,
    repo::Repo,
    shell::ShellHook,
};

use crate::context::Context;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub trait Resolve {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit>;
}

pub trait ResolveInto {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()>;

    fn resolve_into_new(self, context: &mut Context) -> Result<Vec<BuildUnit>>
    where
        Self: Sized,
    {
        let mut output = Vec::new();
        self.resolve_into(context, &mut output)?;
        Ok(output)
    }
}

impl<T> ResolveInto for T
where
    T: Resolve,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        output.push(self.resolve(context)?);
        Ok(())
    }
}

impl<T> ResolveInto for Vec<T>
where
    T: ResolveInto,
{
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        for inner in self {
            inner.resolve_into(context, output)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuildUnit {
    Repo(Repo),
    Link(Link),
    Hook(ShellHook),
    Install(Package),
    Require(PackageManager),
}

#[derive(clap::ValueEnum, Debug, Copy, Clone, Eq, PartialEq)]
pub enum BuildUnitKind {
    Repo,
    Link,
    Hook,
    Install,
    Require,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    Repo(Repo),
    Vars(Vars),
    Matrix(Matrix<Vec<BuildSpec>>),
    Case(Case<Vec<BuildSpec>>),
    Link(Vec<Link>),
    Hook(ShellHook),
    Install(Vec<Package>),
    Require(Vec<PackageManager>),
}

impl BuildSpec {
    pub fn absorb(self: &mut BuildSpec, unit: &BuildUnit) -> bool {
        match (self, unit) {
            (BuildSpec::Link(a), BuildUnit::Link(b)) => {
                a.push(b.clone());
                true
            }
            (BuildSpec::Install(a), BuildUnit::Install(b)) => {
                a.push(b.clone());
                true
            }
            (BuildSpec::Require(a), BuildUnit::Require(b)) => {
                a.push(*b);
                true
            }
            _ => false,
        }
    }
}

impl ResolveInto for BuildSpec {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        match self {
            Self::Repo(r) => r.resolve_into(context, output),
            Self::Vars(v) => v.resolve_into(context, output),
            Self::Matrix(m) => m.resolve_into(context, output),
            Self::Case(v) => v.resolve_into(context, output),
            Self::Link(v) => v.resolve_into(context, output),
            Self::Hook(s) => s.resolve_into(context, output),
            Self::Install(v) => v.resolve_into(context, output),
            Self::Require(v) => v.resolve_into(context, output),
        }
    }
}
