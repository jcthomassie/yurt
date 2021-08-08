use anyhow::{anyhow, Result};
use git2::Repository;
use serde::Deserialize;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Repo {
    pub local: PathBuf,
    pub remote: Url,
}

impl Repo {
    pub fn resolve(&self) -> Result<Self> {
        Ok(Self {
            local: PathBuf::from(
                shellexpand::full(
                    self.local
                        .to_str()
                        .ok_or_else(|| anyhow!("failed to parse path"))?,
                )?
                .to_string(),
            ),
            remote: self.remote.clone(),
        })
    }

    pub fn open(&self) -> Result<Repository> {
        Ok(Repository::open(&self.local)?)
    }

    pub fn clone(&self) -> Result<Repository> {
        Ok(Repository::clone_recurse(
            self.remote.as_ref(),
            &self.local,
        )?)
    }

    pub fn require(&self) -> Result<Repository> {
        match self.open() {
            Err(_) => self.clone(),
            Ok(r) => Ok(r),
        }
    }
}
