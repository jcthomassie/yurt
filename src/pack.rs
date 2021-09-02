use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use log::{debug, info, warn};
use serde::Deserialize;
use std::env;
use std::io::{Read, Write};
use std::process::{Child, Command, Output, Stdio};

pub use PackageManager::{Apt, AptGet, Brew, Cargo, Choco, Yum};
pub use Shell::{Bash, Powershell, Sh, Zsh};

lazy_static! {
    pub static ref SHELL: Shell = Shell::from_env();
}

pub trait Cmd {
    fn name(&self) -> &str;

    #[inline]
    fn command(&self) -> Command {
        Command::new(self.name())
    }

    #[inline]
    fn call(&self, args: &[&str]) -> Result<Output> {
        debug!("Calling command: {} {:?}", self.name(), args);
        Ok(self.command().args(args).output()?)
    }

    #[inline]
    fn call_bool(&self, args: &[&str]) -> Result<bool> {
        Ok(self.call(args)?.status.success())
    }

    #[inline]
    fn child(&self, args: &[&str]) -> Result<Child> {
        Ok(self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?)
    }
}

impl Cmd for &str {
    fn name(&self) -> &str {
        self
    }
}

pub trait ShellCmd {
    fn run(&self) -> Result<Output>;
}

impl ShellCmd for &str {
    fn run(&self) -> Result<Output> {
        SHELL.call(&["-c", self])
    }
}

fn pipe_existing(mut proc_a: Child, mut proc_b: Child) -> Result<Output> {
    if let Some(ref mut stdout) = proc_a.stdout {
        if let Some(ref mut stdin) = proc_b.stdin {
            let mut buf: Vec<u8> = Vec::new();
            stdout.read_to_end(&mut buf).unwrap();
            stdin.write_all(&buf).unwrap();
        }
    }
    let output = proc_b.wait_with_output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(anyhow!("failed to execute piped command"))
    }
}

