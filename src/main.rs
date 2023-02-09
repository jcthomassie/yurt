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
use clap::{builder::PossibleValuesParser, command, Arg, Command};
use log::debug;
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

fn main() -> Result<()> {
    if whoami::username() == "root" {
        bail!("Running as root user is not allowed. Use `sudo -u some-other-user` instead.");
    }

    let matches = yurt_command()
        .subcommand_required(true)
        .arg_required_else_help(true)
        .get_matches();

    if let Some(level) = matches.get_one::<&str>("log") {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();

    let timer = Instant::now();
    let result = ResolvedConfig::try_from(&matches)
        .context("Failed to resolve build")
        .and_then(|r| match matches.subcommand() {
            Some(("show", s)) => r.show(s.get_flag("non-trivial"), s.get_flag("context")),
            Some(("install", s)) => r.install(s.get_flag("clean")),
            Some(("uninstall", _)) => r.uninstall(),
            Some(("clean", _)) => r.clean(),
            Some(("update", _)) => r.update(),
            _ => unreachable!(),
        })
        .with_context(|| format!("Subcommand failed: {}", matches.subcommand_name().unwrap()));
    debug!("Runtime: {:?}", timer.elapsed());
    result
}
