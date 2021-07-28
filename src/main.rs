mod link;
mod yaml;

use clap::{crate_authors, crate_version, App, AppSettings, Arg};

fn main() -> std::io::Result<()> {
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

    let root = link::expand_path(matches.value_of("root").unwrap());
    println!("ROOT: {:?}", root);
    yaml::parse(root.join("install.yaml"))?;

    match matches.subcommand_name() {
        Some("install") => {
            println!("Installing dotfiles...");
            Ok(())
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
            Ok(())
        }
        Some("update") => {
            println!("Updating dotfiles...");
            Ok(())
        }
        _ => unreachable!(),
    }
}
