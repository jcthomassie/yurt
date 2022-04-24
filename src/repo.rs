use anyhow::{Context, Result};
use git2::Repository;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Repo {
    pub path: String,
    pub url: String,
}

impl Repo {
    pub fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("Failed to open git repository: {:?}", self))
    }

    pub fn clone(&self) -> Result<Repository> {
        Repository::clone_recurse(&self.url, &self.path)
            .with_context(|| format!("Failed to clone git repository: {:?}", self))
    }

    pub fn require(&self) -> Result<Repository> {
        self.open().or_else(|_| self.clone())
    }

    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }
}
