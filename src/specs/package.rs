use std::{fmt, process::Command};

use anyhow::{anyhow, Context as _, Result};
use console::style;
use indexmap::IndexMap;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use super::{BuildUnitInterface, BuildUnitKind};
use crate::{
    context::parse::{self, ObjectKey},
    specs::{
        shell::{command, ShellCommand},
        BuildUnit, Context, Resolve,
    },
    yaml_example_doc,
};

/// Installable binary package.
#[doc = yaml_example_doc!("package.yaml")]
#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Package {
    /// Primary identifier of the package
    name: String,
    /// Subset of [`!package_manager`][PackageManager] used to manage the package
    #[serde(default = "Vec::new")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    managers: Vec<String>,
    /// Map of identifier overrides for certain [`!package_manager`][PackageManager]
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
        self.iter_managers(context)
            .any(|manager| manager.has(self, context))
            || which_has(&self.name)
    }
}

impl BuildUnitInterface for Package {
    fn unit_install(&self, context: &Context) -> Result<bool> {
        let progress = context
            .progress_task()
            .with_prefix("Package")
            .with_message(format!("{} {}", self.name, style("installing").dim()));
        if self.is_installed(context) {
            Ok(false)
        } else {
            for manager in self.iter_managers(context) {
                progress.set_message(format!(
                    "{} {} {}",
                    self.name,
                    style("installing with").dim(),
                    manager.name
                ));
                match manager.install(self, context) {
                    Ok(()) => return Ok(true),
                    Err(error) => context.write_error("Package", &self.name, error)?,
                };
            }
            Err(anyhow!("Package unavailable: {}", self.name))
        }
    }

    fn unit_uninstall(&self, context: &Context) -> Result<bool> {
        let mut uninstalled = false;
        for manager in self.iter_managers(context) {
            if manager.has(self, context) {
                manager.uninstall(self, context)?;
                uninstalled = true;
            }
        }
        Ok(uninstalled)
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

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Command line package manager.
#[doc = yaml_example_doc!("package_manager.yaml")]
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct PackageManager {
    /// Identifier referenced from [`!package`][Package]
    name: String,
    /// Command to self-install if unavailable
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_bootstrap: Option<ShellCommand>,
    /// Command to install a [`!package`][Package]
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_install: Option<ShellCommand>,
    /// Command to uninstall a [`!package`][Package]
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_uninstall: Option<ShellCommand>,
    /// Command to check if a [`!package`][Package] is already installed
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_has: Option<ShellCommand>,
}

impl PackageManager {
    /// Inject the alias of `package` into `command`.
    /// ```
    /// "apt install ${{ package.alias }}" -> "apt install my-package-alias"
    /// ```
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
        context: &Context,
        command: &Option<ShellCommand>,
        command_name: &str,
        command_action: F,
    ) -> Result<T>
    where
        F: Fn(&ShellCommand) -> Result<T>,
    {
        context
            .progress_task()
            .with_prefix("Executing")
            .with_message(format!("{}.{command_name}", self.name));
        command
            .as_ref()
            .with_context(|| format!("{}.{command_name} is not implemented", self.name))
            .and_then(command_action)
            .with_context(|| format!("{}.{command_name} failed", self.name))
    }

    /// Install `package` by running [`shell_install`][Self::shell_install]
    pub fn install_package(&self, package: &Package, context: &Context) -> Result<()> {
        self.command(context, &self.shell_install, "shell_install", |command| {
            self.inject_package(command, package)
                .and_then(|command| command.exec())
        })
    }

    /// Uninstall `package` by running [`shell_uninstall`][Self::shell_uninstall]
    pub fn uninstall_package(&self, package: &Package, context: &Context) -> Result<()> {
        self.command(
            context,
            &self.shell_uninstall,
            "shell_uninstall",
            |command| {
                self.inject_package(command, package)
                    .and_then(|command| command.exec())
            },
        )
    }

    /// Check if `package` is installed by running [`shell_has`][Self::shell_has]
    pub fn has(&self, package: &Package, context: &Context) -> bool {
        self.command(context, &self.shell_has, "shell_has", |command| {
            self.inject_package(command, package)
                .and_then(|command| command.exec_bool())
        })
        .map_err(|error| context.write_error(BuildUnitKind::PackageManager, self, error))
        .unwrap_or(false)
    }

    /// Install the package manager by running [`shell_bootstrap`][Self::shell_bootstrap]
    pub fn bootstrap(&self, context: &Context) -> Result<()> {
        self.command(
            context,
            &self.shell_bootstrap,
            "shell_bootstrap",
            ShellCommand::exec,
        )
    }

    /// Check if package manager is installed
    pub fn is_available(&self) -> bool {
        which_has(&self.name)
    }
}

impl BuildUnitInterface for PackageManager {
    fn unit_install(&self, context: &Context) -> Result<bool> {
        if self.is_available() {
            Ok(false)
        } else {
            self.bootstrap(context)?;
            Ok(true)
        }
    }

    fn unit_uninstall(&self, _context: &Context) -> Result<bool> {
        Ok(false)
    }
}

impl Resolve for PackageManager {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        context.managers.insert(self.name.clone(), self.clone());
        Ok(BuildUnit::PackageManager(self))
    }
}

impl fmt::Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Check if a command is available locally
fn which_has(name: &str) -> bool {
    #[cfg(unix)]
    let mut cmd = Command::new("which");
    #[cfg(windows)]
    let mut cmd = Command::new("where");
    command::call_bool(cmd.arg(name)).unwrap_or(false)
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
            package_manager("made-up-name")
                .bootstrap(&Context::default())
                .unwrap_err();
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
            package_manager.bootstrap(&Context::default()).unwrap_err();
        }

        #[test]
        fn install_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            package_manager
                .install(&package, &Context::default())
                .unwrap_err();
        }

        #[test]
        fn uninstall_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            package_manager
                .uninstall(&package, &Context::default())
                .unwrap_err();
        }

        #[test]
        fn has_not_implemented() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            let package_manager: PackageManager =
                serde_yaml::from_str("name: arbitrary_manager").unwrap();
            assert!(!package_manager.has(&package, &Context::default()));
        }

        #[test]
        fn has_package_not_installed() {
            let package: Package = serde_yaml::from_str("name: arbitrary_package").unwrap();
            #[rustfmt::skip]
            let package_manager: PackageManager = serde_yaml::from_str("
                name: cargo
                shell_has: cargo install --list | grep '^${{ package }} v'
            ").unwrap();
            assert!(!package_manager.has(&package, &Context::default()));
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
