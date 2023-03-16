use crate::specs::{BuildUnit, Context, Resolve};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{env, ffi::OsStr, path::Path, process::Command};

pub mod command {
    use anyhow::{Context as _, Result};
    use std::process::{Command, Output, Stdio};

    fn check_output(output: &Output, command_tag: impl std::fmt::Debug) -> Result<()> {
        output
            .status
            .success()
            .then_some(())
            .with_context(|| format!("stderr: {}", String::from_utf8_lossy(&output.stderr)))
            .with_context(|| match output.status.code() {
                Some(c) => format!("Command exited with status code {c}: `{command_tag:?}`"),
                None => format!("Command terminated by signal: `{command_tag:?}`"),
            })
    }

    pub fn call_unchecked(command: &mut Command) -> Result<Output> {
        log::debug!("Calling command: `{command:?}`");
        command
            .output()
            .with_context(|| format!("Failed to run command: `{command:?}`"))
    }

    #[inline]
    pub fn call_bool(command: &mut Command) -> Result<bool> {
        call_unchecked(command).map(|out| out.status.success())
    }

    #[inline]
    pub fn call(command: &mut Command) -> Result<()> {
        call_unchecked(command).and_then(|out| check_output(&out, command))
    }

    pub fn pipe_unchecked(cmd_a: &mut Command, cmd_b: &mut Command) -> Result<Output> {
        log::debug!("Calling piped command: `{cmd_a:?} | {cmd_b:?}`");
        let mut proc_a = cmd_a
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn primary pipe command")?;
        let pipe = proc_a.stdout.take().context("Failed to create pipe")?;
        let proc_b = cmd_b
            .stdin(pipe)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn secondary pipe command")?;
        proc_b
            .wait_with_output()
            .with_context(|| format!("Failed to run piped command: `{cmd_a:?} | {cmd_b:?}`"))
    }

