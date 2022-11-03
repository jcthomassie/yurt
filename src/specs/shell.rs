use crate::specs::{BuildUnit, Context, Resolve};

use anyhow::{anyhow, Context as _, Result};
use log::debug;
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsStr,
    path::Path,
    process::{Command, Output, Stdio},
};

pub trait Cmd {
    fn name(&self) -> &str;

    #[inline]
    fn format(&self, args: &[&str]) -> String {
        format!("{} {}", self.name(), args.join(" "))
    }

    #[inline]
    fn command(&self) -> Command {
        Command::new(self.name())
    }

    #[inline]
    fn call_unchecked(&self, args: &[&str]) -> Result<Output> {
        debug!("Calling command: `{}`", self.format(args));
        self.command()
            .args(args)
            .output()
            .with_context(|| format!("Failed to run command: `{}`", self.format(args)))
    }

    #[inline]
    fn call_bool(&self, args: &[&str]) -> Result<bool> {
        self.call_unchecked(args).map(|out| out.status.success())
    }

    #[inline]
    fn call(&self, args: &[&str]) -> Result<()> {
        self.call_bool(args).and_then(|success| {
            success
                .then_some(())
                .with_context(|| anyhow!("Command exited with error: `{}`", self.format(args)))
        })
    }
}

impl Cmd for str {
    fn name(&self) -> &str {
        self
    }
}

fn pipe<T, U>(cmd_a: &T, args_a: &[&str], cmd_b: &U, args_b: &[&str]) -> Result<Output>
where
    T: Cmd + ?Sized,
    U: Cmd + ?Sized,
{
    debug!(
        "Calling command: `{} | {}`",
        cmd_a.format(args_a),
        cmd_b.format(args_b)
    );
    let mut proc_a = cmd_a
        .command()
        .args(args_a)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn primary pipe command")?;
    let pipe = proc_a.stdout.take().context("Failed to create pipe")?;
    let proc_b = cmd_b
        .command() //
        .args(args_b)
        .stdin(pipe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn secondary pipe command")?;
    match proc_b.wait_with_output() {
        Ok(out) if out.status.success() => Ok(out),
        Ok(out) => Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) => Err(anyhow!(e)),
    }
    .with_context(|| {
        format!(
            "Failed command: `{} | {}`",
            cmd_a.format(args_a),
            cmd_b.format(args_b)
        )
    })
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

    pub fn run(&self, command: &str) -> Result<()> {
        match self.kind {
            ShellKind::Cmd => self.call(&["/C", command]),
            _ => self.call(&["-c", command]),
        }
    }

    pub fn run_bool(&self, command: &str) -> Result<bool> {
        match self.kind {
            ShellKind::Cmd => self.call_bool(&["/C", command]),
            _ => self.call_bool(&["-c", command]),
        }
    }

    /// Use curl to fetch remote script and pipe into shell
    #[inline]
    pub fn remote_script(&self, curl_args: &[&str]) -> Result<()> {
        pipe("curl", curl_args, self, &[]).map(drop)
    }
}

impl Cmd for Shell {
    fn name(&self) -> &str {
        self.command.as_str()
    }
}

impl Default for Shell {
    #[cfg(target_os = "windows")]
    fn default() -> Self {
        Shell::from("cmd")
    }

    #[cfg(target_os = "macos")]
    fn default() -> Self {
        Shell::from("zsh")
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    fn default() -> Self {
        Shell::from("sh")
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
    pub fn run(&self) -> Result<()> {
        self.shell.run(&self.command)
    }

    pub fn run_bool(&self) -> Result<bool> {
        self.shell.run_bool(&self.command)
    }
}

impl Resolve for ShellCommand {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Run(Self {
            command: context.variables.parse_str(&self.command)?,
            ..self
        }))
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn shell_from_env() {
        let shell = Shell::from_env();
        match env::var("SHELL") {
            // Other shells should be added as needed
            Ok(_) => assert_ne!(shell.kind, ShellKind::Empty),
            Err(_) => assert_eq!(shell, Shell::default()),
        }
    }

    #[test]
    fn shell_run_success() {
        Shell::default().run("echo 'hello world!'").unwrap();
    }

    #[test]
    fn shell_run_failure() {
        assert!(Shell::default()
            .run("made_up_command with parameters")
            .is_err());
    }

    #[test]
    fn shell_command_from_str() {
        let cmd = ShellCommand::from("echo 'hello world!'".to_string());
        assert_eq!(cmd.shell, Shell::from_env());
        assert_eq!(cmd.command, "echo 'hello world!'");
    }

    #[test]
    fn shell_command_success() {
        ShellCommand::from("echo 'hello world!'".to_string())
            .run()
            .unwrap();
    }

    #[test]
    fn shell_command_failure() {
        assert!(ShellCommand::from("made_up_command -a -b".to_string())
            .run()
            .is_err());
    }

    #[test]
    fn pipe_success() {
        assert!(if cfg!(windows) {
            pipe("cmd", &["/c"], "cmd", &["/c"])
        } else {
            pipe("echo", &["hello"], "echo", &["world"])
        }
        .expect("Pipe returned error")
        .status
        .success());
    }

    #[test]
    fn pipe_failure() {
        assert!(pipe("fuck", &["this"], "doesn't", &["work"]).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn str_command_success() {
        let out = "echo".call_unchecked(&["hello world!"]).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello world!\n");
    }

    #[test]
    fn str_command_failure() {
        assert!("made_up_command".call_unchecked(&[]).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn str_command_bool_success() {
        assert!("echo".call_bool(&["hello world!"]).unwrap());
    }

    #[test]
    fn str_command_bool_failure() {
        assert!("made_up_command".call_bool(&[]).is_err());
    }
}
