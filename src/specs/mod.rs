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

#[derive(clap::ValueEnum, Debug, Copy, Clone, Eq, PartialEq)]
pub enum BuildUnitKind {
    Repo,
    Link,
    Hook,
    Install,
    Require,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuildUnit {
    Repo(Repo),
    Link(Link),
    Hook(ShellHook),
    Install(Package),
    Require(PackageManager),
}

impl BuildUnit {
    pub fn included_in(&self, units: &[BuildUnitKind]) -> bool {
        match self {
            Self::Repo(_) => units.contains(&BuildUnitKind::Repo),
            Self::Link(_) => units.contains(&BuildUnitKind::Link),
            Self::Hook(_) => units.contains(&BuildUnitKind::Hook),
            Self::Install(_) => units.contains(&BuildUnitKind::Install),
            Self::Require(_) => units.contains(&BuildUnitKind::Require),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    Vars(Vars),
    Case(Case<Vec<BuildSpec>>),
    Matrix(Matrix<Vec<BuildSpec>>),
    Repo(Repo),
    Link(Vec<Link>),
    Hook(ShellHook),
    Install(Vec<Package>),
    Require(Vec<PackageManager>),
}

impl BuildSpec {
    pub fn absorb(&mut self, unit: &BuildUnit) -> bool {
        match (self, unit) {
            (Self::Link(a), BuildUnit::Link(b)) => {
                a.push(b.clone());
                true
            }
            (Self::Install(a), BuildUnit::Install(b)) => {
                a.push(b.clone());
                true
            }
            (Self::Require(a), BuildUnit::Require(b)) => {
                a.push(b.clone());
                true
            }
            _ => false,
        }
    }
}

impl From<BuildUnit> for BuildSpec {
    fn from(unit: BuildUnit) -> BuildSpec {
        match unit {
            BuildUnit::Repo(repo) => Self::Repo(repo),
            BuildUnit::Link(link) => Self::Link(vec![link]),
            BuildUnit::Hook(hook) => Self::Hook(hook),
            BuildUnit::Install(package) => Self::Install(vec![package]),
            BuildUnit::Require(manager) => Self::Require(vec![manager]),
        }
    }
}

impl ResolveInto for BuildSpec {
    fn resolve_into(self, context: &mut Context, output: &mut Vec<BuildUnit>) -> Result<()> {
        match self {
            Self::Vars(v) => v.resolve_into(context, output),
            Self::Case(v) => v.resolve_into(context, output),
            Self::Matrix(m) => m.resolve_into(context, output),
            Self::Repo(r) => r.resolve_into(context, output),
            Self::Link(v) => v.resolve_into(context, output),
            Self::Hook(s) => s.resolve_into(context, output),
            Self::Install(v) => v.resolve_into(context, output),
            Self::Require(v) => v.resolve_into(context, output),
        }
    }
}
