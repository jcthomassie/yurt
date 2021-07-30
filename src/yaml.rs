use super::error::DotsResult;
use super::link::Link;
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
        build: Vec<BuildUnit>,
    },
    Default {
        build: Vec<BuildUnit>,
    },
}

#[derive(Debug, PartialEq, Deserialize)]
pub enum BuildUnit {
    #[serde(rename = "case")]
    CaseVec(Vec<Case>),
    #[serde(rename = "link")]
    LinkVec(Vec<Link>),
}

impl BuildUnit {
    // Recursively resolve all case units; collect into single vec
    pub fn resolve(&self) -> DotsResult<Vec<Link>> {
        match self {
            // Recursively filter case units
            Self::CaseVec(cv) => {
                let mut default = true;
                let mut ln: Vec<Link> = Vec::new();
                for case in cv.iter() {
                    match case {
                        Case::Local { spec, build } if spec.is_local() => {
                            default = false;
                            for unit in build.iter() {
                                ln.extend(unit.resolve()?)
                            }
                        }
                        Case::Default { build } if default => {
                            for unit in build.iter() {
                                ln.extend(unit.resolve()?)
                            }
                        }
                        _ => (),
                    };
                }
                Ok(ln)
            }
            // Expand links
            Self::LinkVec(ln) => ln.iter().map(|ln| ln.expand()).collect(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Build {
    pub repo: Repo,
    pub build: Vec<BuildUnit>,
}

impl Build {
    pub fn resolve(&self) -> DotsResult<Vec<Link>> {
        let repo = self.repo.resolve()?;
        env::set_var("DOTS_REPO_LOCAL", repo.local);
        let mut ln: Vec<Link> = Vec::new();
        for unit in self.build.iter() {
            ln.extend(unit.resolve()?);
        }
        Ok(ln)
    }
}

pub fn parse(path: PathBuf) -> DotsResult<Build> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: Build = serde_yaml::from_reader(reader)?;
    Ok(build)
}
