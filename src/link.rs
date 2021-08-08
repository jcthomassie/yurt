use anyhow::{anyhow, Result};
use log::info;
use serde::Deserialize;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

#[inline]
pub fn expand_path<S: ?Sized + AsRef<str>>(path: &S) -> Result<PathBuf> {
    Ok(PathBuf::from(shellexpand::full(path.as_ref())?.as_ref()))
}

pub enum Status {
    Valid,
    NullHead,
    NullTail,
    InvalidHead(Error),
    InvalidTail(Error),
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Link {
    // head@ -> tail
    head: PathBuf,
    tail: PathBuf,
}

impl Link {
    pub fn new<P: Into<PathBuf>>(head: P, tail: P) -> Self {
        Self {
            head: head.into(),
            tail: tail.into(),
        }
    }

    // Returns new link with paths expanded
    pub fn expand(&self) -> Result<Self> {
        Ok(Self::new(
            expand_path(self.head.to_str().unwrap())?,
            expand_path(self.tail.to_str().unwrap())?,
        ))
    }

    // Get current status of link
    pub fn status(&self) -> Status {
        if !self.tail.exists() {
            return Status::NullTail;
        }
        if !self.head.exists() {
            return Status::NullHead;
        }
        match self.head.read_link() {
            Ok(target) if target == self.tail => Status::Valid,
            Ok(target) => Status::InvalidTail(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "link source points to wrong target: {:?}@=>{:?} != {:?}",
                    self.head, target, self.tail
                ),
            )),
            Err(e) => Status::InvalidHead(e),
        }
    }

    // Try to create link if it does not already exist
    pub fn link(&self) -> Result<()> {
        match self.status() {
            Status::Valid => Ok(()),
            Status::NullHead => {
                info!("Linking {:?}@->{:?}", &self.head, &self.tail);
                Ok(symlink::symlink_file(&self.tail, &self.head)?)
            }
            Status::NullTail => Err(anyhow!("Link tail does not exist")),
            Status::InvalidHead(e) => Err(anyhow!(e).context("Invalid link head")),
            Status::InvalidTail(e) => Err(anyhow!(e).context("Invalid link tail")),
        }
    }

    // Try to remove link if it exists
    pub fn unlink(&self) -> Result<()> {
        match self.status() {
            Status::Valid => {
                info!("Unlinking {:?}@->{:?}", &self.head, &self.tail);
                Ok(symlink::remove_symlink_file(&self.head)?)
            }
            _ => Ok(()),
        }
    }

    // Remove any conflicting files/links at head
    pub fn clean(&self) -> Result<()> {
        match self.status() {
            Status::InvalidHead(_) => {
                info!("Removing {:?}", &self.head);
                Ok(fs::remove_file(&self.head)?)
            }
            Status::InvalidTail(_) => {
                info!("Removing {:?}", &self.head);
                Ok(symlink::remove_symlink_file(&self.head)?)
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
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let ln = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        (dir, ln)
    }

    #[test]
    fn status_no_tail() {
        let (_dir, ln) = fixture();
        assert!(matches!(ln.status(), Status::NullTail));
    }

    #[test]
    fn status_no_head() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        assert!(matches!(ln.status(), Status::NullHead));
    }

    #[test]
    fn status_valid() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        symlink::symlink_file(&ln.tail, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), Status::Valid));
    }

    #[test]
    fn status_wrong_tail() {
        let (dir, ln) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&ln.tail).expect("failed to create tempfile");
        File::create(&wrong).expect("failed to create tempfile");
        symlink::symlink_file(&wrong, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), Status::InvalidTail(_)));
    }

    #[test]
    fn status_head_is_file() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        File::create(&ln.head).expect("failed to create tempfile");
        assert!(matches!(ln.status(), Status::InvalidHead(_)));
    }

    #[test]
    fn link_normal() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        // Link once
        ln.link().expect("failed to create link");
        assert_eq!(ln.head.read_link().expect("failed to read link"), ln.tail);
    }

    #[test]
    fn unlink_normal() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        // Link and unlink once
        ln.link().expect("failed to create link");
        ln.unlink().expect("failed to remove link");
        assert!(!ln.head.exists());
    }

    #[test]
    fn clean_invalid_head() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        File::create(&ln.head).expect("failed to create tempfile");
        ln.clean().expect("failed to clean link");
        ln.link().expect("failed to apply link");
    }

    #[test]
    fn clean_invalid_tail() {
        let (dir, ln) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&ln.tail).expect("failed to create tempfile");
        File::create(&wrong).expect("failed to create tempfile");
        symlink::symlink_file(&wrong, &ln.head).expect("failed to create symlink");
        ln.clean().expect("failed to clean link");
        ln.link().expect("failed to apply link");
    }
}
