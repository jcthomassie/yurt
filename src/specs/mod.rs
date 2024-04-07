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
    Package,
    #[clap(name = "package_manager")]
    PackageManager,
}

/// Single resolved build step
#[derive(Debug, Clone, PartialEq)]
pub enum BuildUnit {
    Repo(Repo),
    Link(Link),
    Hook(ShellHook),
    Package(Package),
    PackageManager(PackageManager),
}

impl BuildUnit {
    pub fn included_in(&self, units: &[BuildUnitKind]) -> bool {
        units.contains(&match self {
            Self::Repo(_) => BuildUnitKind::Repo,
            Self::Link(_) => BuildUnitKind::Link,
            Self::Hook(_) => BuildUnitKind::Hook,
            Self::Package(_) => BuildUnitKind::Package,
            Self::PackageManager(_) => BuildUnitKind::PackageManager,
        })
    }
}

/// Supported YAML build specifiers
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    Vars(Vars),
    Case(Case<Vec<Self>>),
    Matrix(Matrix<Vec<Self>>),
    Repo(Repo),
    Link(Link),
    Hook(ShellHook),
    Package(Package),
    PackageManager(PackageManager),
}

impl From<BuildUnit> for BuildSpec {
    fn from(unit: BuildUnit) -> Self {
        match unit {
            BuildUnit::Repo(repo) => Self::Repo(repo),
            BuildUnit::Link(link) => Self::Link(link),
            BuildUnit::Hook(hook) => Self::Hook(hook),
            BuildUnit::Package(package) => Self::Package(package),
            BuildUnit::PackageManager(manager) => Self::PackageManager(manager),
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
            Self::Package(p) => p.resolve_into(context, output),
            Self::PackageManager(m) => m.resolve_into(context, output),
        }
    }
}
