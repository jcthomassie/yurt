use std::fmt;

use anyhow::{Context as _, Result};
use git2::Repository;
use serde::{Deserialize, Serialize};

use super::BuildUnitInterface;
use crate::{
    context::{parse::ObjectKey, Context},
    specs::{BuildUnit, Resolve},
    yaml_example_doc,
};

/// `git` repository
#[doc = yaml_example_doc!("repo.yaml")]
#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Repo {
    /// Local file path
    path: String,
    /// Remote source url
    url: String,
}

impl Repo {
    fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("Failed to open git repository: {self:?}"))
    }

    fn clone(&self) -> Result<Repository> {
        Repository::clone_recurse(&self.url, &self.path)
            .with_context(|| format!("Failed to clone git repository: {self:?}"))
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

impl BuildUnitInterface for Repo {
    fn unit_install(&self, _context: &Context) -> Result<bool> {
        if self.is_available() {
            Ok(false)
        } else {
            self.clone()?;
            Ok(true)
        }
    }

    fn unit_uninstall(&self, _context: &Context) -> Result<bool> {
        // TODO delete repo
        Ok(false)
    }
}

impl Resolve for Repo {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        let new = Self {
            path: context.parse_path(&self.path)?,
            url: context.parse_str(&self.url)?,
        };
        let new_id = new.name()?;
        for (attr, value) in [("path", &new.path), ("url", &new.url)] {
            context
                .variables
                .push(Self::object_key(attr), value.to_string());
            context
                .variables
                .push(Self::object_instance_key(attr, new_id), value.to_string());
        }
        Ok(BuildUnit::Repo(new))
    }
}

impl ObjectKey for Repo {
    const OBJECT_NAME: &'static str = "repo";
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name().unwrap_or("?"))
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
