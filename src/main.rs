#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::derive_partial_eq_without_eq,
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::must_use_candidate
)]
mod config;
mod context;
mod specs;

use self::{config::ResolvedConfig, specs::BuildUnit};
use anyhow::{bail, Context, Result};
use clap::{builder::PossibleValuesParser, command, Arg, ArgMatches, Command};
use std::{env, time::Instant};

#[inline]
pub fn yurt_command() -> Command {
    command!()
        .subcommand(
            Command::new("install")
                .about("Install the resolved build")
                .arg(
                    Arg::new("clean")
                        .help("Clean link target conflicts")
                        .short('c')
                        .long("clean")
                        .num_args(0),
                ),
        )
        .subcommand(Command::new("uninstall").about("Uninstall the resolved build"))
        .subcommand(Command::new("clean").about("Clean link target conflicts"))
        .subcommand(
            Command::new("show")
                .about("Show the resolved build")
                .arg(
                    Arg::new("non-trivial")
                        .help("Hide trivial build units")
                        .short('n')
                        .long("non-trivial")
                        .num_args(0),
                )
                .arg(
                    Arg::new("context")
                        .help("Print the build context")
                        .short('c')
                        .long("context")
                        .num_args(0),
                ),
        )
        .arg(
            Arg::new("yaml")
                .help("YAML build file path")
                .short('y')
                .long("yaml")
                .num_args(1),
        )
        .arg(
            Arg::new("yaml-url")
                .help("YAML build file URL")
                .long("yaml-url")
                .num_args(1)
                .conflicts_with("yaml"),
        )
        .arg(
            Arg::new("quiet")
                .help("Swallow subprocess stdout")
                .short('q')
                .long("quiet")
                .num_args(0),
        )
        .arg(
            Arg::new("log")
                .help("Logging level")
                .short('l')
                .long("log")
                .num_args(1),
        )
        .arg(
            Arg::new("exclude")
                .help("Exclude build types")
                .short('E')
                .long("exclude")
                .value_delimiter(',')
                .value_parser(PossibleValuesParser::new(BuildUnit::ALL_NAMES))
                .hide_possible_values(true),
        )
        .arg(
            Arg::new("include")
                .help("Include build types")
                .short('I')
                .long("include")
                .value_delimiter(',')
                .value_parser(PossibleValuesParser::new(BuildUnit::ALL_NAMES))
                .hide_possible_values(true)
                .conflicts_with("exclude"),
        )
        .arg(
            Arg::new("user")
                .help("Override target user name")
                .long("override-user")
                .num_args(1),
        )
        .arg(
            Arg::new("platform")
                .help("Override target platform")
                .long("override-platform")
                .num_args(1),
        )
        .arg(
            Arg::new("distro")
                .help("Override target distro")
                .long("override-distro")
                .num_args(1),
        )
}

/// Print the resolved build as YAML; optionally filter out trivial units, optionally print context
fn show(args: &ArgMatches, sub_args: &ArgMatches) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;
    let nontrivial = sub_args.get_flag("non-trivial");
    let context = sub_args.get_flag("context");

    if context {
        println!("{:#?}", config.context);
    };
    print!(
        "---\n{}",
        match nontrivial {
            true => config.nontrivial(),
            false => config,
        }
        .into_config()
        .yaml()?
    );
    Ok(())
}

/// Eliminate elements that will conflict with installation
fn clean(args: &ArgMatches, _sub_args: &ArgMatches) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;

    log::info!("Cleaning link heads...");
    for unit in config.build {
        match unit {
            BuildUnit::Link(link) => link.clean()?,
            _ => continue,
        }
    }
    Ok(())
}

fn install(args: &ArgMatches, sub_args: &ArgMatches) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;
    let clean = sub_args.get_flag("clean");

    log::info!("Installing...");
    for unit in config.build {
        match unit {
            BuildUnit::Repo(repo) => drop(repo.require()?),
            BuildUnit::Link(link) => link.link(clean)?,
            BuildUnit::Run(cmd) => cmd.exec()?,
            BuildUnit::Install(package) => package.install()?,
            BuildUnit::Require(manager) => manager.require()?,
        }
    }
    Ok(())
}

fn uninstall(args: &ArgMatches, _sub_args: &ArgMatches) -> Result<()> {
    let config = ResolvedConfig::try_from(args)?;

    log::info!("Uninstalling...");
    for unit in config.build {
        match unit {
            BuildUnit::Link(link) => link.unlink()?,
            BuildUnit::Install(package) => package.uninstall()?,
            _ => continue,
        }
    }
    Ok(())
}

fn update(_args: &ArgMatches, _sub_args: &ArgMatches) -> Result<()> {
    todo!("update is not yet supported")
}

fn main() -> Result<()> {
    if whoami::username() == "root" {
        bail!("Running as root user is not allowed. Use `sudo -u my-username` instead.");
    }

    let timer = Instant::now();
    let matches = yurt_command()
        .subcommand_required(true)
        .arg_required_else_help(true)
        .get_matches();

    if let Some(level) = matches.get_one::<&str>("log") {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();

    let result = match matches.subcommand() {
        Some(("show", s)) => show(&matches, s),
        Some(("install", s)) => install(&matches, s),
        Some(("uninstall", s)) => uninstall(&matches, s),
        Some(("clean", s)) => clean(&matches, s),
        Some(("update", s)) => update(&matches, s),
        _ => unreachable!(),
    }
    .with_context(|| format!("Subcommand failed: {}", matches.subcommand_name().unwrap()));
    log::debug!("Runtime: {:?}", timer.elapsed());
    result
}
