#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::match_bool,
    clippy::module_name_repetitions,
    clippy::single_match_else
)]
mod config;
mod context;
mod specs;

use self::{
    config::{Config, ResolvedConfig},
    context::Context,
    specs::{BuildUnit, BuildUnitKind, Hook},
};
use anyhow::{bail, Context as _, Result};
use clap::{command, ArgGroup, Parser, Subcommand};
use std::{
    env,
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(arg_required_else_help(true))]
#[command(group(
    ArgGroup::new("build_source")
        .args(["file", "file_url"])
))]
#[command(group(
    ArgGroup::new("filter")
        .args(["include", "exclude"])
))]
pub struct YurtArgs {
    /// YAML build file path
    #[arg(long, short = 'f', value_name = "FILE")]
    file: Option<PathBuf>,

    /// YAML build file URL
    #[arg(long, short = 'u', value_name = "URL")]
    file_url: Option<String>,

    /// Logging level
    #[arg(long)]
    log: Option<String>,

    /// Allow yurt to run as root user
    #[arg(long)]
    root: bool,

    /// Override target username
    #[arg(long, value_name = "USER")]
    override_user: Option<String>,

    /// Override target platform
    #[arg(long, value_name = "PLATFORM")]
    override_platform: Option<String>,

    /// Override target distro
    #[arg(long, value_name = "DISTRO")]
    override_distro: Option<String>,

    /// Include only the specified build unit types
    #[arg(
        value_enum,
        long,
        short = 'I',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    include: Option<Vec<BuildUnitKind>>,

    /// Exclude the specified build unit types
    #[arg(
        value_enum,
        long,
        short = 'E',
        value_delimiter = ',',
        value_name = "TYPE"
    )]
    exclude: Option<Vec<BuildUnitKind>>,

    #[command(subcommand)]
    action: YurtAction,
}

#[derive(Subcommand, Debug)]
enum YurtAction {
    /// Show the resolved build
    #[command(group(
        ArgGroup::new("modifier")
            .args(["raw", "nontrivial"])
    ))]
    Show {
        /// Print unresolved config/context
        #[arg(long, short)]
        raw: bool,

        /// Hide trivial build units
        #[arg(long, short)]
        nontrivial: bool,

        /// Print the build context
        #[arg(long, short)]
        context: bool,
    },

    /// Install the resolved build
    Install {
        /// Clean link target conflicts
        #[arg(long, short)]
        clean: bool,
    },

    /// Uninstall the resolved build
    Uninstall,

    /// Run a custom build hook
    Custom {
        /// Name of the custom hook to run
        name: String,
    },
}

fn main() -> Result<()> {
    let timer = Instant::now();
    let args = YurtArgs::parse();

    if let Some(level) = &args.log {
        env::set_var("RUST_LOG", level);
    }
    env_logger::init();
    log::debug!("{:#?}", &args);

    if !&args.root && whoami::username() == "root" {
        bail!(
            "Running as root user requires the `--root` argument. \
            Use `sudo -u my-username` to run as an elevated non-root user."
        );
    }

    log::info!("{:?}", &args.action);
    let mut context = Context::from(&args);
    let result = match args.action {
        YurtAction::Show {
            raw, context: true, ..
        } => {
            if !raw {
                ResolvedConfig::resolve_from(&args, &mut context)?;
            }
            writeln!(io::stdout(), "{context:#?}").context("Failed to write context to stdout")
        }
        YurtAction::Show {
            raw, nontrivial, ..
        } => {
            let config = if raw {
                Config::try_from(&args)?
            } else {
                let resolved = ResolvedConfig::resolve_from(&args, &mut context)?;
                if nontrivial {
                    let context = resolved.context;
                    resolved
                        .filter(|unit| match unit {
                            BuildUnit::Repo(repo) => !repo.is_available(),
                            BuildUnit::Link(link) => !link.is_valid(),
                            BuildUnit::Package(package) => !package.is_installed(context),
                            BuildUnit::PackageManager(manager) => !manager.is_available(),
                            BuildUnit::Hook(hook) => hook.applies(&Hook::Install),
                        })
                        .into_config()
                } else {
                    resolved.into_config()
                }
            };
            writeln!(io::stdout(), "{}", config.yaml()?).context("Failed to write yaml to stdout")
        }
        YurtAction::Install { clean } => ResolvedConfig::resolve_from(&args, &mut context)
            .and_then(|build| {
                build.for_each_unit(|unit| match unit {
                    BuildUnit::Repo(repo) => repo.require().map(drop),
                    BuildUnit::Link(link) => link.link(clean),
                    BuildUnit::Hook(hook) => hook.exec_for(&Hook::Install),
                    BuildUnit::Package(package) => package.install(build.context),
                    BuildUnit::PackageManager(manager) => manager.require(),
                })
            }),
        YurtAction::Uninstall => ResolvedConfig::resolve_from(&args, &mut context) //
            .and_then(|build| {
                build.for_each_unit(|unit| match unit {
                    BuildUnit::Link(link) => link.unlink(),
                    BuildUnit::Hook(hook) => hook.exec_for(&Hook::Uninstall),
                    BuildUnit::Package(package) => package.uninstall(build.context),
                    _ => Ok(()),
                })
            }),
        YurtAction::Custom { ref name } => ResolvedConfig::resolve_from(&args, &mut context) //
            .and_then(|build| {
                build.for_each_unit(|unit| match unit {
                    BuildUnit::Hook(hook) => hook.exec_for(&Hook::Custom(name.clone())),
                    _ => Ok(()),
                })
            }),
    }
    .with_context(|| format!("Action failed: {:?}", args.action));
    log::debug!("Runtime: {:?}", timer.elapsed());
    result
}
