mod error;
mod link;
mod pack;
mod repo;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use error::YurtResult;
use log::info;
use std::env;
use std::process::Command;
use yaml::{Build, BuildUnit};

#[inline]
fn parse_build(matches: &ArgMatches) -> YurtResult<Build> {
    Build::from_path(link::expand_path(matches.value_of("yaml").unwrap())?)
}

#[inline]
fn parse_resolve_build(matches: &ArgMatches) -> YurtResult<(repo::Repo, Vec<BuildUnit>)> {
    parse_build(matches)?.resolve()
}

macro_rules! skip {
    () => {
        |_| Ok(())
    };
}

fn show(matches: &ArgMatches) -> YurtResult<()> {
    let build = parse_build(matches)?;
    let (repo, units) = build.resolve()?;
    println!("Locale:\n{:#?}", *yaml::LOCALE);
    println!("_______________________________________");
    println!("Repo:\n{:#?}", repo);
    println!("_______________________________________");
    println!("Steps:\n{:#?}", units);
    Ok(())
}

fn install(matches: &ArgMatches) -> YurtResult<()> {
    let (repo, units) = parse_resolve_build(matches)?;
    // Optionally clean before install
    let sub = matches.subcommand_matches("install").unwrap();
    if sub.is_present("clean") {
        clean(matches)?;
    }
    repo.require()?;
    info!("Starting build steps...");
    yaml::apply(
        units,
        |ln| ln.link(),
        |pkg| pkg.install(),
        |pm| pm.require(),
    )?;
    pack::Shell::Zsh.chsh()?;
    Ok(())
}

fn uninstall(matches: &ArgMatches) -> YurtResult<()> {
    let (_, units) = parse_resolve_build(matches)?;
    info!("Uninstalling dotfiles...");
    yaml::apply(units, |ln| ln.unlink(), skip!(), skip!())?;
    Ok(())
}

fn clean(matches: &ArgMatches) -> YurtResult<()> {
    let (_, units) = parse_resolve_build(matches)?;
    info!("Cleaning link heads...");
    yaml::apply(units, |ln| ln.clean(), skip!(), skip!())?;
    Ok(())
}

fn edit() -> YurtResult<()> {
    Command::new(env::var("EDITOR").expect("system editor is unset"))
        .arg(env::var("YURT_REPO_ROOT").expect("dotfile repo root is unset"))
        .output()?;
    Ok(())
}

fn update() -> YurtResult<()> {
    info!("Updating dotfiles...");
    Ok(())
}

fn main() -> YurtResult<()> {
    let matches = App::new("dots")
        .author(crate_authors!())
        .version(crate_version!())
        .about("Simple CLI tool for dotfile management.")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::ColoredHelp)
        .subcommand(
            App::new("install").about("Installs dotfiles").arg(
                Arg::new("clean")
                    .about("Run `dots clean` before install")
                    .short('c')
                    .long("clean")
                    .takes_value(false),
            ),
        )
        .subcommand(App::new("uninstall").about("Uninstalls dotfiles"))
        .subcommand(App::new("update").about("Updates dotfiles and/or system"))
        .subcommand(App::new("clean").about("Cleans output destinations"))
        .subcommand(App::new("edit").about("Opens dotfile repo in system editor"))
        .subcommand(App::new("show").about("Shows the build config"))
        .arg(
            Arg::new("yaml")
                .about("YAML build file path")
                .short('y')
                .long("yaml")
                .default_value("$HOME/dotfiles/install.yaml"),
        )
        .arg(
            Arg::new("log")
                .about("Logging level")
                .short('l')
                .long("log")
                .takes_value(true),
        )
        .get_matches();

    if let Some(level) = matches.value_of("log") {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();

    match matches.subcommand_name() {
        Some("show") => show(&matches),
        Some("install") => install(&matches),
        Some("uninstall") => uninstall(&matches),
        Some("clean") => clean(&matches),
        Some("update") => update(),
        Some("edit") => edit(),
        _ => unreachable!(),
    }
}
