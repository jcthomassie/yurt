mod error;
mod link;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg};
use error::DotsResult;
use yaml::{Directive, YamlBuild};

fn install(build: YamlBuild) -> DotsResult<()> {
    for item in build.build.iter() {
        match item {
            Directive::Link(ln) => ln.expand()?.link()?,
        };
    }
    Ok(())
}

fn uninstall(build: YamlBuild) -> DotsResult<()> {
    for item in build.build.iter() {
        match item {
            Directive::Link(ln) => ln.expand()?.unlink()?,
        };
    }
    Ok(())
}

fn update() -> DotsResult<()> {
    Ok(())
}

fn main() -> DotsResult<()> {
    let matches = App::new("dots")
        .author(crate_authors!())
        .version(crate_version!())
        .about("Simple CLI tool for dotfile management.")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(App::new("install").about("Installs dotfiles"))
        .subcommand(App::new("uninstall").about("Uninstalls dotfiles"))
        .subcommand(App::new("update").about("Updates dotfiles and/or system"))
        .arg(
            Arg::new("root")
                .about("Dotfile repo root directory")
                .short('r')
                .long("root")
                .default_value("$HOME/dotfiles"),
        )
        .get_matches();

    let root = link::expand_path(matches.value_of("root").unwrap()).unwrap();

    match matches.subcommand_name() {
        Some("install") => {
            println!("Installing dotfiles...");
            install(yaml::parse(root.join("install.yaml"))?)
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
            uninstall(yaml::parse(root.join("install.yaml"))?)
        }
        Some("update") => {
            println!("Updating dotfiles...");
            update()
        }
        _ => unreachable!(),
    }
}
