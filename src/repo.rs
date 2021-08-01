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
        let repo = Repository::open(&self.local);
        match repo {
            Err(e) => Err(anyhow!(e)),
            Ok(r) => Ok(r),
        }
    }

    pub fn clone(&self) -> Result<Repository> {
        match Repository::clone_recurse(self.remote.as_ref(), &self.local) {
            Err(e) => Err(anyhow!(e)),
            Ok(r) => Ok(r),
        }
    }

    pub fn require(&self) -> Result<Repository> {
        match self.open() {
            Err(_) => self.clone(),
            Ok(r) => Ok(r),
        }
    }
}
