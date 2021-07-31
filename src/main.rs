mod error;
mod link;
mod pack;
mod repo;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use error::DotsResult;
use std::env;
use std::process::Command;

#[inline(always)]
fn parse_build(matches: &ArgMatches) -> DotsResult<yaml::Build> {
    yaml::parse(link::expand_path(matches.value_of("yaml").unwrap())?)
}

#[inline(always)]
fn parse_resolve_build(matches: &ArgMatches) -> DotsResult<(repo::Repo, Vec<yaml::BuildUnit>)> {
    parse_build(matches)?.resolve()
}

#[inline(always)]
fn filter_links(units: Vec<yaml::BuildUnit>) -> Vec<link::Link> {
    units
        .into_iter()
        .filter_map(|x| match x {
            yaml::BuildUnit::Link(ln) => Some(ln),
            _ => None,
        })
        .collect()
}

fn show(matches: ArgMatches) -> DotsResult<()> {
    let build = parse_build(&matches)?;
    let (_, units) = build.resolve()?;
    println!("Locale:\n{:#?}", *yaml::LOCALE);
    println!("_______________________________________");
    println!("Build:\n{:#?}", build);
    println!("_______________________________________");
    println!("Steps:\n{:#?}", units);
    Ok(())
}

fn install(matches: ArgMatches) -> DotsResult<()> {
    let (repo, units) = parse_resolve_build(&matches)?;
    // Optionally clean before install
    let sub = matches.subcommand_matches("install").unwrap();
    if sub.is_present("clean") {
        clean(matches)?;
    }
    println!("Installing dotfiles...");
    repo.require()?;
    link::install_links(filter_links(units))?;
    pack::Shell::Zsh.chsh()?;
    Ok(())
}

fn uninstall(matches: ArgMatches) -> DotsResult<()> {
    let (_, units) = parse_resolve_build(&matches)?;
    println!("Unstalling dotfiles...");
    link::uninstall_links(filter_links(units))?;
    Ok(())
}

fn clean(matches: ArgMatches) -> DotsResult<()> {
    let (_, units) = parse_resolve_build(&matches)?;
    println!("Cleaning invalid links...");
    link::clean_links(filter_links(units))?;
    Ok(())
}

fn edit() -> DotsResult<()> {
    Command::new(env::var("EDITOR").expect("system editor is unset"))
        .arg(env::var("DOTS_REPO_ROOT").expect("dotfile repo root is unset"))
        .output()?;
    Ok(())
}

fn update() -> DotsResult<()> {
    println!("Updating dotfiles...");
    Ok(())
}

fn main() -> DotsResult<()> {
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
        .get_matches();

    match matches.subcommand_name() {
        Some("show") => show(matches),
        Some("install") => install(matches),
        Some("uninstall") => uninstall(matches),
        Some("clean") => clean(matches),
        Some("update") => update(),
        Some("edit") => edit(),
        _ => unreachable!(),
    }
}
