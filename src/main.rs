#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::derive_partial_eq_without_eq,
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::too_many_lines
)]
mod config;
mod context;
mod specs;

use self::{
    config::{Config, ResolvedConfig},
    context::Context,
    specs::{BuildUnit, BuildUnitKind},
};
use anyhow::{bail, Context as _, Result};
use clap::{command, ArgGroup, Parser, Subcommand};
use std::{env, path::PathBuf, time::Instant};

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

    /// Swallow subprocess stdout
    #[arg(long, short)]
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
    Show {
        /// Print unresolved config
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

    /// Clean link target conflicts
    Clean,
}

fn show(args: &YurtArgs, raw: bool, nontrivial: bool, context: bool) -> Result<()> {
    let config = if raw {
        if context {
            println!("{:#?}\n---", Context::from(args));
        };
        Config::try_from(args)?
    } else {
        let mut res = ResolvedConfig::try_from(args)?;
        if nontrivial {
            res = res.nontrivial();
        }
        if context {
            println!("{:#?}\n---", res.context);
        };
        res.into_config()
    };
    print!("{}", config.yaml()?);
    Ok(())
}

fn clean(args: &YurtArgs) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;

    log::info!("Cleaning link heads...");
    config.for_each_unit(|unit| match unit {
        BuildUnit::Link(link) => link.clean(),
        _ => Ok(()),
    })
}

fn install(args: &YurtArgs, clean: bool) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;

    log::info!("Installing...");
    config.for_each_unit(|unit| match unit {
        BuildUnit::Repo(repo) => repo.require().map(drop),
        BuildUnit::Link(link) => link.link(clean),
        BuildUnit::Run(cmd) => cmd.exec(),
        BuildUnit::Install(package) => package.install(),
        BuildUnit::Require(manager) => manager.require(),
    })
}

fn uninstall(args: &YurtArgs) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;

    log::info!("Uninstalling...");
    config.for_each_unit(|unit| match unit {
        BuildUnit::Link(link) => link.unlink(),
        BuildUnit::Install(package) => package.uninstall(),
        _ => Ok(()),
    })
}

fn main() -> Result<()> {
    if whoami::username() == "root" {
        bail!("Running as root user is not allowed. Use `sudo -u my-username` instead.");
    }

    let timer = Instant::now();
    let args = YurtArgs::parse();

    if let Some(level) = &args.log {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();

    let result = match args.action {
        YurtAction::Show {
            raw,
            nontrivial,
            context,
        } => show(&args, raw, nontrivial, context),
        YurtAction::Clean => clean(&args),
        YurtAction::Install { clean } => install(&args, clean),
        YurtAction::Uninstall => uninstall(&args),
    }
    .with_context(|| format!("Action failed: {:?}", args.action));
    log::debug!("Runtime: {:?}", timer.elapsed());
    result
}
