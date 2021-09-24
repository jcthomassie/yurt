use anyhow::{anyhow, Error, Result};
use log::info;
use serde::Deserialize;
use std::{fs, path::PathBuf};

#[derive(Debug)]
enum Status {
    Valid,
    NullHead,
    NullTail,
    InvalidHead(Error),
    InvalidTail(Error),
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct Link {
    // head@ -> tail
    pub head: PathBuf,
    pub tail: PathBuf,
}

impl Link {
    pub fn new<P: Into<PathBuf>>(head: P, tail: P) -> Self {
        Self {
            head: head.into(),
            tail: tail.into(),
        }
    }

    // Get current status of link
    fn status(&self) -> Status {
        if !self.tail.exists() {
            return Status::NullTail;
        }
        match self.head.read_link() {
            Ok(target) if target == self.tail => Status::Valid,
            Ok(target) => Status::InvalidTail(anyhow!(
                "Link source points to wrong target: {:?} -> {:?}",
                self.head,
                target
            )),
            Err(e) if self.head.exists() => Status::InvalidHead(anyhow!(e)),
            Err(_) => Status::NullHead,
        }
    }

    // Return true if the link is valid
    pub fn is_valid(&self) -> bool {
        matches!(self.status(), Status::Valid)
    }

    // Try to create link if it does not already exist
    pub fn link(&self, clean: bool) -> Result<()> {
        if clean {
            self.clean()?;
        }
        match self.status() {
            Status::Valid => Ok(()),
            Status::NullHead => {
                info!("Linking {:?} -> {:?}", &self.head, &self.tail);
                if let Some(dir) = self.head.parent() {
                    fs::create_dir_all(dir)?;
                }
                Ok(symlink::symlink_file(&self.tail, &self.head)?)
            }
            Status::NullTail => Err(anyhow!("Link tail does not exist")),
            Status::InvalidHead(e) => Err(e.context("Invalid link head")),
            Status::InvalidTail(e) => Err(e.context("Invalid link tail")),
        }
    }

    // Try to remove link if it exists
    pub fn unlink(&self) -> Result<()> {
        match self.status() {
            Status::Valid => {
                info!("Unlinking {:?} -> {:?}", &self.head, &self.tail);
                Ok(symlink::remove_symlink_file(&self.head)?)
            }
            _ => Ok(()),
        }
    }

    // Remove any conflicting files/links at head
    pub fn clean(&self) -> Result<()> {
        match self.status() {
            Status::InvalidHead(_) | Status::InvalidTail(_) => {
                info!("Removing {:?}", &self.head);
                Ok(fs::remove_file(&self.head)?)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn fixture() -> (tempfile::TempDir, Link) {
        let dir = tempfile::tempdir().expect("Failed to create tempdir");
        let link = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        (dir, link)
    }

    #[test]
    fn status_no_tail() {
        let (_dir, link) = fixture();
        assert!(matches!(link.status(), Status::NullTail));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_no_head() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        assert!(matches!(link.status(), Status::NullHead));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_valid() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        symlink::symlink_file(&link.tail, &link.head).expect("Failed to create symlink");
        assert!(matches!(link.status(), Status::Valid));
        assert!(link.is_valid());
    }

    #[test]
    fn status_wrong_tail() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.tail).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.head).expect("Failed to create symlink");
        assert!(matches!(link.status(), Status::InvalidTail(_)));
        assert!(!link.is_valid());
    }

    #[test]
    fn status_head_is_file() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        File::create(&link.head).expect("Failed to create tempfile");
        assert!(matches!(link.status(), Status::InvalidHead(_)));
        assert!(!link.is_valid());
    }

    #[test]
    fn link_normal() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        // Link once
        link.link(false).expect("Failed to create link");
        assert_eq!(
            link.head.read_link().expect("Failed to read link"),
            link.tail
        );
    }

    #[test]
    fn link_create_parent_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create tempdir");
        let link = Link::new(
            dir.path().join("parent").join("link.head"),
            dir.path().join("link.tail"),
        );
        File::create(&link.tail).expect("Failed to create tempfile");
        // Link once
        link.link(false).expect("Failed to create link");
        assert_eq!(
            link.head.read_link().expect("Failed to read link"),
            link.tail
        );
    }

    #[test]
    fn unlink_normal() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        // Link and unlink once
        link.link(false).expect("Failed to create link");
        link.unlink().expect("Failed to remove link");
        assert!(!link.head.exists());
    }

    #[test]
    fn clean_invalid_head() {
        let (_dir, link) = fixture();
        File::create(&link.tail).expect("Failed to create tempfile");
        File::create(&link.head).expect("Failed to create tempfile");
        link.clean().expect("Failed to clean link");
        link.link(false).expect("Failed to apply link");
    }

    #[test]
    fn clean_invalid_tail() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.tail).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.head).expect("Failed to create symlink");
        link.clean().expect("Failed to clean link");
        link.link(false).expect("Failed to apply link");
    }

    #[test]
    fn clean_broken_link() {
        let (dir, link) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&link.tail).expect("Failed to create tempfile");
        File::create(&wrong).expect("Failed to create tempfile");
        symlink::symlink_file(&wrong, &link.head).expect("Failed to create symlink");
        fs::remove_file(&wrong).expect("Failed to delete tail");
        link.clean().expect("Failed to clean link");
        link.link(false).expect("Failed to apply link");
    }
}
