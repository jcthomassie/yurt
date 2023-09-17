use crate::context::parse::{self, ObjectKey};
use crate::specs::{
    shell::{command, ShellCommand},
    BuildUnit, Context, Resolve,
};

use anyhow::{anyhow, Context as _, Result};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Package {
    name: String,
    #[serde(default = "Vec::new")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    managers: Vec<String>,
    #[serde(default = "IndexMap::new")]
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    aliases: IndexMap<String, String>,
}

impl Package {
    fn alias(&self, manager: &PackageManager) -> &String {
        self.aliases.get(&manager.name).unwrap_or(&self.name)
    }

    fn iter_managers<'a>(
        &'a self,
        context: &'a Context,
    ) -> impl Iterator<Item = &'a PackageManager> {
        self.managers
            .iter()
            .filter_map(|manager| context.managers.get(manager.as_str()))
    }

    pub fn is_installed(&self, context: &Context) -> bool {
        self.iter_managers(context).any(|manager| manager.has(self)) || which_has(&self.name)
    }

    pub fn install(&self, context: &Context) -> Result<()> {
        if self.is_installed(context) {
            log::info!("Package already installed: {}", self.name);
            Ok(())
        } else {
            for manager in self.iter_managers(context) {
                log::info!("Installing {} with {}", self.name, manager.name);
                match manager.install(self) {
                    Ok(_) => return Ok(()),
                    Err(error) => log::error!("{error}"),
                };
            }
            Err(anyhow!("Package unavailable: {}", self.name))
        }
    }

    pub fn uninstall(&self, context: &Context) -> Result<()> {
        for manager in self.iter_managers(context) {
            if manager.has(self) {
                manager.uninstall(self)?;
            }
        }
        Ok(())
    }
}

impl Resolve for Package {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Package(Self {
            name: context.parse_str(&self.name)?,
            managers: match self.managers.is_empty() {
                false => self
                    .managers
                    .into_iter()
                    .filter(|manager| context.managers.contains_key(manager.as_str()))
                    .collect(),
                true => context.managers.keys().map(ToString::to_string).collect(),
            },
            ..self
        }))
    }
}

impl ObjectKey for Package {
    const OBJECT_NAME: &'static str = "package";
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct PackageManager {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_bootstrap: Option<ShellCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_install: Option<ShellCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_uninstall: Option<ShellCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_has: Option<ShellCommand>,
}

impl PackageManager {
    fn inject_package(&self, command: &ShellCommand, package: &Package) -> Result<ShellCommand> {
        lazy_static! {
            static ref PACKAGE_KEY: parse::Key = Package::object_key("alias");
        }
        Ok(ShellCommand {
            shell: command.shell.clone(),
            command: parse::replace(&command.command, |input_key| {
                (input_key == *PACKAGE_KEY)
                    .then(|| package.alias(self).to_string())
                    .with_context(|| format!("Unexpected key: {input_key:?}"))
            })?,
        })
    }

    fn command<F, T>(
        &self,
        command: &Option<ShellCommand>,
        command_name: &str,
        command_action: F,
    ) -> Result<T>
    where
        F: Fn(&ShellCommand) -> Result<T>,
    {
        log::info!("Calling `{}.{command_name}`", self.name);
        command
            .as_ref()
            .with_context(|| format!("{}.{command_name} is not implemented", self.name))
            .and_then(command_action)
            .with_context(|| format!("{}.{command_name} failed", self.name))
    }

    /// Install the package manager
    pub fn bootstrap(&self) -> Result<()> {
        self.command(&self.shell_bootstrap, "shell_bootstrap", ShellCommand::exec)
    }

    /// Install a package
    pub fn install(&self, package: &Package) -> Result<()> {
        self.command(&self.shell_install, "shell_install", |command| {
            self.inject_package(command, package)
                .and_then(|command| command.exec())
        })
    }

    /// Uninstall a package
    pub fn uninstall(&self, package: &Package) -> Result<()> {
        self.command(&self.shell_uninstall, "shell_uninstall", |command| {
            self.inject_package(command, package)
                .and_then(|command| command.exec())
        })
    }

    /// Check if a package is installed
    pub fn has(&self, package: &Package) -> bool {
        self.command(&self.shell_has, "shell_has", |command| {
            self.inject_package(command, package)
                .and_then(|command| command.exec_bool())
        })
        .unwrap_or_else(|error| {
            log::warn!("{error}");
            false
        })
    }

