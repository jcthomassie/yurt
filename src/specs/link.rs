use std::{fmt, fs, path::PathBuf};

use anyhow::{anyhow, Context as _, Error, Result};
use serde::{Deserialize, Serialize};

use super::{BuildUnitInterface, BuildUnitKind};
use crate::{
    specs::{BuildUnit, Context, Resolve},
    yaml_example_doc,
};

#[derive(Debug)]
enum Status {
    Valid,
    NullSource,
    NullTarget,
    InvalidSource(Error),
    InvalidTarget(Error),
}

/// Symbolic link representation ([`source`][Self::source] -> [`target`][Self::target])
#[doc = yaml_example_doc!("link.yaml")]
#[derive(Debug, PartialEq, Deserialize, Serialize, Clone)]
pub struct Link {
    /// Path of the real source file
    source: PathBuf,
    /// Path of the symbolic link
    target: PathBuf,
}

impl Link {
    fn new<S, T>(source: S, target: T) -> Self
    where
        S: Into<PathBuf>,
        T: Into<PathBuf>,
    {
        Self {
            source: source.into(),
            target: target.into(),
        }
    }

    /// Get current status of link
    fn status(&self) -> Status {
        if !self.target.exists() {
            return Status::NullTarget;
        }
        match self.source.read_link() {
            Ok(target) if target == self.target => Status::Valid,
            Ok(target) => Status::InvalidTarget(anyhow!(
                "Link source points to wrong target: {}",
                Self::new(self.source.clone(), target)
            )),
            Err(e) if self.source.exists() => Status::InvalidSource(anyhow!(e)),
            Err(_) => Status::NullSource,
        }
    }

    /// Return true if the link is valid
    pub fn is_valid(&self) -> bool {
        matches!(self.status(), Status::Valid)
    }

    /// Remove any conflicting files/links at source
    pub fn clean(&self, context: &Context) -> Result<()> {
        match self.status() {
            Status::InvalidSource(_) | Status::InvalidTarget(_) => {
                fs::remove_file(&self.source)
                    .with_context(|| format!("Failed to clean link source: {self}"))?;
                context.write_warning(
                    BuildUnitKind::Link,
                    self.source.to_string_lossy(),
                    "removed",
                )
            }
            _ => Ok(()),
        }
    }
}

impl BuildUnitInterface for Link {
    /// Try to create link if it does not already exist
    fn unit_install(&self, context: &Context) -> Result<bool> {
        // TODO improve clean handling
        if false {
            self.clean(context)?;
        }
        match self.status() {
            Status::Valid => Ok(false),
            Status::NullSource => {
                if let Some(dir) = self.source.parent() {
                    fs::create_dir_all(dir)?;
                }
                symlink::symlink_auto(&self.target, &self.source)
                    .with_context(|| format!("Failed to apply symlink: {self}"))?;
                Ok(true)
            }
            Status::NullTarget => Err(anyhow!("Link target does not exist")),
            Status::InvalidSource(e) => Err(e.context("Invalid link source")),
            Status::InvalidTarget(e) => Err(e.context("Invalid link target")),
        }
    }

    /// Try to remove link if it exists
    fn unit_uninstall(&self, _context: &Context) -> Result<bool> {
        match self.status() {
            Status::Valid => {
                if self.target.is_file() {
                    symlink::remove_symlink_file(&self.source)
                } else {
                    symlink::remove_symlink_dir(&self.source)
                }
                .with_context(|| format!("Failed to remove symlink: {self}"))?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} >> {}",
            &self.source.to_string_lossy(),
            &self.target.to_string_lossy()
        )
    }
}

impl Resolve for Link {
    fn resolve(self, context: &mut Context) -> Result<BuildUnit> {
        Ok(BuildUnit::Link(Self::new(
            context.parse_path(self.source.to_str().unwrap_or(""))?,
            context.parse_path(self.target.to_str().unwrap_or(""))?,
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;

    fn fixture() -> (tempfile::TempDir, Link) {
        let dir = tempfile::tempdir().expect("Failed to create tempdir");
        let link = Link::new(
            dir.path().join("link.source"),
            dir.path().join("link.target"),
        );
        (dir, link)
    }

    #[test]
    fn status_no_target() {
        let (_dir, link) = fixture();
        assert!(matches!(link.status(), Status::NullTarget));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_no_source() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        assert!(matches!(link.status(), Status::NullSource));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_valid() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        symlink::symlink_file(&link.target, &link.source).expect("Failed to create symlink");
        assert!(matches!(link.status(), Status::Valid));
        assert!(link.is_valid());
    }

    #[test]
    fn status_wrong_target() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.target).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.source).expect("Failed to create symlink");
        assert!(matches!(link.status(), Status::InvalidTarget(_)));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_source_is_file() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        File::create(&link.source).expect("Failed to create tempfile");
        assert!(matches!(link.status(), Status::InvalidSource(_)));
        assert!(!link.is_valid());
    }

    #[test]
    fn link_normal() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        // Link once
        link.unit_install(&Context::default())
            .expect("Failed to create link");
        assert_eq!(
            link.source.read_link().expect("Failed to read link"),
            link.target
        );
    }

    #[test]
    fn link_create_parent_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create tempdir");
        let link = Link::new(
            dir.path().join("parent").join("link.source"),
            dir.path().join("link.target"),
        );
        File::create(&link.target).expect("Failed to create tempfile");
        // Link once
        link.unit_install(&Context::default())
            .expect("Failed to create link");
        assert_eq!(
            link.source.read_link().expect("Failed to read link"),
            link.target
        );
    }

    #[test]
    fn unlink_normal() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        // Link and unlink once
        link.unit_install(&Context::default())
            .expect("Failed to create link");
        link.unit_uninstall(&Context::default())
            .expect("Failed to remove link");
        assert!(!link.source.exists());
    }

    #[test]
    fn clean_invalid_source() {
        let (_dir, link) = fixture();
        File::create(&link.target).expect("Failed to create tempfile");
        File::create(&link.source).expect("Failed to create tempfile");
        link.clean(&Context::default())
            .expect("Failed to clean link");
        link.unit_install(&Context::default())
            .expect("Failed to apply link");
    }

    #[test]
    fn clean_invalid_target() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.target).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.source).expect("Failed to create symlink");
        link.clean(&Context::default())
            .expect("Failed to clean link");
        link.unit_install(&Context::default())
            .expect("Failed to apply link");
    }

    #[test]
    fn clean_broken_link() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.target).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.source).expect("Failed to create symlink");
        fs::remove_file(&wrong).expect("Failed to delete target");
        link.clean(&Context::default())
            .expect("Failed to clean link");
        link.unit_install(&Context::default())
            .expect("Failed to apply link");
    }
}
