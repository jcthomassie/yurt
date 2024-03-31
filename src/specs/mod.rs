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

    pub fn should_apply(&self, context: &Context, hook: &Hook) -> bool {
        let should_install = matches!(hook, Hook::Install);
        match hook {
            Hook::Install | Hook::Uninstall => match self {
                Self::Repo(repo) => repo.is_available() != should_install,
                Self::Link(link) => link.is_valid() != should_install,
                Self::Package(package) => package.is_installed(context) != should_install,
                Self::PackageManager(manager) => manager.is_available() != should_install,
                Self::Hook(shell_hook) => shell_hook.applies(hook),
            },
            Hook::Custom(_) => match self {
                Self::Hook(shell_hook) => shell_hook.applies(hook),
                _ => false,
            },
        }
    }

    pub fn install(&self, context: &Context, clean: bool) -> Result<()> {
        match self {
            Self::Repo(repo) => repo.require().map(drop),
            Self::Link(link) => link.link(clean),
            Self::Hook(hook) => hook.exec_for(&Hook::Install),
            Self::Package(package) => package.install(context),
            Self::PackageManager(manager) => manager.require(),
        }
    }

    pub fn uninstall(&self, context: &Context) -> Result<()> {
        match self {
            Self::Link(link) => link.unlink(),
            Self::Hook(hook) => hook.exec_for(&Hook::Uninstall),
            Self::Package(package) => package.uninstall(context),
            _ => Ok(()),
        }
    }

    pub fn hook(&self, hook: &Hook) -> Result<()> {
        match self {
            Self::Hook(shell_hook) => shell_hook.exec_for(hook),
            _ => Ok(()),
        }
    }
}

/// Supported YAML build specifiers
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BuildSpec {
    /// [`!vars`][Vars]
    Vars(Vars),
    /// [`!case`][Case<Vec<Self>>]
    Case(Case<Vec<Self>>),
    /// [`!matrix`][Matrix<Vec<Self>>]
    Matrix(Matrix<Vec<Self>>),
    /// [`!repo`][Repo]
    Repo(Repo),
    /// [`!link`][Link]
    Link(Link),
    /// [`!hook`][ShellHook]
    Hook(ShellHook),
    /// [`!package`][Package]
    Package(Package),
    /// [`!package_manager`][PackageManager]
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
