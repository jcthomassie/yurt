use super::error::DotsResult;
use super::link::Link;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use whoami;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Repo {
    local: String,
    remote: String,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Locale {
    user: Option<String>,
    platform: Option<String>,
    distro: Option<String>,
}

impl Locale {
    pub fn auto() -> Self {
        Self {
            user: Some(whoami::username()),
            platform: Some(format!("{:?}", whoami::platform())),
            distro: whoami::distro().split(" ").next().map(String::from),
        }
    }

    pub fn is_subset(&self, other: &Self) -> bool {
        (self.user.is_none() || self.user == other.user)
            && (self.platform.is_none() || self.platform == other.platform)
            && (self.distro.is_none() || self.distro == other.distro)
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum Case {
    Local { spec: Locale, build: Vec<BuildUnit> },
    Default { build: Vec<BuildUnit> },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum BuildUnit {
    Case(Vec<Case>),
    Link(Vec<Link>),
}

impl BuildUnit {
    // Recursively resolve all case units; collect into single vec
    pub fn resolve(&self) -> Vec<Link> {
        match self {
            Self::Case(cv) => {
                // TODO: make locale static
                let locale = Locale::auto();
                let mut default = true;
                // Build flat vec of links
                let mut ln: Vec<Link> = Vec::new();
                for case in cv.into_iter() {
                    match case {
                        Case::Local { spec, build } if spec.is_subset(&locale) => {
                            default = false;
                            for unit in build.into_iter() {
                                ln.extend(unit.resolve())
                            }
                        }
                        Case::Default { build } if default => {
                            for unit in build.into_iter() {
                                ln.extend(unit.resolve())
                            }
                        }
                        _ => (),
                    };
                }
                ln
            }
            Self::Link(ln) => ln.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Build {
    pub repo: Repo,
    pub build: Vec<BuildUnit>,
}

impl Build {
    pub fn resolve(&self) -> Vec<Link> {
        let mut ln: Vec<Link> = Vec::new();
        for unit in self.build.iter() {
            ln.extend(unit.resolve());
        }
        ln
    }
}

pub fn parse(path: PathBuf) -> DotsResult<Build> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: Build = serde_yaml::from_reader(reader)?;
    Ok(build)
}