    /// Check if package manager is installed
    pub fn is_available(&self) -> bool {
        which_has(&self.name)
    }

    /// Install the package manager if not already installed
    pub fn require(&self) -> Result<()> {
        match self.is_available() {
            true => Ok(()),
            false => self.bootstrap(),
        }
    }
}

impl Resolve for PackageManager {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        context.managers.insert(self.name.clone(), self.clone());
        Ok(BuildUnit::PackageManager(self))
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

    macro_rules! unpack {
        ($value:expr, BuildUnit::$variant:ident) => {
            if let BuildUnit::$variant(ref unwrapped) = $value {
                unwrapped
            } else {
                panic!("Failed to unpack build unit");
            }
        };
    }

    mod manager {
        use super::*;

        fn package_manager(name: &str) -> PackageManager {
            PackageManager {
                name: name.to_string(),
                shell_bootstrap: None,
                shell_install: None,
                shell_uninstall: None,
                shell_has: None,
            }
        }
        #[test]
        fn empty_bootstrap() {
            package_manager("made-up-name").bootstrap().unwrap_err();
        }

        #[test]
        fn alias() {
            let not_aliased = package_manager("not_aliased");
            let aliased = package_manager("aliased");
            let package = Package {
                name: "name".to_string(),
                managers: vec![aliased.name.clone()],
                aliases: {
                    let mut map = IndexMap::new();
                    map.insert(aliased.name.clone(), "alias".into());
                    map
                },
            };
            assert_eq!(package.alias(&aliased), "alias");
            assert_eq!(package.alias(&not_aliased), "name");
        }

        #[test]
        fn prune_empty() {
            let package: Package = serde_yaml::from_str("name: some-package").unwrap();
            let mut context = Context::default();
            // No managers remain
            let resolved = package.resolve(&mut context).unwrap();
            let package = unpack!(resolved, BuildUnit::Package);
            assert!(package.managers.is_empty());
        }

        #[test]
        fn prune_some() {
            #[rustfmt::skip]
            let package: Package = serde_yaml::from_str("
                name: some-package
                managers: [ apt, brew ]
            ").unwrap();
            // Add partially overlapping managers
            let mut context = Context::default();
            context
                .managers
                .insert("cargo".into(), package_manager("cargo"));
            context
                .managers
                .insert("brew".into(), package_manager("brew"));
            // Overlap remains
            let resolved = package.resolve(&mut context).unwrap();
            let package = unpack!(resolved, BuildUnit::Package);
            assert_eq!(package.managers, vec!["brew"]);
        }

        #[test]
        fn bootstrap_not_implemented() {
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            package_manager.bootstrap().unwrap_err();
        }

        #[test]
        fn install_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            package_manager.install(&package).unwrap_err();
        }

        #[test]
        fn uninstall_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            package_manager.uninstall(&package).unwrap_err();
        }

        #[test]
        fn has_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            assert!(!package_manager.has(&package));
        }

        #[test]
        fn has_package_not_installed() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            #[rustfmt::skip]
            let package_manager: PackageManager = serde_yaml::from_str("
                name: cargo
                shell_has: cargo install --list | grep '^${{ package }} v'
            ").unwrap();
            assert!(!package_manager.has(&package));
        }
    }

    #[test]
    fn which_has_cargo() {
        assert!(which_has("cargo"));
    }

    #[test]
    fn which_not_has_fake() {
        assert!(!which_has("some_missing_package"));
    }

    #[test]
    fn check_installed() {
        let context = Context::default();
        assert!(Package {
            name: "cargo".to_string(),
            managers: context.managers.keys().cloned().collect(),
            aliases: IndexMap::new(),
        }
        .is_installed(&context));
    }

    #[test]
    fn check_not_installed() {
        let context = Context::default();
        assert!(!Package {
            name: "some_missing_package".to_string(),
            managers: context.managers.keys().cloned().collect(),
            aliases: IndexMap::new()
        }
        .is_installed(&context));
    }

    #[test]
    fn package_name_substitution() {
        let package: Package = serde_yaml::from_str("name: ${{ key }}").unwrap();
        let mut context = Context::default();
        context.variables.try_push("key", "value").unwrap();
        // No managers remain
        let resolved = package.resolve(&mut context).unwrap();
        let package = unpack!(resolved, BuildUnit::Package);
        assert_eq!(package.name, "value");
    }
}
