use super::error::{YurtError, YurtResult};
use git2::Repository;
use serde::Deserialize;
use shellexpand;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Repo {
    pub local: PathBuf,
    pub remote: Url,
}

impl Repo {
    pub fn resolve(&self) -> YurtResult<Self> {
        Ok(Self {
            local: PathBuf::from(
                shellexpand::full(self.local.to_str().ok_or("failed to parse path")?)?.to_string(),
            ),
            remote: self.remote.clone(),
        })
    }

    pub fn open(&self) -> YurtResult<Repository> {
        let repo = Repository::open(&self.local);
        match repo {
            Err(e) => Err(YurtError::Upstream(Box::new(e))),
            Ok(r) => Ok(r),
        }
    }

    pub fn clone(&self) -> YurtResult<Repository> {
        match Repository::clone_recurse(self.remote.as_ref(), &self.local) {
            Err(e) => Err(YurtError::Upstream(Box::new(e))),
            Ok(r) => Ok(r),
        }
    }

    pub fn require(&self) -> YurtResult<Repository> {
        match self.open() {
            Err(_) => self.clone(),
            Ok(r) => Ok(r),
        }
    }
}
