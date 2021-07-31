use super::error::DotsResult;
use super::link::Link;
use super::pack::Package;
use super::repo::Repo;
use lazy_static::lazy_static;
use serde::Deserialize;
use std::borrow::Cow;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use whoami;

lazy_static! {
    pub static ref LOCALE: Locale<Cow<'static, str>> = Locale {
        user: Cow::Owned(whoami::username()),
        platform: Cow::Owned(format!("{:?}", whoami::platform())),
        distro: Cow::Owned(
            whoami::distro()
                .split(" ")
                .next()
                .map(String::from)
                .expect("failed to determine distro"),
        ),
    };
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Locale<T> {
    user: T,
    platform: T,
    distro: T,
}

impl Locale<Option<String>> {
    pub fn is_local(&self) -> bool {
        let s_vals = vec![
            self.user.as_deref(),
            self.platform.as_deref(),
            self.distro.as_deref(),
        ];
        let o_vals = vec![
            LOCALE.user.as_ref(),
            LOCALE.platform.as_ref(),
            LOCALE.distro.as_ref(),
        ];
        s_vals
            .into_iter()
            .zip(o_vals.into_iter())
            .all(|(s, o)| match s {
                Some(val) if val != o => false,
                _ => true,
            })
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case {
    Local {
        spec: Locale<Option<String>>,
        build: Vec<BuildSet>,
    },
    Default {
        build: Vec<BuildSet>,
    },
}

#[derive(Debug)]
pub enum BuildUnit {
    Link(Link),
    Package(Package),
}

#[derive(Debug, PartialEq, Deserialize)]
pub enum BuildSet {
    #[serde(rename = "case")]
    CaseVec(Vec<Case>),
    #[serde(rename = "link")]
    LinkVec(Vec<Link>),
    #[serde(rename = "install")]
    PackageVec(Vec<Package>),
}

impl BuildSet {
    // Recursively resolve all case units; collect into single vec
    pub fn resolve(&self) -> DotsResult<Vec<BuildUnit>> {
        match self {
            // Recursively filter case units
            Self::CaseVec(case_vec) => {
                let mut default = true;
                let mut unit_vec: Vec<BuildUnit> = Vec::new();
                for case in case_vec.iter() {
                    match case {
                        Case::Local { spec, build } if spec.is_local() => {
                            default = false;
                            for set in build.iter() {
                                unit_vec.extend(set.resolve()?)
                            }
                        }
                        Case::Default { build } if default => {
                            for set in build.iter() {
                                unit_vec.extend(set.resolve()?)
                            }
                        }
                        _ => (),
                    };
                }
                Ok(unit_vec)
            }
            // Expand links
            Self::LinkVec(link_vec) => link_vec
                .iter()
                .map(|la| la.expand().and_then(|lb| Ok(BuildUnit::Link(lb))))
                .collect(),
            // Clone packages
            Self::PackageVec(pkg_vec) => pkg_vec
                .iter() // comment
                .map(|pkg| Ok(BuildUnit::Package(pkg.clone())))
                .collect(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Build {
    pub repo: Repo,
    pub build: Vec<BuildSet>,
}

impl Build {
    pub fn resolve(&self) -> DotsResult<(Repo, Vec<BuildUnit>)> {
        let repo = self.repo.resolve()?;
        env::set_var("DOTS_REPO_LOCAL", &repo.local);
        let mut build_vec: Vec<BuildUnit> = Vec::new();
        for set in self.build.iter() {
            build_vec.extend(set.resolve()?);
        }
        Ok((repo, build_vec))
    }
}

pub fn parse(path: PathBuf) -> DotsResult<Build> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: Build = serde_yaml::from_reader(reader)?;
    Ok(build)
}
