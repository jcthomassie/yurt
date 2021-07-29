use super::error::DotsResult;
use super::link::Link;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use whoami::Platform;

#[derive(Debug)]
pub struct Locale {
    user: String,
    os: Platform,
}

impl Locale {
    pub fn auto() -> Self {
        Locale {
            user: whoami::username(),
            os: whoami::platform(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct BuildCase {
    user: Option<String>,
    os: Option<String>,
    pub link: Vec<Link>,
}

impl PartialEq<Locale> for &BuildCase {
    fn eq(&self, other: &Locale) -> bool {
        // TODO: implement complete locale matching
        self.user == None && self.os == None
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all(deserialize = "snake_case"))]
pub enum BuildScope {
    All(BuildCase),
    Case(BuildCase),
    Default(BuildCase),
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Build {
    build: Vec<BuildScope>,
}

impl Build {
    pub fn apply<T>(&self, f: fn(&BuildCase) -> T) -> Vec<T> {
        let locale = Locale::auto();
        let mut default = true;
        self.build
            .iter()
            .filter_map(|scope| match scope {
                BuildScope::All(c) => Some(f(c)),
                BuildScope::Case(c) if c == locale => {
                    default = false;
                    Some(f(c))
                }
                BuildScope::Default(c) if default => Some(f(c)),
                BuildScope::Default(_) => {
                    default = true;
                    None
                }
                _ => None,
            })
            .collect()
    }
}

pub fn parse(path: PathBuf) -> DotsResult<Build> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: Build = serde_yaml::from_reader(reader)?;
    Ok(build)
}
