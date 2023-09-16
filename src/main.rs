#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::single_match_else
)]
mod config;
mod context;
mod specs;

use self::{
    config::{Config, ResolvedConfig},
    context::Context,
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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(arg_required_else_help(true))]
#[command(group(
    ArgGroup::new("yaml_file")
        .args(["yaml", "yaml_url"])
))]
#[command(group(
    ArgGroup::new("filter")
        .args(["include", "exclude"])
))]
pub struct YurtArgs {
    /// YAML build file path
    #[arg(long, short, value_name = "FILE")]
    yaml: Option<PathBuf>,

    /// YAML build file URL
    #[arg(long, value_name = "URL")]
    yaml_url: Option<String>,

    /// Logging level
    #[arg(long, short)]
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
            .args(["raw", "nontrivial"])
    ))]
    Show {
        /// Print unresolved config/context
        #[arg(long, short)]
        raw: bool,

        /// Hide trivial build units
        #[arg(long, short)]
        nontrivial: bool,

        /// Print the build context
        #[arg(long, short)]
        context: bool,
    },

    /// Install the resolved build
    Install {
        /// Clean link target conflicts
        #[arg(long, short)]
        clean: bool,
    },

    /// Uninstall the resolved build
    Uninstall,
}

fn show_context(args: &YurtArgs, raw: bool) -> Result<()> {
    let context = if raw {
        Context::from(args)
    } else {
        ResolvedConfig::try_from(args)?.context
    };
    writeln!(io::stdout(), "{context:#?}").context("Failed to write context to stdout")
}

fn show_build(args: &YurtArgs, raw: bool, nontrivial: bool) -> Result<()> {
    let config = if raw {
        Config::try_from(args)?
    } else {
        let mut res = ResolvedConfig::try_from(args)?;
        // TODO: get rid of this clone
        let context = res.context.clone();
        if nontrivial {
            res = res.filter(|unit| match unit {
                BuildUnit::Repo(repo) => !repo.is_available(),
                BuildUnit::Link(link) => !link.is_valid(),
                BuildUnit::Install(package) => !package.is_installed(&context),
                BuildUnit::Require(manager) => !manager.is_available(),
                BuildUnit::Hook(hook) => hook.applies(Hook::Install),
            });
        }
        res.into_config()
    };
    writeln!(io::stdout(), "{}", config.yaml()?).context("Failed to write yaml to stdout")
}

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
    let result = match args.action {
        YurtAction::Show {
            raw, context: true, ..
        } => show_context(&args, raw),
        YurtAction::Show {
            raw, nontrivial, ..
        } => show_build(&args, raw, nontrivial),
        YurtAction::Install { clean } => ResolvedConfig::try_from(&args).and_then(|build| {
            build.for_each_unit(|unit| match unit {
                BuildUnit::Repo(repo) => repo.require().map(drop),
                BuildUnit::Link(link) => link.link(clean),
                BuildUnit::Hook(hook) => hook.exec_for(Hook::Install),
                BuildUnit::Install(package) => package.install(&build.context),
                BuildUnit::Require(manager) => manager.require(),
            })
        }),
        YurtAction::Uninstall => ResolvedConfig::try_from(&args).and_then(|build| {
            build.for_each_unit(|unit| match unit {
                BuildUnit::Link(link) => link.unlink(),
                BuildUnit::Hook(hook) => hook.exec_for(Hook::Uninstall),
                BuildUnit::Install(package) => package.uninstall(&build.context),
                _ => Ok(()),
            })
        }),
    }
    .with_context(|| format!("Action failed: {:?}", args.action));
    log::debug!("Runtime: {:?}", timer.elapsed());
    result
}
