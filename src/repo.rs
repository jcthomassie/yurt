use super::error::{DotsError, DotsResult};
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
    pub fn resolve(&self) -> DotsResult<Self> {
        Ok(Self {
            local: PathBuf::from(
                shellexpand::full(self.local.to_str().ok_or("failed to parse path")?)?.to_string(),
            ),
            remote: self.remote.clone(),
        })
    }

    pub fn open(&self) -> DotsResult<Repository> {
        let repo = Repository::open(&self.local);
        match repo {
            Err(e) => Err(DotsError::Upstream(Box::new(e))),
            Ok(r) => Ok(r),
        }
    }

    pub fn clone(&self) -> DotsResult<Repository> {
        match Repository::clone_recurse(self.remote.as_ref(), &self.local) {
            Err(e) => Err(DotsError::Upstream(Box::new(e))),
            Ok(r) => Ok(r),
        }
    }

    pub fn require(&self) -> DotsResult<Repository> {
        match self.open() {
            Err(_) => self.clone(),
            Ok(r) => Ok(r),
        }
    }
}
