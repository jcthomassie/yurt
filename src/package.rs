use crate::build::{self, BuildUnit, Resolve};
use crate::shell::{Cmd, Shell};
use anyhow::{anyhow, bail, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub use PackageManager::{Apt, AptGet, Brew, Cargo, Choco, Yum};

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Package {
    pub name: String,
    #[serde(default = "BTreeSet::new")]
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub managers: BTreeSet<PackageManager>,
    #[serde(default = "BTreeMap::new")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<PackageManager, String>,
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
    fn resolve(self, context: &mut build::Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Install(Package {
            name: context.replace_variables(&self.name)?,
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

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Hash, Copy, Clone, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum PackageManager {
    Apt,
    AptGet,
    Brew,
    Cargo,
    Choco,
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
            Self::Yum => "yum",
        }
    }
}

impl PackageManager {
    fn _install(self, package: &str, args: &[&str]) -> Result<()> {
        info!("Installing package ({} install {})", self.name(), package);
        self.call(&[&["install", package], args].concat())
    }

    fn _sudo_install(self, package: &str, args: &[&str]) -> Result<()> {
        info!(
            "Installing package (sudo {} install {})",
            self.name(),
            package
        );
        "sudo".call(&[&[self.name(), "install", package], args].concat())
    }

    // Install a package
    pub fn install(self, package: &str) -> Result<()> {
        match self {
            Self::Apt | Self::AptGet | Self::Yum => self._sudo_install(package, &["-y"]),
            Self::Cargo => self._install(package, &[]),
            _ => self._install(package, &["-y"]),
        }
    }

    // Uninstall a package
    pub fn uninstall(self, package: &str) -> Result<()> {
        info!(
            "Uninstalling package ({} uninstall {})",
            self.name(),
            package
        );
        self.call(&["uninstall", package])
    }

    // Check if a package is installed
    pub fn has(self, package: &str) -> bool {
        let res = match self {
            Self::Apt | Self::AptGet => "dpkg".call_bool(&["-l", package]),
            Self::Brew => self.call_bool(&["list", package]),
            Self::Cargo => Shell::default()
                .run(
                    if cfg!(windows) {
                        format!("cargo install --list | findstr /b /l /c:{}", package)
                    } else {
                        format!("cargo install --list | grep '^{} v'", package)
                    }
                    .as_str(),
                )
                .map(|s| s.status.success()),
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

    // Install the package manager and perform setup
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

    // Install the package manager if not already installed
    pub fn require(self) -> Result<()> {
        if self.is_available() {
            return Ok(());
        }
        self.bootstrap()
    }

    // Check if package manager is installed
    pub fn is_available(self) -> bool {
        which_has(self.name())
    }
}

impl Resolve for PackageManager {
    fn resolve(self, context: &mut build::Context) -> Result<BuildUnit> {
        context.managers.insert(self);
        Ok(BuildUnit::Require(self))
    }
}

// Check if a command is available locally
#[inline]
pub fn which_has(cmd: &str) -> bool {
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
    use crate::build::tests::get_context;

    macro_rules! check_missing {
        ($manager:ident, $mod_name:ident) => {
            mod $mod_name {
                use super::*;

                #[test]
                fn not_has_fake() {
                    assert!(!$manager.has("some_missing_package"));
                }

                #[test]
                fn not_has_empty() {
                    assert!(!$manager.has(""));
                }
            }
        };
    }

    check_missing!(Apt, apt);

    check_missing!(AptGet, apt_get);

    check_missing!(Brew, brew);

    check_missing!(Cargo, cargo);

    check_missing!(Choco, choco);

    check_missing!(Yum, yum);

    #[test]
    fn which_has_cargo() {
        assert!(which_has("cargo"));
    }

    #[test]
    fn which_not_has_fake() {
        assert!(!which_has("some_missing_package"));
    }

    fn all() -> BTreeSet<PackageManager> {
        vec![Apt, AptGet, Brew, Cargo, Choco, Yum]
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
                let mut map = BTreeMap::new();
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
            aliases: BTreeMap::new(),
        }
        .is_installed());
    }

    #[test]
    fn check_not_installed() {
        assert!(!Package {
            name: "some_missing_package".to_string(),
            managers: all(),
            aliases: BTreeMap::new()
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
        context.set_variable("namespace", "key", "value");
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
            vec![PackageManager::Brew].into_iter().collect()
        );
    }
}
