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
use anyhow::{Context, Result};
use clap::{command, Arg, Command};
use log::debug;
use std::{env, time::Instant};

#[inline]
pub fn yurt_command() -> Command<'static> {
    command!()
        .subcommand(
            Command::new("install")
                .about("Install the resolved build")
                .arg(
                    Arg::new("clean")
                        .help("Clean link target conflicts")
                        .short('c')
                        .long("clean")
                        .takes_value(false),
                ),
        )
        .subcommand(Command::new("uninstall").about("Uninstall the resolved build"))
        .subcommand(Command::new("clean").about("Clean link target conflicts"))
        .subcommand(
            Command::new("show").about("Show the resolved build").arg(
                Arg::new("non-trivial")
                    .help("Hide trivial build units")
                    .short('n')
                    .long("non-trivial")
                    .takes_value(false),
            ),
        )
        .arg(
            Arg::new("yaml")
                .help("YAML build file path")
                .short('y')
                .long("yaml")
                .takes_value(true),
        )
        .arg(
            Arg::new("yaml-url")
                .help("YAML build file URL")
                .long("yaml-url")
                .takes_value(true)
                .conflicts_with("yaml"),
        )
        .arg(
            Arg::new("log")
                .help("Logging level")
                .short('l')
                .long("log")
                .takes_value(true),
        )
        .arg(
            Arg::new("exclude")
                .help("Exclude build types")
                .short('E')
                .long("exclude")
                .takes_value(true)
                .use_value_delimiter(true)
                .require_value_delimiter(true)
                .multiple_values(true)
                .possible_values(BuildUnit::ALL_NAMES)
                .hide_possible_values(true),
        )
        .arg(
            Arg::new("include")
                .help("Include build types")
                .short('I')
                .long("include")
                .takes_value(true)
                .use_value_delimiter(true)
                .require_value_delimiter(true)
                .multiple_values(true)
                .possible_values(BuildUnit::ALL_NAMES)
                .hide_possible_values(true)
                .conflicts_with("exclude"),
        )
        .arg(
            Arg::new("user")
                .help("Override target user name")
                .long("override-user")
                .takes_value(true),
        )
        .arg(
            Arg::new("platform")
                .help("Override target platform")
                .long("override-platform")
                .takes_value(true),
        )
        .arg(
            Arg::new("distro")
                .help("Override target distro")
                .long("override-distro")
                .takes_value(true),
        )
}

fn main() -> Result<()> {
    let matches = yurt_command()
        .subcommand_required(true)
        .arg_required_else_help(true)
        .get_matches();

    if let Some(level) = matches.value_of("log") {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();

    let timer = Instant::now();
    let result = ResolvedConfig::try_from(&matches)
        .context("Failed to resolve build")
        .and_then(|r| match matches.subcommand() {
            Some(("show", s)) => r.show(s.is_present("non-trivial")),
            Some(("install", s)) => r.install(s.is_present("clean")),
            Some(("uninstall", _)) => r.uninstall(),
            Some(("clean", _)) => r.clean(),
            Some(("update", _)) => r.update(),
            _ => unreachable!(),
        })
        .with_context(|| format!("Subcommand failed: {:?}", matches.subcommand_name()));
    debug!("Runtime: {:?}", timer.elapsed());
    result
}
