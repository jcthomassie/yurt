use crate::specs::{
    shell::{command, Shell},
    BuildUnit, Context, Resolve,
};

use anyhow::{anyhow, bail, Result};
use indexmap::{IndexMap, IndexSet};
use serde::{Deserialize, Serialize};
use std::process::Command;

pub use PackageManager::{Apt, AptGet, Brew, Cargo, Choco, Pkg, Yum};

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Package {
    name: String,
    #[serde(default = "IndexSet::new")]
    #[serde(skip_serializing_if = "IndexSet::is_empty")]
    managers: IndexSet<PackageManager>,
    #[serde(default = "IndexMap::new")]
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    aliases: IndexMap<PackageManager, String>,
}

impl Package {
    fn get_name(&self, manager: PackageManager) -> &String {
        self.aliases.get(&manager).unwrap_or(&self.name)
    }

    fn manager_names(&self) -> impl Iterator<Item = (PackageManager, &String)> {
        self.managers
            .iter()
            .copied()
            .map(move |manager| (manager, self.get_name(manager)))
    }

    pub fn is_installed(&self) -> bool {
        which_has(&self.name)
            || self
                .manager_names()
                .any(|(manager, package)| manager.has(package))
    }

    pub fn install(&self) -> Result<()> {
        if self.is_installed() {
            log::info!("Package already installed: {}", self.name);
        } else if let Some((manager, package)) = self.manager_names().next() {
            manager.install(package)?;
        } else {
            bail!("Package unavailable: {}", self.name);
        }
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        for (manager, package) in self.manager_names() {
            if manager.has(package) {
                manager.uninstall(package)?;
            }
        }
        Ok(())
    }
}

impl Resolve for Package {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Install(Self {
            name: context.parse_str(&self.name)?,
            managers: match self.managers.is_empty() {
                false => context
                    .managers
                    .intersection(&self.managers)
                    .copied()
                    .collect(),
                true => context.managers.clone(),
            },
            ..self
        }))
    }
}

#[derive(Debug, Deserialize, Serialize, Copy, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum PackageManager {
    Apt,
    AptGet,
    Brew,
    Cargo,
    Choco,
    Pkg,
    Yum,
}

impl PackageManager {
    fn name(&self) -> &str {
        match self {
            Self::Apt => "apt",
            Self::AptGet => "apt-get",
            Self::Brew => "brew",
            Self::Cargo => "cargo",
            Self::Choco => "choco",
            Self::Pkg => "pkg",
            Self::Yum => "yum",
        }
    }

    /// Install a package
    pub fn install(self, package: &str) -> Result<()> {
        log::info!("Installing package `{}` with `{}`", package, self.name());
        let mut cmd = Command::new(self.name());
        match self {
            Self::Cargo => {
                cmd.args(["install", package]);
            }
            _ => {
                cmd.args(["install", "-y", package]);
            }
        };
        command::call(&mut cmd)
    }

    /// Uninstall a package
    pub fn uninstall(self, package: &str) -> Result<()> {
        log::info!("Uninstalling package `{}` from `{}`", package, self.name());
        let mut cmd = Command::new(self.name());
        match self {
            Self::Apt | Self::AptGet | Self::Pkg | Self::Yum => {
                cmd.args(["remove", "-y", package]);
            }
            Self::Cargo => {
                cmd.args(["uninstall", package]);
            }
            Self::Choco | Self::Brew => {
                cmd.args(["uninstall", "-y", package]);
            }
        };
        command::call(&mut cmd)
    }

    /// Check if a package is installed
    pub fn has(self, package: &str) -> bool {
        let res = match self {
            Self::Apt | Self::AptGet => {
                command::call_bool(Command::new("dpkg").args(["-l", package]))
            }
            Self::Brew => command::call_bool(Command::new(self.name()).args(["list", package])),
            Self::Cargo => Shell::default().exec_bool(
                if cfg!(windows) {
                    format!("cargo install --list | findstr /b /l /c:{package}")
                } else {
                    format!("cargo install --list | grep '^{package} v'")
                }
                .as_str(),
            ),
            Self::Pkg => command::call_bool(Command::new(self.name()).args(["info", package])),
            _ => Ok(false),
        };
        match res {
            Ok(has) => has,
            Err(err) => {
                log::warn!("{err}");
                false
            }
        }
    }

    /// Install the package manager and perform setup
    pub fn bootstrap(self) -> Result<()> {
        log::info!("Bootstrapping {}", self.name());
        match self {
            Self::Brew => Shell::from("bash").exec_remote(&[
                "-fsSL",
                "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
            ]),
            Self::Cargo => Shell::from("sh").exec_remote(&[
                "--proto",
                "'=https'",
                "--tlsv1.2",
                "-sSf",
                "https://sh.rustup.rs",
            ]),
            manager => Err(anyhow!("Bootstrap not supported for {}", manager.name())),
        }
    }

