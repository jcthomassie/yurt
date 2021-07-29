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
            // TODO: handle errors correctly
            yaml::parse(root.join("install.yaml"))?.apply(install);
            Ok(())
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
            // TODO: handle errors correctly
            yaml::parse(root.join("install.yaml"))?.apply(uninstall);
            Ok(())
        }
        Some("update") => {
            println!("Updating dotfiles...");
            update()
        }
        _ => unreachable!(),
    }
}
