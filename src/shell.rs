use anyhow::{anyhow, Context, Result};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::{Read, Write},
    process::{Child, Command, Output, Stdio},
};

pub use Shell::{Bash, Powershell, Sh, Zsh};

pub trait Cmd {
    fn name(&self) -> &str;

    #[inline]
    fn command(&self) -> Command {
        Command::new(self.name())
    }

    #[inline]
    fn call_unchecked(&self, args: &[&str]) -> Result<Output> {
        debug!("Calling command: `{} {}`", self.name(), args.join(" "));
        Ok(self.command().args(args).output()?)
    }

    #[inline]
    fn call(&self, args: &[&str]) -> Result<()> {
        match self.call_unchecked(args)?.status.success() {
            true => Ok(()),
            false => Err(anyhow!(
                "Failed command: `{} {}`",
                self.name(),
                args.join(" ")
            )),
        }
    }

    #[inline]
    fn call_bool(&self, args: &[&str]) -> Result<bool> {
        Ok(self.call_unchecked(args)?.status.success())
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
    fn run(&self, shell: &Shell) -> Result<Output>;
}

impl ShellCmd for &str {
    fn run(&self, shell: &Shell) -> Result<Output> {
        shell.call_unchecked(&["-c", self])
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
        Err(anyhow!("Piped command returned error"))
    }
}

fn pipe<T, U>(cmd_a: T, args_a: &[&str], cmd_b: U, args_b: &[&str]) -> Result<Output>
where
    T: Cmd,
    U: Cmd,
{
    debug!(
        "Calling command: `{} {} | {} {}`",
        cmd_a.name(),
        args_a.join(" "),
        cmd_b.name(),
        args_b.join(" ")
    );
    let proc_a = cmd_a
        .command()
        .args(args_a)
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to spawn primary pipe command")?;
    let proc_b = cmd_b
        .command() //
        .args(args_b)
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to spawn secondary pipe command")?;
    pipe_existing(proc_a, proc_b).with_context(|| {
        format!(
            "Failed command: `{} {} | {} {}`",
            cmd_a.name(),
            args_a.join(" "),
            cmd_b.name(),
            args_b.join(" ")
        )
    })
}

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
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

    #[cfg(target_os = "macos")]
    fn default() -> Shell {
        Shell::Zsh
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let shell = Shell::from_env();
        match env::var("SHELL") {
            // Other shells should be added as needed
            Ok(_) => assert!(!matches!(shell, Shell::Other(_))),
            Err(_) => assert_eq!(shell, Shell::default()),
        }
    }

    #[test]
    fn shell_command_success() {
        let out = "ls ~".run(&Shell::default()).unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn shell_command_failure() {
        let out = "made_up_command with parameters"
            .run(&Shell::default())
            .unwrap();
        assert!(!out.status.success());
    }
}