fn pipe<T, U>(cmd_a: T, args_a: &[&str], cmd_b: U, args_b: &[&str]) -> Result<Output>
where
    T: Cmd,
    U: Cmd,
{
    let proc_a = cmd_a
        .command()
        .args(args_a)
        .stdout(Stdio::piped())
        .spawn()?;
    let proc_b = cmd_b
        .command() //
        .args(args_b)
        .stdin(Stdio::piped())
        .spawn()?;
    pipe_existing(proc_a, proc_b)
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Package {
    pub name: String,
    pub alias: Option<String>,
    pub managers: Vec<PackageManager>,
}

impl Package {
    pub fn new(name: String, managers: Vec<PackageManager>) -> Self {
        Package {
            name,
            alias: None,
            managers,
        }
    }

    fn _is_installed(&self, name: &str) -> bool {
        if which_has(name) {
            return true;
        }
        for pm in &self.managers {
            if pm.has(name) {
                return true;
            }
        }
        false
    }

    pub fn is_installed(&self) -> bool {
        self._is_installed(&self.name)
            || match &self.alias {
                Some(alias) => self._is_installed(alias),
                None => false,
            }
    }

    pub fn install(&self) -> Result<()> {
        if self.is_installed() {
            info!("Package already installed: {}", self.name);
        } else {
            let alias = self.alias.clone();
            for pm in &self.managers {
                if pm.is_available() {
                    let res = pm.install(&self.name);
                    return match alias {
                        Some(a) if res.is_err() => pm.install(a.as_str()),
                        _ => res,
                    };
                }
            }
            warn!("Package unavailable: {}", self.name);
        }
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        if self.is_installed() {
            for pm in &self.managers {
                if pm.has(&self.name) {
                    pm.uninstall(&self.name)?;
                    continue;
                }
                if matches!(self.alias, Some(ref a) if pm.has(a)) {
                    pm.uninstall(&self.alias.clone().unwrap())?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct PackageBundle {
    pub manager: PackageManager,
    pub packages: Vec<String>,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
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
    fn _install(&self, package: &str) -> Result<()> {
        info!("Installing package ({} install {})", self.name(), package);
        self.command()
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .arg("install")
            .arg(package)
            .output()?;
        Ok(())
    }

    fn _sudo_install(&self, package: &str) -> Result<()> {
        info!(
            "Installing package (sudo {} install {})",
            self.name(),
            package
        );
        Command::new("sudo")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .arg(self.name())
            .arg("install")
            .arg(package)
            .output()?;
        Ok(())
    }

    // Install a package
    pub fn install(&self, package: &str) -> Result<()> {
        match self {
            Self::Apt | Self::AptGet | Self::Yum => self._sudo_install(package),
            _ => self._install(package),
        }
    }

    // Uninstall a package
    pub fn uninstall(&self, package: &str) -> Result<()> {
        info!(
            "Uninstalling package ({} uninstall {})",
            self.name(),
            package
        );
        self.command()
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .arg("uninstall")
            .arg(package)
            .output()?;
        Ok(())
    }

    // Check if a package is installed
    pub fn has(&self, package: &str) -> bool {
        let res = match self {
            Self::Apt | Self::AptGet => "dpkg".call_bool(&["-l", package]),
            Self::Brew => self.call_bool(&["list", package]),
            Self::Cargo => pipe(
                "cargo",
                &["install", "--list"],
                "grep",
                &[&format!("    {}$", package)],
            )
            .map(|o| o.status.success()),
            _ => Ok(false),
        };
        match res {
            Ok(has) => has,
            Err(_) => {
                warn!("{} failed to check for package", self.name());
                false
            }
        }
    }

    // Install the package manager and perform setup
    pub fn bootstrap(&self) -> Result<()> {
        info!("Bootstrapping {}", self.name());
        match self {
            Self::Brew => Bash.remote_script(&[
                "-fsSL",
                "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
            ]),
            Self::Cargo => Sh.remote_script(&[
                "--proto",
                "'=https'",
                "--tlsv1.2",
                "-sSf",
                "https://sh.rustup.rs",
            ]),
            pm => Err(anyhow!("bootstrap not supported for {}", pm.name())),
        }
    }

    // Install the package manager if not already installed
    pub fn require(&self) -> Result<()> {
        if self.is_available() {
            return Ok(());
        }
        self.bootstrap()
    }

    pub fn is_available(&self) -> bool {
        which_has(self.name())
    }
}

// Check if a command is available locally
#[inline]
pub fn which_has(cmd: &str) -> bool {
    #[cfg(not(target_os = "windows"))]
    let name = "which";
    #[cfg(target_os = "windows")]
    let name = "where";
    match name.call_bool(&[cmd]) {
        Ok(has) => has,
        Err(e) => {
            warn!("'{}' failed for {}: {}", name, cmd, e);
            false
        }
    }
}

#[derive(PartialEq, Debug, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Shell {
    Sh,
    Bash,
    Zsh,
    Powershell,
    Other(String),
}

impl Cmd for Shell {
    fn name(&self) -> &str {
        match self {
            Self::Sh => "sh",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "pwsh",
            Self::Other(name) => name,
        }
    }
}

impl Default for Shell {
    #[cfg(target_os = "windows")]
    fn default() -> Shell {
        Shell::Powershell
    }

    #[cfg(not(target_os = "windows"))]
    fn default() -> Shell {
        Shell::Sh
    }
}

impl Shell {
    pub fn from_env() -> Self {
        match env::var("SHELL") {
            Ok(s) => Self::from_name(s.split('/').last().unwrap()),
            Err(_) => Self::default(),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "sh" => Self::Sh,
            "bash" => Self::Bash,
            "zsh" => Self::Zsh,
            "pwsh" => Self::Powershell,
            other => Self::Other(other.to_string()),
        }
    }

    // Use curl to fetch remote script and pipe into shell
    pub fn remote_script(self, curl_args: &[&str]) -> Result<()> {
        info!("Running remote script");
        pipe("curl", curl_args, self, &[])?;
        Ok(())
    }

    // Set self as the default system shell
    #[cfg(not(target_os = "windows"))]
    pub fn chsh(&self) -> Result<()> {
        info!("Current shell: {}", SHELL.name());
        if std::mem::discriminant(self) == std::mem::discriminant(&SHELL) {
            return Ok(());
        }
        info!("Changing shell to: {}", self.name());
        pipe("which", &[], "chsh", &["-s"])?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    pub fn chsh(&self) -> Result<()> {
        warn!("Shell change not implemented for windows");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ALL_PMS: [PackageManager; 6] = [Apt, AptGet, Brew, Cargo, Choco, Yum];

    macro_rules! check_missing {
        ($pm:ident, $mod_name:ident) => {
            mod $mod_name {
                use super::*;

                #[test]
                fn not_has_fake() {
                    assert!(!$pm.has("some_missing_package"));
                }

                #[test]
                fn not_has_empty() {
                    assert!(!$pm.has(""));
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
    fn shell_from_name() {
        assert_eq!(Shell::from_name("sh"), Shell::Sh);
        assert_eq!(Shell::from_name("bash"), Shell::Bash);
        assert_eq!(Shell::from_name("zsh"), Shell::Zsh);
        assert_eq!(Shell::from_name("pwsh"), Shell::Powershell);
        assert!(matches!(Shell::from_name("/home/crush"), Shell::Other(_)));
    }

    #[test]
    fn shell_from_env() {
        match env::var("SHELL") {
            // Other shells should be added as needed
            Ok(_) => assert!(!matches!(*SHELL, Shell::Other(_))),
            Err(_) => assert_eq!(*SHELL, Shell::default()),
        }
    }

    #[test]
    fn shell_command_success() {
        let out = "ls ~".run().unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn shell_command_failure() {
        let out = "made_up_command with parameters".run().unwrap();
        assert!(!out.status.success());
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
    fn package_check_success() {
        assert!(Package::new("cargo".to_string(), ALL_PMS.to_vec()).is_installed());
    }

    #[test]
    fn package_check_failure() {
        assert!(!Package::new("some_missing_package".to_string(), ALL_PMS.to_vec()).is_installed());
    }
}
