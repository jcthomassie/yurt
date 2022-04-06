mod build;
mod files;
mod package;
mod repo;
mod shell;

use anyhow::{Context as AnyContext, Result};
use build::{yaml::Config, Context, ResolvedConfig};
use clap::{command, Arg, ArgMatches, Command};
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

#[inline]
fn parse_resolved(matches: &ArgMatches) -> Result<ResolvedConfig> {
    Config::from_args(matches)?
        .resolve(Context::from(matches))
        .context("Failed to resolve build")
}

fn show(matches: &ArgMatches) -> Result<()> {
    let sub = matches.subcommand_matches("show").unwrap();
    parse_resolved(matches)?
        .show(sub.is_present("non-trivial"))
        .context("Failed to show resolved build")
}

fn install(matches: &ArgMatches) -> Result<()> {
    let sub = matches.subcommand_matches("install").unwrap();
    parse_resolved(matches)?
        .install(sub.is_present("clean"))
        .context("Failed to complete install steps")
}

fn uninstall(matches: &ArgMatches) -> Result<()> {
    let sub = matches.subcommand_matches("uninstall").unwrap();
    parse_resolved(matches)?
        .uninstall(sub.is_present("packages"))
        .context("Failed to complete uninstall steps")
}

fn clean(matches: &ArgMatches) -> Result<()> {
    parse_resolved(matches)?
        .clean()
        .context("Failed to clean link heads")
}

fn update(matches: &ArgMatches) -> Result<()> {
    parse_resolved(matches)?
        .update()
        .context("Failed to complete update")
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
    let result = match matches.subcommand_name() {
        Some("show") => show(&matches),
        Some("install") => install(&matches),
        Some("uninstall") => uninstall(&matches),
        Some("clean") => clean(&matches),
        Some("update") => update(&matches),
        _ => unreachable!(),
    };
    debug!("Runtime: {:?}", timer.elapsed());
    result
}
