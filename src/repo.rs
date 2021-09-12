use super::yaml::Context;
use anyhow::Result;
use git2::Repository;
use serde::Deserialize;

#[derive(Debug, PartialEq, Deserialize)]
pub struct Repo {
    pub local: String,
    pub remote: String,
}

impl Repo {
    pub fn resolve(self, context: &mut Context) -> Result<Self> {
        Ok(Self {
            local: context.substitute(&self.local)?,
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
