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

    /// Diff resolved build against another resolved build
    Diff { base: PathBuf },
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
        } else if let Some(ref path) = self.file {
            Config::from_path(path)
        } else {
            Config::from_env()
        }
    }

    fn resolve(&self, config: Config) -> Result<ResolvedConfig> {
        Ok(config
            .resolve(self.get_context())
            .context("Failed to resolve config")?
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
            .filter(|unit, context| {
                match &self.action {
                    YurtAction::Show { hook, .. } => hook.as_ref(),
                    YurtAction::Hook { ref hook } => Some(hook),
                    YurtAction::Install { .. } => Some(&Hook::Install),
                    YurtAction::Uninstall { .. } => Some(&Hook::Uninstall),
                    YurtAction::Diff { .. } => None,
                }
                .map_or(true, |hook| unit.should_apply(context, hook))
            }))
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
                    self.get_config()
                        .and_then(|config| self.resolve(config))?
                        .context
                };
                writeln!(io::stdout(), "{context:#?}").context("Failed to write context to stdout")
            }
            // $ yurt show
            YurtAction::Show { raw, .. } => {
                let config = if raw {
                    self.get_config()?
                } else {
                    self.resolve(self.get_config()?)?.into_config()
                };
                writeln!(io::stdout(), "{}", config.yaml()?)
                    .context("Failed to write yaml to stdout")
            }
            // $ yurt install
            YurtAction::Install { clean } => self
                .resolve(self.get_config()?)?
                .for_each_unit(|unit, context| unit.install(context, clean)),
            // $ yurt uninstall
            YurtAction::Uninstall => self
                .resolve(self.get_config()?)?
                .for_each_unit(BuildUnit::uninstall),
            // $ yurt hook
            YurtAction::Hook { ref hook } => self
                .resolve(self.get_config()?)?
                .for_each_unit(|unit, _| unit.hook(hook)),
            // $ yurt diff
            YurtAction::Diff { ref base } => self //
                .resolve(self.get_config()?)?
                .for_each_unit_diff(&self.resolve(Config::from_git_path(base)?)?, |unit, _| {
                    writeln!(io::stdout(), "{unit:#?}").context("Failed to write diff to stdout")
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
