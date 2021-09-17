use anyhow::Result;
use git2::Repository;
use serde::Deserialize;

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct Repo {
    pub local: String,
    pub remote: String,
}

impl Repo {
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
        self.open().or_else(|_| self.clone())
    }

    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }
}
