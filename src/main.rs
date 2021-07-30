mod error;
mod link;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg};
use error::{DotsError, DotsResult};
use git2::Repository;
use std::path::PathBuf;
use yaml::BuildCase;

fn open_dotfiles(local: &PathBuf) -> DotsResult<Repository> {
    let repo = Repository::open(local);
    match repo {
        Err(e) => Err(DotsError::Upstream(Box::new(e))),
        Ok(r) => Ok(r),
    }
}

fn clone_dotfiles(local: &PathBuf, remote: &str) -> DotsResult<()> {
    match Repository::clone_recurse(remote, local) {
        Err(e) => Err(DotsError::Upstream(Box::new(e))),
        Ok(_) => Ok(()),
    }
}

fn install(case: &BuildCase) -> DotsResult<()> {
    case.link
        .iter()
        .map(|ln| Ok(ln).and_then(|ln| ln.expand()).and_then(|ln| ln.link()))
        .collect()
}

fn uninstall(case: &BuildCase) -> DotsResult<()> {
    case.link
        .iter()
        .map(|ln| Ok(ln).and_then(|ln| ln.expand()).and_then(|ln| ln.unlink()))
        .collect()
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
    let build = yaml::parse(yaml)?;

    match matches.subcommand_name() {
        Some("show") => {
            println!("{:#?}", build);
            Ok(())
        }
        Some("install") => {
            println!("Installing dotfiles...");
            // TODO: handle errors correctly
            build.apply(install);
            Ok(())
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
            // TODO: handle errors correctly
            build.apply(uninstall);
            Ok(())
        }
        Some("update") => {
            println!("Updating dotfiles...");
            update()
        }
        _ => unreachable!(),
    }
}
