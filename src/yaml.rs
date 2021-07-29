use super::error::DotsResult;
use super::link::Link;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Deserialize)]
pub struct YamlBuild {
    pub build: Vec<Directive>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Directive {
    Link(Link),
}

pub fn parse(path: PathBuf) -> DotsResult<YamlBuild> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let build: YamlBuild = serde_yaml::from_reader(reader)?;
    Ok(build)
}
