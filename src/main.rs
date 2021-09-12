mod build;
mod files;
mod repo;
mod shell;

use anyhow::{Context, Result};
use build::{yaml::Config, ResolvedConfig};
use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use log::info;
use std::env;

fn parse_config(matches: &ArgMatches) -> Result<Config> {
    if let Some(yaml_url) = matches.value_of("yaml-url") {
        Config::from_url(yaml_url).context("Failed to parse remote build file")
    } else {
        let yaml = match matches.value_of("yaml") {
            Some(path) => Ok(path.to_string()),
            None => env::var("YURT_BUILD_FILE"),
        }
        .context("Config file not specified")?;
        let path = files::expand_path(&yaml, None).context("Failed to expand path")?;
        Config::from_path(path).context("Failed to parse local build file")
    }
}

#[inline]
fn parse_resolved(matches: &ArgMatches) -> Result<ResolvedConfig> {
    parse_config(matches)?
        .resolve()
        .context("Failed to resolve build")
}

fn show(matches: &ArgMatches) -> Result<()> {
    println!("{:#?}", parse_resolved(matches)?);
    Ok(())
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

fn edit(matches: &ArgMatches) -> Result<()> {
    let editor = env::var("EDITOR").context("System editor is not set")?;
    parse_resolved(matches)?
        .edit(&editor)
        .context("Failed to open dotfiles in editor")?;
    Ok(())
}

fn update() -> Result<()> {
    info!("Updating dotfiles...");
    Ok(())
}

fn main() -> Result<()> {
    let matches = App::new("yurt")
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
        .subcommand(
            App::new("uninstall").about("Uninstalls dotfiles").arg(
                Arg::new("packages")
                    .about("Uninstall packages too")
                    .short('p')
                    .long("packages")
                    .takes_value(false),
            ),
        )
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
        Some("edit") => edit(&matches),
        _ => unreachable!(),
    }
}
