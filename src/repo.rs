use super::error::{DotsError, DotsResult};
use super::link::expand_path;
use git2::Repository;
use serde::Deserialize;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Repo {
    local: String,
    remote: String,
}

impl Repo {
    pub fn open(&self) -> DotsResult<Repository> {
        let repo = Repository::open(expand_path(self.local.as_str())?);
        match repo {
            Err(e) => Err(DotsError::Upstream(Box::new(e))),
            Ok(r) => Ok(r),
        }
    }

    pub fn clone(&self) -> DotsResult<()> {
        match Repository::clone_recurse(self.remote.as_ref(), expand_path(self.local.as_str())?) {
            Err(e) => Err(DotsError::Upstream(Box::new(e))),
            Ok(_) => Ok(()),
        }
    }
}
