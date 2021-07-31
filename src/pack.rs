use super::error::DotsResult;
use lazy_static::lazy_static;
use log::{info, warn};
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::io::{Read, Write};
use std::mem::discriminant;
use std::process::{Command, Stdio};

lazy_static! {
    static ref SHELL: Shell<'static> = match env::var("SHELL")
        .expect("failed to read shell")
        .split("/")
        .last()
    {
        Some("sh") => Shell::Sh,
        Some("bash") => Shell::Bash,
        Some("zsh") => Shell::Zsh,
        Some("pwsh") => Shell::Powershell,
        Some(other) => Shell::Other(Cow::Owned(other.to_string())),
        _ => unreachable!(),
    };
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct Package {
    name: String,
    alias: Option<String>,
    managers: Vec<PackageManager>,
}

impl Package {
    fn _is_installed(&self, name: &str) -> bool {
        if which_has(name) || dpkg_has(name) {
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

    pub fn install(&self) -> DotsResult<()> {
        if !self.is_installed() {
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
        self.command()
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .arg("install")
            .arg(package)
            .output()?;
        Ok(())
    }

    fn _sudo_install(&self, package: &str) -> DotsResult<()> {
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

#[derive(PartialEq)]
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
        info!("Current shell: {}", &*SHELL.path());
        if discriminant(self) == discriminant(&*SHELL) {
            return Ok(());
        }
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
