use super::error::DotsResult;
use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
const PM: PackageManager = PackageManager::Brew;
#[cfg(target_os = "windows")]
const PM: PackageManager = PackageManager::Choco;
#[cfg(target_os = "macos")]
const PM: PackageManager = PackageManager::Brew;

// TODO: read environment variable
static SH: Shell = Shell::Zsh;

enum PackageManager {
    Brew,
    Cargo,
    Choco,
    Yum,
    Apt,
    AptGet,
}

impl PackageManager {
    fn name(&self) -> &str {
        match self {
            Self::Brew => "brew",
            Self::Cargo => "cargo",
            Self::Choco => "choco",
            Self::Yum => "yum",
            Self::Apt => "apt",
            Self::AptGet => "apt-get",
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
            Self::Yum => self._sudo_install(package),
            Self::Apt => self._sudo_install(package),
            Self::AptGet => self._sudo_install(package),
            _ => self._install(package),
        }
    }

    // Install the package manager and perform setup
    pub fn bootstrap(&self) -> DotsResult<()> {
        match self {
            Self::Brew => Ok(()),
            Self::Cargo => Ok(()),
            Self::Choco => Ok(()),
            Self::Yum => Ok(()),
            Self::Apt => Ok(()),
            Self::AptGet => Ok(()),
        }
    }
}

pub enum Shell {
    Sh,
    Bash,
    Zsh,
    Powershell,
}

impl Shell {
    #[inline(always)]
    fn path(&self) -> &str {
        match self {
            Self::Sh => "sh",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "pwsh",
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

    pub fn install(&self) -> DotsResult<()> {
        match self {
            Self::Sh => Ok(()),
            Self::Bash => Ok(()),
            Self::Zsh => Ok(()),
            Self::Powershell => Ok(()),
        }
    }
}

// TODO
pub fn install_essential() -> DotsResult<()> {
    PM.bootstrap()?;
    PM.install("zsh").unwrap();
    PM.install("cargo").unwrap();
    PM.install("git").unwrap();
    PM.install("git-delta").unwrap();
    PM.install("bat").unwrap();
    Ok(())
}
