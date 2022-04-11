#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::wildcard_imports
)]
mod build;
mod condition;
mod files;
mod package;
mod repo;
mod shell;

use anyhow::{Context as AnyContext, Result};
use build::{Config, Context};
use clap::{command, Arg, Command};
use log::debug;
use std::{env, time::Instant};

#[inline]
pub fn yurt_command() -> Command<'static> {
    command!()
        .subcommand(
            Command::new("install").about("Installs dotfiles").arg(
                Arg::new("clean")
                    .help("Run `dots clean` before install")
                    .short('c')
                    .long("clean")
                    .takes_value(false),
            ),
        )
        .subcommand(
            Command::new("uninstall").about("Uninstalls dotfiles").arg(
                Arg::new("packages")
                    .help("Uninstall packages too")
                    .short('p')
                    .long("packages")
                    .takes_value(false),
            ),
        )
        .subcommand(Command::new("update").about("Updates dotfiles and/or system"))
        .subcommand(Command::new("clean").about("Cleans output destinations"))
        .subcommand(
            Command::new("show").about("Shows the build config").arg(
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
    let result = Config::from_args(&matches)?
        .resolve(Context::from(&matches))
        .context("Failed to resolve build")
        .and_then(|r| match matches.subcommand() {
            Some(("show", s)) => r.show(s.is_present("non-trivial")),
            Some(("install", s)) => r.install(s.is_present("clean")),
            Some(("uninstall", s)) => r.uninstall(s.is_present("packages")),
            Some(("clean", _)) => r.clean(),
            Some(("update", _)) => r.update(),
            _ => unreachable!(),
        })
        .with_context(|| format!("Subcommand failed: {:?}", matches.subcommand_name()));
    debug!("Runtime: {:?}", timer.elapsed());
    result
}
