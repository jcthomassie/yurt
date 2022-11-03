use crate::units::{
    shell::{Cmd, Shell},
    BuildUnit, Context, Resolve,
};

use anyhow::{anyhow, bail, Result};
use indexmap::{IndexMap, IndexSet};
use log::{info, warn};
use serde::{Deserialize, Serialize};

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
    fn get_name(&self, manager: &PackageManager) -> &String {
        self.aliases.get(manager).unwrap_or(&self.name)
    }

    fn manager_names(&self) -> impl Iterator<Item = (&PackageManager, &String)> {
        self.managers
            .iter()
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
            info!("Package already installed: {}", self.name);
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
            name: context.variables.parse_str(&self.name)?,
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

impl Cmd for PackageManager {
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
}

impl PackageManager {
    /// Install a package
    pub fn install(self, package: &str) -> Result<()> {
        info!("Installing package `{}` with `{}`", package, self.name());
        match self {
            Self::Apt | Self::AptGet | Self::Pkg | Self::Yum => {
                "sudo".call(&[self.name(), "install", "-y", package])
            }
            Self::Cargo => self.call(&["install", package]),
            _ => self.call(&["install", "-y", package]),
        }
    }

    /// Uninstall a package
    pub fn uninstall(self, package: &str) -> Result<()> {
        info!("Uninstalling package `{}` from `{}`", package, self.name());
        match self {
            Self::Apt | Self::AptGet | Self::Pkg | Self::Yum => {
                "sudo".call(&[self.name(), "remove", "-y", package])
            }
            Self::Cargo => self.call(&["uninstall", package]),
            _ => self.call(&["uninstall", "-y", package]),
        }
    }

    /// Check if a package is installed
    pub fn has(self, package: &str) -> bool {
        let res = match self {
            Self::Apt | Self::AptGet => "dpkg".call_bool(&["-l", package]),
            Self::Brew => self.call_bool(&["list", package]),
            Self::Cargo => Shell::default().run_bool(
                if cfg!(windows) {
                    format!("cargo install --list | findstr /b /l /c:{}", package)
                } else {
                    format!("cargo install --list | grep '^{} v'", package)
                }
                .as_str(),
            ),
            Self::Pkg => self.call_bool(&["info", package]),
            _ => Ok(false),
        };
        match res {
            Ok(has) => has,
            Err(err) => {
                warn!("{}", err);
                false
            }
        }
    }

    /// Install the package manager and perform setup
    pub fn bootstrap(self) -> Result<()> {
        info!("Bootstrapping {}", self.name());
        match self {
            Self::Brew => Shell::from("bash").remote_script(&[
                "-fsSL",
                "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
            ]),
            Self::Cargo => Shell::from("sh").remote_script(&[
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
fn which_has(cmd: &str) -> bool {
    #[cfg(unix)]
    let name = "which";
    #[cfg(windows)]
    let name = "where";
    match name.call_bool(&[cmd]) {
        Ok(has) => has,
        Err(e) => {
            warn!("'{}' failed for {}: {}", name, cmd, e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::tests::get_context;

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
        assert_eq!(package.get_name(&aliased), "alias");
        for manager in &managers {
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
        let spec: Package = serde_yaml::from_str("name: ${{ namespace.key }}").unwrap();
        let mut context = get_context(&[]);
        context
            .variables
            .push("namespace", [("key", "value")].into_iter());
        // No managers remain
        let resolved = spec.resolve(&mut context).unwrap();
        let package = unpack!(@unit_vec resolved, BuildUnit::Install);
        assert_eq!(package.name, "value");
    }

    #[test]
    fn package_manager_prune_empty() {
        let spec: Package = serde_yaml::from_str("name: some-package").unwrap();
        let mut context = get_context(&[]);
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
        let mut context = get_context(&[]);
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
