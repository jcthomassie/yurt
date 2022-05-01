use anyhow::{anyhow, Context, Result};
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
    fn call(&self, args: &[&str]) -> Result<()> {
        match self.call_unchecked(args)?.status.success() {
            true => Ok(()),
            false => Err(anyhow!(
                "Command exited with error: `{}`",
                self.format(args)
            )),
        }
    }

    #[inline]
    fn call_bool(&self, args: &[&str]) -> Result<bool> {
        Ok(self.call_unchecked(args)?.status.success())
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

#[derive(Clone, Debug, PartialEq)]
pub struct Shell {
    kind: ShellKind,
    command: String,
}

impl<T> From<T> for Shell
where
    T: Into<String>,
{
    fn from(command: T) -> Self {
        let command = command.into();
        Self {
            kind: ShellKind::from(Path::new(&command)),
            command,
        }
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

impl Shell {
    pub fn from_env() -> Self {
        match env::var("SHELL") {
            Ok(s) => Self::from(s),
            Err(_) => Self::default(),
        }
    }

    pub fn run(&self, command: &str) -> Result<Output> {
        match self.kind {
            ShellKind::Cmd => self.call_unchecked(&["/C", command]),
            _ => self.call_unchecked(&["-c", command]),
        }
    }

    /// Use curl to fetch remote script and pipe into shell
    #[inline]
    pub fn remote_script(&self, curl_args: &[&str]) -> Result<()> {
        pipe("curl", curl_args, self, &[]).map(drop)
    }
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename = "shell")]
pub struct ShellSpec(String);

impl From<ShellSpec> for Shell {
    fn from(spec: ShellSpec) -> Self {
        Self::from(spec.0)
    }
}

impl From<Shell> for ShellSpec {
    fn from(shell: Shell) -> Self {
        Self(shell.command)
    }
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
    fn shell_command_success() {
        let out = Shell::default().run("echo 'hello world!'").unwrap();
        assert!(out.status.success());
        #[cfg(unix)]
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello world!\n");
        #[cfg(windows)]
        assert_eq!(String::from_utf8_lossy(&out.stdout), "'hello world!'\r\n");
        // Windows escapes shell commands over-eagerly
    }

    #[test]
    fn shell_command_failure() {
        let out = Shell::default()
            .run("made_up_command with parameters")
            .unwrap();
        assert!(!out.status.success());
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
