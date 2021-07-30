use super::error::DotsResult;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::process::{Command, Stdio};

lazy_static! {
    static ref SH: Shell<'static> = match env::var("SHELL").expect("failed to read shell").as_ref()
    {
        "sh" => Shell::Sh,
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "pwsh" => Shell::Powershell,
        other => Shell::Other(Cow::Owned(other.to_string())),
    };
}

#[inline(always)]
pub fn install(packages: Vec<Package>) -> DotsResult<()> {
    packages.iter().map(|p| p.install()).collect()
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Package {
    name: String,
    managers: Vec<PackageManager>,
}

impl Package {
    pub fn is_installed(&self) -> bool {
        false
    }

    pub fn install(&self) -> DotsResult<()> {
        if !self.is_installed() {
            for pm in self.managers.iter() {
                if pm.is_available() {
                    return pm.install(&self.name);
                }
            }
        }
        // TODO: return error when no installer is available
        Ok(())
    }
}

#[derive(Debug, PartialEq, Deserialize)]
enum PackageManager {
    Apt,
    AptGet,
    Brew,
    Cargo,
    Choco,
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
            Self::Yum => "yum",
        }
    }

    #[inline(always)]
    fn command(&self) -> Command {
        Command::new(self.name())
    }

    fn _install(&self, package: &str) -> DotsResult<()> {
        self.command().arg("install").arg(package).output()?;
        Ok(())
    }

    fn _sudo_install(&self, package: &str) -> DotsResult<()> {
        Command::new("sudo")
            .arg(self.name())
            .arg("install")
            .arg(package)
            .output()?;
        Ok(())
    }

    // Install a package
    pub fn install(&self, package: &str) -> DotsResult<()> {
        match self {
            Self::Apt => self._sudo_install(package),
            Self::AptGet => self._sudo_install(package),
            Self::Yum => self._sudo_install(package),
            _ => self._install(package),
        }
    }

    // Install the package manager and perform setup
    pub fn bootstrap(&self) -> DotsResult<()> {
        match self {
            Self::Apt => Ok(()),
            Self::AptGet => Ok(()),
            Self::Brew => Ok(()),
            Self::Cargo => Ok(()),
            Self::Choco => Ok(()),
            Self::Yum => Ok(()),
        }
    }

    // Check if the package manager is available locally
    pub fn is_available(&self) -> bool {
        false
    }
}

pub enum Shell<'a> {
    Sh,
    Bash,
    Zsh,
    Powershell,
    Other(Cow<'a, str>),
}

impl<'a> Shell<'a> {
    #[inline(always)]
    fn path(&self) -> &str {
        match self {
            Self::Sh => "sh",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "pwsh",
            Self::Other(name) => name,
        }
    }

    fn command(&self) -> Command {
        Command::new(self.path())
    }

    pub fn chsh(&self) -> DotsResult<()> {
        let output = Command::new("which")
            .arg(self.path())
            .stdout(Stdio::piped())
            .output()
            .expect("Failed to locate shell");
        let path = String::from_utf8_lossy(&output.stdout);
        Command::new("chsh")
            .arg("-s")
            .arg(path.as_ref())
            .output()
            .expect("Failed to change shell");
        Ok(())
    }
}
