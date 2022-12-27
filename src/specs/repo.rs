use crate::specs::{BuildUnit, Context, Resolve};

use anyhow::{Context as _, Result};
use git2::Repository;
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Repo {
    path: String,
    url: String,
}

impl Repo {
    pub fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("Failed to open git repository: {self:?}"))
    }

    pub fn clone(&self) -> Result<Repository> {
        Repository::clone_recurse(&self.url, &self.path)
            .with_context(|| format!("Failed to clone git repository: {self:?}"))
    }

    pub fn require(&self) -> Result<Repository> {
        self.open().or_else(|_| self.clone())
    }

    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }

    fn name(&self) -> Result<&str> {
        self.path
            .split(&['/', '\\'])
            .last()
            .filter(|name| !name.is_empty())
            .context("Repo name is empty")
    }
}

impl Resolve for Repo {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        let new = Self {
            path: context.parse_path(&self.path)?,
            ..self
        };
        context
            .variables
            .push(new.name()?, [("path", &new.path)].into_iter());
        Ok(BuildUnit::Repo(new))
    }
}

#[cfg(test)]
mod tests {
    use super::Repo;

    fn repo(path: &str) -> Repo {
        Repo {
            path: path.to_string(),
            url: "repo-url".to_string(),
        }
    }

    #[test]
    fn empty_path() {
        assert!(repo("").name().is_err());
    }

    #[test]
    fn unix_path() {
        assert_eq!(repo("path/to/my-repo").name().unwrap(), "my-repo");
    }

    #[test]
    fn windows_path() {
        assert_eq!(repo("path\\to\\my-repo").name().unwrap(), "my-repo");
    }
}
