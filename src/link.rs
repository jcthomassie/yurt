use super::error::YurtResult;
use log::info;
use serde::Deserialize;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

#[inline]
pub fn expand_path<S: ?Sized + AsRef<str>>(path: &S) -> YurtResult<PathBuf> {
    Ok(PathBuf::from(shellexpand::full(path.as_ref())?.as_ref()))
}

pub enum Status {
    Exists,
    NotExists,
    Invalid(Error),
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
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
    pub fn expand(&self) -> YurtResult<Self> {
        Ok(Self::new(
            expand_path(self.head.to_str().ok_or("")?)?,
            expand_path(self.tail.to_str().ok_or("")?)?,
        ))
    }

    // Get current status of link
    pub fn status(&self) -> Status {
        if !self.tail.exists() {
            return Status::Invalid(Error::new(
                ErrorKind::NotFound,
                "link target does not exist",
            ));
        }
        if !self.head.exists() {
            return Status::NotExists;
        }
        match self.head.read_link() {
            Ok(target) if target == self.tail => Status::Exists,
            Ok(target) => Status::Invalid(Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "link source points to wrong target: {:?}@=>{:?} != {:?}",
                    self.head, target, self.tail
                ),
            )),
            Err(e) => Status::Invalid(e),
        }
    }

    // Try to create link if it does not already exist
    pub fn link(&self) -> YurtResult<()> {
        match self.status() {
            Status::Exists => Ok(()),
            Status::NotExists => {
                info!("Linking {:?}@->{:?}", &self.head, &self.tail);
                Ok(symlink::symlink_file(&self.tail, &self.head)?)
            }
            Status::Invalid(e) => Err(e.into()),
        }
    }

    // Try to remove link if it exists
    pub fn unlink(&self) -> YurtResult<()> {
        match self.status() {
            Status::Exists => {
                info!("Unlinking {:?}@->{:?}", &self.head, &self.tail);
                Ok(symlink::remove_symlink_file(&self.head)?)
            }
            _ => Ok(()),
        }
    }

    // Remove any conflicting files/links at head
    pub fn clean(&self) -> YurtResult<()> {
        match self.status() {
            Status::Invalid(_) => {
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
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let ln = Link::new(dir.path().join("link.head"), dir.path().join("link.tail"));
        (dir, ln)
    }

    #[test]
    fn status_null() {
        let (_dir, ln) = fixture();
        assert!(matches!(ln.status(), Status::Invalid(_)));
    }

    #[test]
    fn status_no_head() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        assert!(matches!(ln.status(), Status::NotExists));
    }

    #[test]
    fn status_exists() {
        let (_dir, ln) = fixture();
        File::create(&ln.tail).expect("failed to create tempfile");
        symlink::symlink_file(&ln.tail, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), Status::Exists));
        symlink::remove_symlink_file(&ln.head).expect("failed to remove symlink");
    }

    #[test]
    fn status_wrong_tail() {
        let (dir, ln) = fixture();
        let wrong = dir.path().join("wrong.thing");
        File::create(&wrong).expect("failed to create tempfile");
        symlink::symlink_file(&wrong, &ln.head).expect("failed to create symlink");
        assert!(matches!(ln.status(), Status::Invalid(_)));
        symlink::remove_symlink_file(&ln.head).expect("failed to remove symlink");
    }

    #[test]
    fn status_head_is_file() {
        let (_dir, ln) = fixture();
        File::create(&ln.head).expect("failed to create tempfile");
        assert!(matches!(ln.status(), Status::Invalid(_)));
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
}