    /// Install the package manager if not already installed
    pub fn require(self) -> Result<()> {
        if self.is_available() {
            return Ok(());
        }
        self.bootstrap()
    }

    /// Check if package manager is installed
    pub fn is_available(self) -> bool {
        which_has(self.name())
    }
}

impl Resolve for PackageManager {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        context.managers.insert(self);
        Ok(BuildUnit::Require(self))
    }
}

/// Check if a command is available locally
#[inline]
fn which_has(name: &str) -> bool {
    #[cfg(unix)]
    let mut cmd = Command::new("which");
    #[cfg(windows)]
    let mut cmd = Command::new("where");
    match command::call_bool(cmd.arg(name)) {
        Ok(has) => has,
        Err(err) => {
            log::warn!("{err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! check_missing {
        ($manager:ident, $mod_name:ident, $expect_fake:expr, $expect_empty:expr) => {
            mod $mod_name {
                use super::*;

                #[test]
                fn fake_package() {
                    assert_eq!($manager.has("some_missing_package"), $expect_fake);
                }

                #[test]
                fn empty_package() {
                    assert_eq!($manager.has(""), $expect_empty);
                }
            }
        };

        ($manager:ident, $mod_name:ident) => {
            check_missing!($manager, $mod_name, false, false);
        };
    }

    check_missing!(Apt, apt);

    check_missing!(AptGet, apt_get);

    check_missing!(Brew, brew);

    check_missing!(Cargo, cargo);

    check_missing!(Choco, choco);

    #[cfg(not(target_os = "freebsd"))]
    check_missing!(Pkg, pkg);
    #[cfg(target_os = "freebsd")]
    check_missing!(Pkg, pkg, false, true);

    check_missing!(Yum, yum);

    #[test]
    fn which_has_cargo() {
        assert!(which_has("cargo"));
    }

    #[test]
    fn which_not_has_fake() {
        assert!(!which_has("some_missing_package"));
    }

    fn all() -> IndexSet<PackageManager> {
        vec![Apt, AptGet, Brew, Cargo, Choco, Pkg, Yum]
            .into_iter()
            .collect()
    }

    #[test]
    fn get_name_for_manager() {
        let mut managers = all();
        let aliased = managers.take(&Brew).unwrap();
        let package = Package {
            name: "name".to_string(),
            managers: all(),
            aliases: {
                let mut map = IndexMap::new();
                map.insert(aliased, "alias".into());
                map
            },
        };
        assert_eq!(package.get_name(aliased), "alias");
        for manager in managers {
            assert_eq!(package.get_name(manager), "name");
        }
    }

    #[test]
    fn check_installed() {
        assert!(Package {
            name: "cargo".to_string(),
            managers: all(),
            aliases: IndexMap::new(),
        }
        .is_installed());
    }

    #[test]
    fn check_not_installed() {
        assert!(!Package {
            name: "some_missing_package".to_string(),
            managers: all(),
            aliases: IndexMap::new()
        }
        .is_installed());
    }

    macro_rules! unpack {
        (@unit $value:expr, BuildUnit::$variant:ident) => {
            if let BuildUnit::$variant(ref unwrapped) = $value {
                unwrapped
            } else {
                panic!("Failed to unpack build unit");
            }
        };
        (@unit_vec $value:expr, BuildUnit::$variant:ident) => {
            {
                unpack!(@unit ($value), BuildUnit::$variant)
            }
        };
    }

    #[test]
    fn package_name_substitution() {
        let spec: Package = serde_yaml::from_str("name: ${{ key }}").unwrap();
        let mut context = Context::default();
        context.variables.try_push("key", "value").unwrap();
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(package.name, "value");
    }

    #[test]
    fn package_manager_prune_empty() {
        let spec: Package = serde_yaml::from_str("name: some-package").unwrap();
        let mut context = Context::default();
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert!(package.managers.is_empty());
    }

    #[test]
    fn package_manager_prune_some() {
        #[rustfmt::skip]
        let spec: Package = serde_yaml::from_str("
            name: some-package
            managers: [ apt, brew ]
        ").unwrap();
        // Add partially overlapping managers
        let mut context = Context::default();
        context.managers.insert(PackageManager::Cargo);
        context.managers.insert(PackageManager::Brew);
        // Overlap remains
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(
            package.managers,
            vec![PackageManager::Brew]
                .into_iter()
                .collect::<IndexSet<_>>()
        );
    }
}
