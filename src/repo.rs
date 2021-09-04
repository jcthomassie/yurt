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
    pub fn resolve(self) -> Result<Self> {
        Ok(Self {
            local: PathBuf::from(
                shellexpand::full(
                    self.local
                        .to_str()
                        .ok_or_else(|| anyhow!("failed to parse path"))?,
                )?
                .to_string(),
            ),
            ..self
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

    #[inline]
    pub fn require(&self) -> Result<Repository> {
        self.open().or_else(|_| self.clone())
    }
}
