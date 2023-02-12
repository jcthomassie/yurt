use crate::{
    context::{parse::ObjectKey, Context},
    specs::{BuildUnit, Resolve},
};
use anyhow::{anyhow, Context as _, Result};
use git2::{build::CheckoutBuilder, Cred, RemoteCallbacks, Repository};
use serde::{Deserialize, Serialize};

const DEFAULT_BRANCH: &str = "main";
const DEFAULT_REMOTE: &str = "origin";

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Repo {
    path: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssh_key_path: Option<String>,
}

impl Repo {
    fn fetch_options(&self) -> git2::FetchOptions {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, user_from_url, allowed| {
            let user = user_from_url.unwrap_or("git");
            if allowed.contains(git2::CredentialType::USERNAME) {
                log::debug!("Using username as git credential");
                Cred::username(user)
            } else if let Some(key) = &self.ssh_key_path {
                log::debug!("Using ssh key as git credential");
                Cred::ssh_key(user, None, key.as_ref(), None)
            } else {
                log::debug!("Using default git credential");
                Cred::default()
            }
        });

        let mut opts = git2::FetchOptions::new();
        opts.remote_callbacks(callbacks);
        opts.download_tags(git2::AutotagOption::All);
        opts
    }

    fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("Failed to open git repository: {self:?}"))
    }

    fn clone(&self) -> Result<Repository> {
        log::info!("Cloning repository {} into {}", self.url, self.path);
        let branch = self.branch.as_deref().unwrap_or(DEFAULT_BRANCH);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(self.fetch_options());
        builder.branch(branch);
        builder
            .clone(&self.url, self.path.as_ref())
            .with_context(|| format!("Failed to clone git repository: {self:?}"))
        // TODO: init submodules/recurse
    }

    #[inline]
    pub fn require(&self) -> Result<Repository> {
        self.open().or_else(|_| self.clone())
    }

    #[inline]
    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }

    pub fn pull(&self) -> Result<bool> {
        log::info!("Updating repository: {}", self.path);
        let repo = self.open()?;
        let branch = self.branch.as_deref().unwrap_or(DEFAULT_BRANCH);
        let remote = self.remote.as_deref().unwrap_or(DEFAULT_REMOTE);
        // Fetch remote
        repo.find_remote(remote)
            .with_context(|| anyhow!("Failed to find repo remote {remote:?}"))?
            .fetch(&[branch], Some(&mut self.fetch_options()), None)
            .with_context(|| anyhow!("Failed to fetch remote branch {branch:?}"))?;
        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        // Try fast-forward merge
        let (analysis, _) = repo.merge_analysis(&[&fetch_commit])?;
        if analysis.is_up_to_date() {
            Ok(false)
        } else if analysis.is_fast_forward() {
            let refname = format!("refs/heads/{branch}");
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

impl ObjectKey for Repo {
    const OBJECT_NAME: &'static str = "repo";
}

impl Resolve for Repo {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        let new = Self {
            path: context.parse_path(&self.path)?,
            url: context.parse_str(&self.url)?,
            ssh_key_path: self
                .ssh_key_path
                .map(|key| context.parse_path(&key))
                .transpose()?,
            ..self
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

#[cfg(test)]
mod tests {
    use super::Repo;
    use lazy_static::lazy_static;

    lazy_static! {
        pub static ref REPO: Repo = Repo {
            path: env!("CARGO_MANIFEST_DIR").to_string(),
            url: "https://github.com/jcthomassie/yurt.git".to_string(),
            branch: Some("main".to_string()),
            remote: Some("origin".to_string()),
            ssh_key_path: None,
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
        serde_yaml::from_str(&format!("{{ path: {path}, url: www.web.site }}")).unwrap()
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
