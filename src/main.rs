mod error;
mod link;
mod pack;
mod repo;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg};
use error::DotsResult;

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
        .subcommand(App::new("show").about("Shows the build config"))
        .arg(
            Arg::new("yaml")
                .about("YAML build file path")
                .short('y')
                .long("yaml")
                .default_value("$HOME/dotfiles/install.yaml"),
        )
        .get_matches();

    let yaml = link::expand_path(matches.value_of("yaml").unwrap()).unwrap();
    let build = yaml::parse(yaml.clone())?;
    let links = build.resolve()?;

    match matches.subcommand_name() {
        Some("show") => {
            println!("Locale:\n{:#?}", *yaml::LOCALE);
            println!("_______________________________________");
            println!("Build:\n{:#?}", build);
            println!("_______________________________________");
            println!("Links:\n{:#?}", links);
        }
        Some("install") => {
            println!("Installing dotfiles...");
            build.repo.require()?;
            link::install_links(links)?;
            pack::Shell::Zsh.chsh()?;
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
            link::uninstall_links(links)?;
        }
        Some("update") => {
            println!("Updating dotfiles...");
            update()?;
        }
        _ => unreachable!(),
    }
    Ok(())
}