    #[inline]
    pub fn pipe(cmd_a: &mut Command, cmd_b: &mut Command) -> Result<()> {
        pipe_unchecked(cmd_a, cmd_b)
            .and_then(|out| check_output(&out, format!("{cmd_a:?} | {cmd_b:?}")))
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum ShellKind {
    Sh,
    Bash,
    Zsh,
    Powershell,
    Cmd,
    Other,
    Empty,
}

impl From<&Path> for ShellKind {
    fn from(command: &Path) -> Self {
        match command.file_stem().and_then(OsStr::to_str) {
            Some("sh") => Self::Sh,
            Some("bash") => Self::Bash,
            Some("zsh") => Self::Zsh,
            Some("pwsh") => Self::Powershell,
            Some("cmd") => Self::Cmd,
            Some("") | None => Self::Empty,
            _ => Self::Other,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(from = "String", into = "String")]
pub struct Shell {
    kind: ShellKind,
    command: String,
}

impl Shell {
    pub fn from_env() -> Self {
        match env::var("SHELL") {
            Ok(s) => Self::from(s),
            Err(_) => Self::default(),
        }
    }

    #[inline]
    fn _exec(&self, command: &str) -> Command {
        let mut cmd = Command::new(&self.command);
        cmd.arg(match self.kind {
            ShellKind::Cmd => "/C",
            _ => "-c",
        })
        .arg(command);
        cmd
    }

    pub fn exec(&self, command: &str) -> Result<()> {
        command::call(&mut self._exec(command))
    }

    pub fn exec_bool(&self, command: &str) -> Result<bool> {
        command::call_bool(&mut self._exec(command))
    }

    /// Use curl to fetch remote script and pipe into shell
    #[inline]
    pub fn exec_remote(&self, curl_args: &[&str]) -> Result<()> {
        command::pipe(
            Command::new("curl").args(curl_args),
            &mut Command::new(&self.command),
        )
    }
}

impl Default for Shell {
    #[cfg(target_os = "windows")]
    fn default() -> Self {
        Self::from("cmd")
    }

    #[cfg(target_os = "macos")]
    fn default() -> Self {
        Self::from("zsh")
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn default() -> Self {
        Self::from("sh")
    }
}

impl From<String> for Shell {
    fn from(command: String) -> Self {
        Self {
            kind: ShellKind::from(Path::new(&command)),
            command,
        }
    }
}

impl From<&str> for Shell {
    fn from(command: &str) -> Self {
        Self::from(command.to_string())
    }
}

impl From<Shell> for String {
    fn from(shell: Shell) -> Self {
        shell.command
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(from = "ShellCommandSpec")]
pub struct ShellCommand {
    shell: Shell,
    command: String,
}

impl ShellCommand {
    pub fn exec(&self) -> Result<()> {
        self.shell.exec(&self.command)
    }

    pub fn exec_bool(&self) -> Result<bool> {
        self.shell.exec_bool(&self.command)
    }
}

impl From<String> for ShellCommand {
    fn from(command: String) -> Self {
        Self {
            shell: Shell::from_env(),
            command,
        }
    }
}

impl From<ShellCommandSpec> for ShellCommand {
    fn from(spec: ShellCommandSpec) -> Self {
        match spec {
            ShellCommandSpec::String(command) => Self::from(command),
            ShellCommandSpec::Struct { shell, command } => Self { shell, command },
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ShellCommandSpec {
    String(String),
    Struct { shell: Shell, command: String },
}

#[derive(Deserialize, Serialize, Copy, Clone, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Hook {
    Install,
    Uninstall,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct ShellHook {
    on: Vec<Hook>,
    exec: ShellCommand,
}

impl ShellHook {
    #[inline]
    pub fn applies(&self, hook: Hook) -> bool {
        self.on.contains(&hook)
    }

    #[inline]
    pub fn exec(&self) -> Result<()> {
        self.exec.exec()
    }

    #[inline]
    pub fn exec_for(&self, hook: Hook) -> Result<()> {
        self.applies(hook).then(|| self.exec()).unwrap_or(Ok(()))
    }
}

impl Resolve for ShellHook {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Hook(Self {
            exec: ShellCommand {
                command: context.parse_str(&self.exec.command)?,
                ..self.exec
            },
            ..self
        }))
    }
}

#[cfg(test)]
mod tests {
    mod command {
        #[allow(clippy::wildcard_imports)]
        use super::super::*;

        #[test]
        #[cfg(unix)]
        fn call_unchecked_success() {
            let out = command::call_unchecked(Command::new("echo").arg("hello world!")).unwrap();
            assert!(out.status.success());
            assert_eq!(String::from_utf8_lossy(&out.stdout), "hello world!\n");
        }

        #[test]
        fn call_unchecked_failure() {
            assert!(command::call_unchecked(&mut Command::new("made_up_command")).is_err());
        }

        #[test]
        #[cfg(unix)]
        fn call_bool_success() {
            assert!(command::call_bool(Command::new("echo").arg("hello world!")).unwrap());
        }

        #[test]
        fn call_bool_failure() {
            assert!(command::call_bool(&mut Command::new("made_up_command")).is_err());
        }

        #[test]
        #[cfg(unix)]
        fn pipe_unchecked_success() {
            let out = command::pipe_unchecked(
                Command::new("echo").arg("hello world!"),
                Command::new("xargs").args(["-L", "1", "echo"]),
            )
            .unwrap();
            assert!(out.status.success());
            assert_eq!(String::from_utf8_lossy(&out.stdout), "hello world!\n");
        }

        #[test]
        fn pipe_success() {
            #[cfg(windows)]
            {
                command::pipe(Command::new("cmd").arg("/c"), Command::new("cmd").arg("/c"))
                    .expect("Pipe returned error");
            }

            #[cfg(not(windows))]
            {
                command::pipe(
                    Command::new("echo").arg("hello world"),
                    Command::new("xargs").args(["-L", "1", "echo"]),
                )
                .expect("Pipe returned error");
            }
        }

        #[test]
        fn pipe_failure() {
            assert!(command::pipe(
                Command::new("fuck").arg("this"),
                Command::new("doesn't").arg("work")
            )
            .is_err());
        }
    }

    mod shell {
        #[allow(clippy::wildcard_imports)]
        use super::super::*;

        fn check_shell(input: &str, expected: ShellKind) {
            let shell = Shell::from(input);
            assert_eq!(shell.kind, expected);
            assert_eq!(shell.command.as_str(), input);
        }

        #[test]
        fn shell_kind_match() {
            check_shell("zsh", ShellKind::Zsh);
            check_shell("longer/path/bash", ShellKind::Bash);
            check_shell("some/other/shell/nonsense", ShellKind::Other);
            check_shell("", ShellKind::Empty);
        }

        #[test]
        #[cfg(windows)]
        fn shell_kind_match_windows() {
            check_shell(r#"c:\windows\path\cmd.exe"#, ShellKind::Cmd);
            check_shell(r#"c:\windows\path\pwsh"#, ShellKind::Powershell);
        }

        #[test]
        fn from_env() {
            let shell = Shell::from_env();
            match env::var("SHELL") {
                // Other shells should be added as needed
                Ok(_) => assert_ne!(shell.kind, ShellKind::Empty),
                Err(_) => assert_eq!(shell, Shell::default()),
            }
        }

        #[test]
        fn exec_success() {
            Shell::default().exec("echo 'hello world!'").unwrap();
        }

        #[test]
        fn exec_failure() {
            assert!(Shell::default()
                .exec("made_up_command with parameters")
                .is_err());
        }

        #[test]
        fn command_from_str() {
            let cmd = ShellCommand::from("echo 'hello world!'".to_string());
            assert_eq!(cmd.shell, Shell::from_env());
            assert_eq!(cmd.command, "echo 'hello world!'");
        }

        #[test]
        fn command_success() {
            ShellCommand::from("echo 'hello world!'".to_string())
                .exec()
                .unwrap();
        }

        #[test]
        fn command_failure() {
            assert!(ShellCommand::from("made_up_command -a -b".to_string())
                .exec()
                .is_err());
        }
    }
}
