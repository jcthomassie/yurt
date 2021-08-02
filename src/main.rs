mod link;
mod pack;
mod repo;
mod yaml;

use anyhow::{Context, Result};
use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use log::info;
use std::env;
use std::process::Command;
use yaml::{Build, BuildUnit};

#[inline]
fn parse_build(matches: &ArgMatches) -> Result<Build> {
    if let Some(yaml_url) = matches.value_of("yaml-url") {
        Build::from_url(yaml_url).context("Failed to parse remote build file")
    } else {
        let yaml = match matches.value_of("yaml") {
            Some(path) => Ok(path.to_string()),
            None => env::var("YURT_BUILD_FILE"),
        }
        .context("Build file not specified")?;
        let path = link::expand_path(&yaml).context("Failed to expand path")?;
        Build::from_path(path).context("Failed to parse local build file")
    }
}

#[inline]
fn parse_resolve_build(matches: &ArgMatches) -> Result<(repo::Repo, Vec<BuildUnit>)> {
    parse_build(matches)?
        .resolve()
        .context("Failed to resolve build")
}

macro_rules! skip {
    () => {
        |_| Ok(())
    };
}

fn show(matches: &ArgMatches) -> Result<()> {
    let (repo, units) = parse_resolve_build(matches)?;
    println!("Locale:\n{:#?}", *yaml::LOCALE);
    println!("_______________________________________");
    println!("Repo:\n{:#?}", repo);
    println!("_______________________________________");
    println!("Steps:\n{:#?}", units);
    Ok(())
}

fn install(matches: &ArgMatches) -> Result<()> {
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
    )
    .context("Failed to complete install steps")?;
    pack::Shell::Zsh.chsh()?;
    Ok(())
}

fn uninstall(matches: &ArgMatches) -> Result<()> {
    let (_, units) = parse_resolve_build(matches)?;
    info!("Uninstalling dotfiles...");
    yaml::apply(units, |ln| ln.unlink(), skip!(), skip!())
        .context("Failed to complete uninstall steps")?;
    Ok(())
}

fn clean(matches: &ArgMatches) -> Result<()> {
    let (_, units) = parse_resolve_build(matches)?;
    info!("Cleaning link heads...");
    yaml::apply(units, |ln| ln.clean(), skip!(), skip!()).context("Failed to clean link heads")?;
    Ok(())
}

fn edit() -> Result<()> {
    Command::new(env::var("EDITOR").context("System editor is not set")?)
        .arg(env::var("YURT_REPO_ROOT").context("dotfile repo root is not set")?)
        .output()
        .context("Failed to open dotfiles in editor")?;
    Ok(())
}

fn update() -> Result<()> {
    info!("Updating dotfiles...");
    Ok(())
}

fn main() -> Result<()> {
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
                .takes_value(true),
        )
        .arg(
            Arg::new("yaml-url")
                .about("YAML build file URL")
                .long("yaml-url")
                .takes_value(true)
                .conflicts_with("yaml"),
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
