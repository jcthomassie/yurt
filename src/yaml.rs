use super::link::Link;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

#[derive(Debug)]
pub enum YamlError {
    IoError(std::io::Error),
    SerdeError(serde_yaml::Error),
}

impl From<std::io::Error> for YamlError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<serde_yaml::Error> for YamlError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::SerdeError(e)
    }
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct YamlBuild {
    pub build: Vec<Directive>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Directive {
    Link(Link),
}

pub fn parse(path: PathBuf) -> Result<YamlBuild, YamlError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: YamlBuild = serde_yaml::from_reader(reader)?;
    Ok(build)
}
