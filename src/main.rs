use clap::{crate_version, App, AppSettings};

fn main() {
    let matches = App::new("dots")
        .version(crate_version!())
        .setting(AppSettings::ArgRequiredElseHelp)
        .subcommand(App::new("install").about("install dotfiles"))
        .subcommand(App::new("uninstall").about("uninstall dotfiles"))
        .get_matches();

    match matches.subcommand_name() {
        Some("install") => {
            println!("Installing dotfiles...");
        }
        Some("uninstall") => {
            println!("Unstalling dotfiles...");
        }
        _ => unreachable!(),
    }
}
