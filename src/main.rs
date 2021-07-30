mod error;
mod link;
mod pack;
mod repo;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches};
use error::DotsResult;

#[inline(always)]
fn parse_build(matches: &ArgMatches) -> DotsResult<yaml::Build> {
    yaml::parse(link::expand_path(matches.value_of("yaml").unwrap())?)
}

#[inline(always)]
fn parse_resolve_build(matches: &ArgMatches) -> DotsResult<(repo::Repo, Vec<link::Link>)> {
    parse_build(matches)?.resolve()
}

fn show(matches: ArgMatches) -> DotsResult<()> {
    let build = parse_build(&matches)?;
    let (_, links) = build.resolve()?;
    println!("Locale:\n{:#?}", *yaml::LOCALE);
    println!("_______________________________________");
    println!("Build:\n{:#?}", build);
    println!("_______________________________________");
    println!("Links:\n{:#?}", links);
    Ok(())
}

fn install(matches: ArgMatches) -> DotsResult<()> {
    let (repo, links) = parse_resolve_build(&matches)?;
    // Optionally clean before install
    let sub = matches.subcommand_matches("install").unwrap();
    if sub.is_present("clean") {
        clean(matches)?;
    }
    println!("Installing dotfiles...");
    repo.require()?;
    link::install_links(links)?;
    pack::Shell::Zsh.chsh()?;
    Ok(())
}

fn uninstall(matches: ArgMatches) -> DotsResult<()> {
    let (_, links) = parse_resolve_build(&matches)?;
    println!("Unstalling dotfiles...");
    link::uninstall_links(links)?;
    Ok(())
}

fn clean(matches: ArgMatches) -> DotsResult<()> {
    let (_, links) = parse_resolve_build(&matches)?;
    println!("Cleaning invalid links...");
    link::clean_links(links)?;
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
        .subcommand(App::new("clean").about("Cleans output destinations"))
        .subcommand(App::new("update").about("Updates dotfiles and/or system"))
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
        _ => unreachable!(),
    }
}
