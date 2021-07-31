use super::error::DotsResult;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use regex::Regex;
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::mem::discriminant;
use std::process::{Child, Command, Output, Stdio};

pub use Shell::{Bash, Powershell, Sh, Zsh};

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

#[derive(Debug, PartialEq, Deserialize)]
pub struct Source(Vec<String>);

impl Source {
    // Resolve source file paths
    pub fn resolve(&self) -> DotsResult<Self> {
        let mut vec: Vec<String> = Vec::with_capacity(self.0.len());
        for path in &self.0 {
            vec.push(shellexpand::full(path)?.to_string())
        }
        Ok(Source(vec))
    }

    // Reload all of the source files
    pub fn reload(&self) -> DotsResult<()> {
        let re = Regex::new(r#"export ([0-9A-Za-z_-]+)=([0-9A-Za-z_"-:{}\$]+)[;$]?"#).unwrap();
        for path in &self.0 {
            let file = File::open(path)?;
            for line in BufReader::new(file).lines() {
                for cap in re.captures_iter(&line?) {
                    let k = &cap[1];
                    let v = shellexpand::full(&cap[2]);
                    match v {
                        Ok(v) => {
                            debug!("Setting ${} = {}", k, v);
                            env::set_var(k, v.as_ref());
                        }
                        Err(e) => warn!("Failed setting ${}; {:?}", k, e),
                    };
                }
            }
        }
        Ok(())
    }
}

trait Cmd {
    fn name(&self) -> &str;

    #[inline(always)]
    fn command(&self) -> Command {
        Command::new(self.name())
    }

    #[inline(always)]
    fn call(&self, args: &[&str]) -> DotsResult<Output> {
        debug!("Calling command: {} {:?}", self.name(), args);
        Ok(self.command().args(args).output()?)
    }

    #[inline(always)]
    fn call_bool(&self, args: &[&str]) -> bool {
        self.call(args)
            .expect(&format!("'{}' failed", self.name()))
            .status
            .success()
    }

    #[inline(always)]
    fn child(&self, args: &[&str]) -> DotsResult<Child> {
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

fn pipe_existing(mut proc_a: Child, mut proc_b: Child) -> DotsResult<()> {
    if let Some(ref mut stdout) = proc_a.stdout {
        if let Some(ref mut stdin) = proc_b.stdin {
            let mut buf: Vec<u8> = Vec::new();
            stdout.read_to_end(&mut buf).unwrap();
            stdin.write_all(&buf).unwrap();
        }
    }
    match proc_b.wait_with_output()?.status.success() {
        true => Ok(()),
        false => Err("failed to execute piped command".into()),
    }
}

fn pipe<T, U>(cmd_a: T, args_a: &[&str], cmd_b: U, args_b: &[&str]) -> DotsResult<()>
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
            Self::Brew => self.call_bool(&["list", package]),
            _ => false,
        }
    }

    // Install the package manager and perform setup
    pub fn bootstrap(&self) -> DotsResult<()> {
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

// Check if a command is available locally
#[inline(always)]
pub fn which_has(cmd: &str) -> bool {
    "which".call_bool(&[cmd])
}

#[inline(always)]
pub fn dpkg_has(cmd: &str) -> bool {
    "dpkg".call_bool(&["-s", cmd])
}

#[derive(PartialEq)]
pub enum Shell<'a> {
    Sh,
    Bash,
    Zsh,
    Powershell,
    Other(Cow<'a, str>),
}

impl<'a> Cmd for Shell<'a> {
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

impl<'a> Shell<'a> {
    // Use curl to fetch remote script and pipe into shell
    pub fn remote_script(self, curl_args: &[&str]) -> DotsResult<()> {
        info!("Running remote script");
        pipe("curl", curl_args, self, &[])
    }

    // Set self as the default system shell
    pub fn chsh(&self) -> DotsResult<()> {
        info!("Current shell: {}", &*SHELL.name());
        if discriminant(self) == discriminant(&*SHELL) {
            return Ok(());
        }
        info!("Changing shell to: {}", self.name());
        pipe("which", &[], "chsh", &["-s"])
    }
}
