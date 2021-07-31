use super::error::DotsResult;
use lazy_static::lazy_static;
use log::{info, warn};
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

lazy_static! {
    static ref SHELL: Shell<'static> =
        match env::var("SHELL").expect("failed to read shell").as_ref() {
            "sh" => Shell::Sh,
            "bash" => Shell::Bash,
            "zsh" => Shell::Zsh,
            "pwsh" => Shell::Powershell,
            other => Shell::Other(Cow::Owned(other.to_string())),
        };
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct Package {
    name: String,
    managers: Vec<PackageManager>,
}

impl Package {
    pub fn is_installed(&self) -> bool {
        if which_has(&self.name) || dpkg_has(&self.name) {
            return true;
        }
        for pm in &self.managers {
            if pm.has(&self.name) {
                return true;
            }
        }
        false
    }

    pub fn install(&self) -> DotsResult<()> {
        if !self.is_installed() {
            for pm in &self.managers {
                if pm.is_available() {
                    return pm.install(&self.name);
                }
            }
            warn!("Package unavailable: {}", self.name);
        } else {
            info!("Package already installed: {}", self.name);
        }
        Ok(())
    }
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
        info!("Installing package ({} install {})", self.name(), package);
        self.command().arg("install").arg(package).output()?;
        Ok(())
    }

    fn _sudo_install(&self, package: &str) -> DotsResult<()> {
        info!(
            "Installing package (sudo {} install {})",
            self.name(),
            package
        );
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

    // Check if a package is installed
    pub fn has(&self, package: &str) -> bool {
        match self {
            Self::Brew => bool_command("brew", &["list", package]),
            _ => false,
        }
    }

    // Install the package manager and perform setup
    pub fn bootstrap(&self) -> DotsResult<()> {
        info!("Bootstrapping {}", self.name());
        match self {
            Self::Brew => Shell::Bash.remote_script(&[
                "-fsSL",
                "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
            ]),
            Self::Cargo => Shell::Sh.remote_script(&[
                "--proto",
                "'=https'",
                "--tlsv1.2",
                "-sSf",
                "https://sh.rustup.rs",
            ]),
            pm => Err(format!("bootstrap not supported for {}", pm.name()).into()),
        }
    }

    // Install the package manager if not already installed
    pub fn require(&self) -> DotsResult<()> {
        if self.is_available() {
            return Ok(());
        }
        self.bootstrap()
    }

    pub fn is_available(&self) -> bool {
        which_has(self.name())
    }
}

pub fn bool_command(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .expect(&format!("'{}' failed", cmd))
        .status
        .success()
}

// Check if a command is available locally
#[inline(always)]
pub fn which_has(cmd: &str) -> bool {
    bool_command("which", &[cmd])
}

#[inline(always)]
pub fn dpkg_has(cmd: &str) -> bool {
    bool_command("dpkg", &["-s", cmd])
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

    // Use curl to fetch remote script and pipe into shell
    pub fn remote_script(&self, curl_args: &[&str]) -> DotsResult<()> {
        info!("Running remote script with curl: {:?}", curl_args);
        let mut cmd_curl = Command::new("curl")
            .args(curl_args)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let mut cmd_sh = Command::new(self.path())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        if let Some(ref mut stdout) = cmd_curl.stdout {
            if let Some(ref mut stdin) = cmd_sh.stdin {
                let mut buf: Vec<u8> = Vec::new();
                stdout.read_to_end(&mut buf).unwrap();
                stdin.write_all(&buf).unwrap();
            }
        }
        match cmd_sh.wait_with_output()?.status.success() {
            true => Ok(()),
            false => Err("failed to execute remote script".into()),
        }
    }

    pub fn chsh(&self) -> DotsResult<()> {
        info!("Changing shell to: {}", self.path());
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
