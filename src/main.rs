#![doc = include_str!("../README.md")]
#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::single_match_else
)]
mod config;
mod context;
mod docs;
mod specs;

use self::{
    config::{Config, ResolvedConfig},
    context::{Context, Locale},
    specs::{BuildUnit, BuildUnitKind, Hook},
};
use anyhow::{bail, Context as _, Result};
use clap::{command, ArgGroup, Parser, Subcommand};
use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

#[derive(Subcommand, Debug)]
enum YurtAction {
    /// Show the resolved build
    #[command(group(
        ArgGroup::new("modifier")
            .args(["raw", "hook"])
    ))]
    Show {
        /// Print unresolved config/context
        #[arg(long, short)]
        raw: bool,

        /// Print the build context
        #[arg(long, short)]
        context: bool,

        /// Show non-trivial units for the specified hook
        #[arg(long)]
        hook: Option<Hook>,
    },

    /// Install the resolved build
    Install {
        /// Clean link target conflicts
        #[arg(long, short)]
        clean: bool,
    },

    /// Uninstall the resolved build
    Uninstall,

    /// Run resolved build hooks
    Hook {
        /// Type of hook to run
        hook: Hook,
    },
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(arg_required_else_help(true))]
#[command(group(
    ArgGroup::new("build_source")
        .args(["file", "file_url"])
))]
pub struct YurtArgs {
    /// YAML build file path
    #[arg(long, short = 'f', value_name = "FILE")]
    file: Option<PathBuf>,

    /// YAML build file URL
    #[arg(long, short = 'u', value_name = "URL")]
    file_url: Option<String>,

    /// Logging level
    #[arg(long)]
    log: Option<String>,

    /// Allow yurt to run as root user
    #[arg(long)]
    root: bool,

    /// Override target username
    #[arg(long, value_name = "USER")]
    override_user: Option<String>,

    /// Override target platform
    #[arg(long, value_name = "PLATFORM")]
    override_platform: Option<String>,

    /// Override target distro
    #[arg(long, value_name = "DISTRO")]
    override_distro: Option<String>,

    /// Include only the specified build unit types
    #[arg(
        value_enum,
        long,
        short = 'i',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    include: Option<Vec<BuildUnitKind>>,

    /// Exclude the specified build unit types
    #[arg(
        value_enum,
        long,
        short = 'e',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    exclude: Option<Vec<BuildUnitKind>>,

    #[command(subcommand)]
    action: YurtAction,
}

impl YurtArgs {
    fn get_locale(&self) -> Locale {
        Locale::with_overrides(
            self.override_user.clone(),
            self.override_platform.clone(),
            self.override_distro.clone(),
        )
    }

    fn get_context(&self) -> Context {
        Context::new(self.get_locale())
    }

    fn get_config(&self) -> Result<Config> {
        if let Some(ref url) = self.file_url {
            Config::from_url(url)
        } else if let Some(ref file) = self.file {
            Config::from_path(file)
        } else {
            Config::from_env()
        }
    }

    fn get_resolved_config(&self) -> Result<ResolvedConfig> {
        self.get_config()
            .and_then(|config| config.resolve(self.get_context()))
            .map(|resolved| {
                resolved
                    .filter(|unit, _| {
                        self.include
                            .as_ref()
                            .map_or(true, |kinds| unit.included_in(kinds))
                    })
                    .filter(|unit, _| {
                        self.exclude
                            .as_ref()
                            .map_or(true, |kinds| !unit.included_in(kinds))
                    })
            })
    }

    fn execute(&self) -> Result<()> {
        match self.action {
            // $ yurt show --context
            YurtAction::Show {
                raw, context: true, ..
            } => {
                let context = if raw {
                    self.get_context()
                } else {
                    self.get_resolved_config()?.context
                };
                writeln!(io::stdout(), "{context:#?}").context("Failed to write context to stdout")
            }
            // $ yurt show
            YurtAction::Show {
                raw,
                hook: ref hook_arg,
                ..
            } => {
                let config = if raw {
                    self.get_config()?
                } else {
                    let resolved = self.get_resolved_config()?;
                    if let Some(hook_arg) = hook_arg {
                        resolved
                            .filter(|unit, context| {
                                let expect = matches!(hook_arg, Hook::Install);
                                match hook_arg {
                                    Hook::Install | Hook::Uninstall => match unit {
                                        BuildUnit::Repo(repo) => repo.is_available() != expect,
                                        BuildUnit::Link(link) => link.is_valid() != expect,
                                        BuildUnit::Package(package) => {
                                            package.is_installed(context) != expect
                                        }
                                        BuildUnit::PackageManager(manager) => {
                                            manager.is_available() != expect
                                        }
                                        BuildUnit::Hook(hook) => hook.applies(hook_arg),
                                    },
                                    Hook::Custom(_) => match unit {
                                        BuildUnit::Hook(hook) => hook.applies(hook_arg),
                                        _ => false,
                                    },
                                }
                            })
                            .into_config()
                    } else {
                        resolved.into_config()
                    }
                };
                writeln!(io::stdout(), "{}", config.yaml()?)
                    .context("Failed to write yaml to stdout")
            }
            // $ yurt install
            YurtAction::Install { clean } => self.get_resolved_config().and_then(|build| {
                build.for_each_unit(|unit, context| match unit {
                    BuildUnit::Repo(repo) => repo.require().map(drop),
                    BuildUnit::Link(link) => link.link(clean),
                    BuildUnit::Hook(hook) => hook.exec_for(&Hook::Install),
                    BuildUnit::Package(package) => package.install(context),
                    BuildUnit::PackageManager(manager) => manager.require(),
                })
            }),
            // $ yurt uninstall
            YurtAction::Uninstall => self.get_resolved_config().and_then(|build| {
                build.for_each_unit(|unit, context| match unit {
                    BuildUnit::Link(link) => link.unlink(),
                    BuildUnit::Hook(hook) => hook.exec_for(&Hook::Uninstall),
                    BuildUnit::Package(package) => package.uninstall(context),
                    _ => Ok(()),
                })
            }),
            // $ yurt hook
            YurtAction::Hook { hook: ref arg } => self.get_resolved_config().and_then(|build| {
                build.for_each_unit(|unit, _| match unit {
                    BuildUnit::Hook(hook) => hook.exec_for(arg),
                    _ => Ok(()),
                })
            }),
        }
    }
}

#[doc(hidden)]
fn main() -> Result<()> {
    let timer = Instant::now();
    let args = YurtArgs::parse();

    if let Some(level) = &args.log {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();
    log::debug!("{:#?}", &args);

    if !&args.root && whoami::username() == "root" {
        bail!(
            "Running as root user requires the `--root` argument. \
            Use `sudo -u my-username` to run as an elevated non-root user."
        );
    }

    log::info!("{:?}", &args.action);
    let result = args
        .execute()
        .with_context(|| format!("Action failed: {:?}", args.action));
    log::debug!("Runtime: {:?}", timer.elapsed());
    result
}
