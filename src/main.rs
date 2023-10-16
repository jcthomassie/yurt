#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::single_match_else
)]
mod config;
mod context;
mod specs;

use std::{
    io::{self, Write},
    iter::Map,
    path::PathBuf,
};

use anyhow::{bail, Context as _, Result};
use clap::{command, ArgGroup, Parser, Subcommand};
use console::style;
use indicatif::{ProgressBarIter, ProgressFinish, ProgressIterator};
use specs::BuildUnitInterface;

use self::{
    config::{Config, ResolvedConfig},
    context::Context,
    specs::{BuildUnit, BuildUnitKind, Hook},
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(arg_required_else_help(true))]
#[command(group(
    ArgGroup::new("build_source")
        .args(["file", "file_url"])
))]
#[command(group(
    ArgGroup::new("filter")
        .args(["include", "exclude"])
))]
pub struct YurtArgs {
    /// YAML build file path
    #[arg(long, short = 'f', value_name = "FILE")]
    file: Option<PathBuf>,

    /// YAML build file URL
    #[arg(long, short = 'u', value_name = "URL")]
    file_url: Option<String>,

    /// Allow yurt to run as root user
    #[arg(long)]
    root: bool,

    /// Reduce output verbosity
    #[arg(long, short = 'q')]
    quiet: bool,

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
        short = 'I',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    include: Option<Vec<BuildUnitKind>>,

    /// Exclude the specified build unit types
    #[arg(
        value_enum,
        long,
        short = 'E',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    exclude: Option<Vec<BuildUnitKind>>,

    #[command(subcommand)]
    action: YurtAction,
}

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

fn iter_units(
    msg: &'static str,
    msg_finish: &'static str,
    resolved: ResolvedConfig,
) -> impl Iterator<Item = (BuildUnitKind, Box<dyn BuildUnitInterface>)> {
    let progress = resolved
        .context
        .progress_bar(resolved.build.len())
        .with_message(msg)
        .with_finish(ProgressFinish::WithMessage(
            format!("{}", style(msg_finish).green().bold()).into(),
        ));
    resolved
        .build
        .into_iter()
        .map(|unit| (unit.kind(), unit.unwrap()))
        .progress_with(progress)
}

fn main() -> Result<()> {
    let args = YurtArgs::parse();
    if !&args.root && whoami::username() == "root" {
        bail!(
            "Running as root user requires the `--root` argument. \
            Use `sudo -u my-username` to run as an elevated non-root user."
        );
    }

    let mut context = Context::from(&args);
    if !args.quiet {
        writeln!(io::stderr(), "{:#?}", style(&args).dim())?;
    }

    match args.action {
        YurtAction::Show {
            context: true, raw, ..
        } => {
            if !raw {
                ResolvedConfig::resolve_from(&args, &mut context)?;
            }
            writeln!(io::stdout(), "{context:#?}").context("Failed to write context to console")
        }
        YurtAction::Show { raw: true, .. } => {
            writeln!(io::stdout(), "{}", Config::try_from(&args)?.yaml()?)
                .context("Failed to write config to console")
        }
        YurtAction::Show {
            hook: ref hook_arg, ..
        } => {
            let mut resolved = ResolvedConfig::resolve_from(&args, &mut context)?;
            if let Some(hook_arg) = hook_arg {
                let context = resolved.context;
                resolved = resolved.filter(|unit| {
                    let expect = matches!(hook_arg, Hook::Install);
                    match hook_arg {
                        Hook::Install | Hook::Uninstall => match unit {
                            BuildUnit::Repo(repo) => repo.is_available() != expect,
                            BuildUnit::Link(link) => link.is_valid() != expect,
                            BuildUnit::Package(package) => package.is_installed(context) != expect,
                            BuildUnit::PackageManager(manager) => manager.is_available() != expect,
                            BuildUnit::Hook(hook) => hook.applies(hook_arg),
                        },
                        Hook::Custom(_) => match unit {
                            BuildUnit::Hook(hook) => hook.applies(hook_arg),
                            _ => false,
                        },
                    }
                });
            };
            writeln!(io::stdout(), "{}", resolved.into_config().yaml()?)
                .context("Failed to write config to console")
        }
        // TODO use clean
        YurtAction::Install { clean: _ } => iter_units(
            "Installing",
            "Install finished",
            ResolvedConfig::resolve_from(&args, &mut context)?,
        )
        .try_for_each(|(kind, interface)| {
            let result = interface.unit_install(&context);
            context.write_result(kind, interface, "installed", result)
        }),
        YurtAction::Uninstall => iter_units(
            "Uninstalling",
            "Uninstall finished",
            ResolvedConfig::resolve_from(&args, &mut context)?,
        )
        .try_for_each(|(kind, interface)| {
            let result = interface.unit_uninstall(&context);
            context.write_result(kind, interface, "uninstalled", result)
        }),
        YurtAction::Hook { ref hook } => iter_units(
            "Hook",
            "Hook finished",
            ResolvedConfig::resolve_from(&args, &mut context)?,
        )
        .try_for_each(|(kind, interface)| {
            let result = interface.unit_hook(&context, hook);
            context.write_result(kind, interface, format!("{hook:?}"), result)
        }),
    }
    .with_context(|| format!("Action failed: {:?}", args.action))
}
