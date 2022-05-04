use crate::build::{self, BuildUnit, Resolve};
use anyhow::{anyhow, Context, Result};
use git2::{build::CheckoutBuilder, Repository};
use serde::{Deserialize, Serialize};

fn default_branch() -> String {
    "main".to_string()
}

fn default_remote() -> String {
    "origin".to_string()
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Repo {
    path: String,
    url: String,
    #[serde(default = "default_branch")]
    branch: String,
    #[serde(default = "default_remote")]
    remote: String,
}

impl Repo {
    pub fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("Failed to open git repository: {:?}", self))
    }

    pub fn clone_recurse(&self) -> Result<Repository> {
        Repository::clone_recurse(&self.url, &self.path)
            .with_context(|| format!("Failed to clone git repository: {:?}", self))
    }

    pub fn require(&self) -> Result<Repository> {
        self.open().or_else(|_| self.clone_recurse())
    }

    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }

    pub fn pull(&self) -> Result<bool> {
        let repo = self.require()?;
        // Fetch remote
        repo.find_remote(&self.remote)?
            .fetch(&[&self.branch], None, None)?;
        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        // Try fast-forward merge
        let analysis = repo.merge_analysis(&[&fetch_commit])?;
        if analysis.0.is_up_to_date() {
            Ok(false)
        } else if analysis.0.is_fast_forward() {
            let refname = format!("refs/heads/{}", &self.branch);
            let mut reference = repo.find_reference(&refname)?;
            reference.set_target(fetch_commit.id(), "Fast-Forward")?;
            repo.set_head(&refname)?;
            repo.checkout_head(Some(CheckoutBuilder::default().force()))?;
            Ok(true)
        } else {
            Err(anyhow!("Could not perform fast-forward merge"))
        }
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
    fn resolve(self, context: &mut build::Context) -> Result<BuildUnit> {
        let new = Self {
            path: context.replace_variables(&self.path)?,
            ..self
        };
        context.set_variable(new.name()?, "path", &new.path);
        Ok(BuildUnit::Repo(new))
    }
}

#[cfg(test)]
mod tests {
    use super::Repo;
    use lazy_static::lazy_static;

    // Assumes cargo is run from repository root
    lazy_static! {
        pub static ref REPO: Repo = Repo {
            path: "./".to_string(),
            url: "https://github.com/jcthomassie/yurt.git".to_string(),
            branch: "main".to_string(),
            remote: "origin".to_string(),
        };
    }

    #[test]
    fn open() {
        REPO.open().unwrap();
    }

    #[test]
    fn require() {
        REPO.require().unwrap();
    }

    #[test]
    fn is_available() {
        assert!(REPO.is_available());
    }

    fn repo(path: &str) -> Repo {
        serde_yaml::from_str(&format!("{{ path: {}, url: www.web.site }}", path)).unwrap()
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
